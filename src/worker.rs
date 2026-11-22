//! Thread di lavoro: connessione, lettura, polling e scrittura dei file
//! avvengono qui, mai nel thread dell'interfaccia.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::diagnosis::{self, Contesto, DiagnosisReport};
use crate::elm::{ConfermaCancellazione, Elm, ElmError, Esito, EsitoCancellazione};
use crate::mcp::{McpConfig, Ospite, StatoMcpInfo};
use crate::model::{Marker, Origine, Sample, Stato, adesso_iso};
use crate::mqtt::{MqttConfig, MqttPublisher, StatoMqtt, StatoMqttInfo};
use crate::obd::ObdRequest;
use crate::obd::frames::BusKind;
use crate::obd::pids::{self, Decodifica};
use crate::recording::{self, Intestazione, Recorder, SessionData};
use crate::riproduzione::{self, Riproduttore};
pub use crate::riproduzione::{ComandoRiproduzione, StatoRiproduzione};
use crate::transport::Transport;
use crate::transport::ble::{BleDispositivo, BleErrore, BleManager, GattInfo};
use crate::transport::demo::DemoTransport;
use crate::transport::serial::{self, PortaSeriale, SerialTransport};

/// Da dove arrivano i dati.
pub enum Sorgente {
    /// Dispositivo BLE scelto dall'elenco di scansione (indirizzo).
    Ble(String),
    Seriale {
        porta: String,
        baud: u32,
    },
    Demo,
    /// Trasporto già pronto (verifiche automatiche).
    Trasporto(Box<dyn Transport>),
    /// Sessione registrata da «rivivere»: i suoi campioni scorrono nel tempo
    /// e arrivano come quelli dal vivo (vedi [`crate::riproduzione`]).
    Riproduzione(Box<SessionData>),
}

pub enum Comando {
    ScansionaBle,
    FermaScansione,
    ElencaSeriali,
    Collega(Sorgente),
    Scollega,
    LeggiDiagnosi,
    /// Cancella i codici guasto (servizio 04) dopo aver salvato una copia
    /// della diagnosi. Richiede la conferma dell'utente.
    CancellaDtc(ConfermaCancellazione),
    /// Avvia o aggiorna il polling; `prioritarie` sono le chiavi delle sonde.
    /// Durante la riproduzione è accettato e ignorato (come `FermaPolling`):
    /// scorrono sempre tutti i campioni della sessione.
    AvviaPolling {
        chiavi: Vec<String>,
        prioritarie: Vec<String>,
    },
    FermaPolling,
    AvviaRegistrazione,
    FermaRegistrazione,
    Marcatore(String),
    /// Avvia, riavvia o ferma la pubblicazione MQTT secondo la configurazione.
    ConfiguraMqtt(MqttConfig),
    /// Avvia, riavvia o ferma il server MCP per assistenti AI.
    ConfiguraMcp(McpConfig),
    /// Barra di trasporto della riproduzione (ignorato senza riproduzione).
    Riproduzione(ComandoRiproduzione),
    Esci,
}

/// Stato del collegamento, sempre esplicito.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatoConn {
    Scollegato,
    Scansione,
    /// In connessione; il testo descrive la fase in corso.
    Connessione(String),
    Connesso,
    Diagnosi,
    /// Cancellazione dei codici guasto in corso (copia, invio, rilettura).
    Cancellazione,
    Disconnessione,
}

/// Errori distinti da mostrare all'utente.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErroreUtente {
    BluetoothSpento,
    NessunBluetooth,
    AdattatoreNonRaggiungibile(String),
    ProfiloBleSconosciuto,
    AdattatoreNonRisponde(String),
    CentralinaNonRisponde(String),
    CollegamentoPerso(String),
    Registrazione(String),
    /// La cancellazione dei codici guasto non è stata nemmeno tentata.
    CancellazioneNonEseguita(String),
    /// Operazione rifiutata perché è in corso la riproduzione di una sessione.
    Riproduzione(String),
    Altro(String),
}

impl ErroreUtente {
    pub fn titolo(&self) -> &'static str {
        match self {
            ErroreUtente::BluetoothSpento => "Bluetooth disattivato",
            ErroreUtente::NessunBluetooth => "Nessun adattatore Bluetooth nel PC",
            ErroreUtente::AdattatoreNonRaggiungibile(_) => "Adattatore non raggiungibile",
            ErroreUtente::ProfiloBleSconosciuto => "Profilo BLE sconosciuto",
            ErroreUtente::AdattatoreNonRisponde(_) => "L'adattatore non risponde ai comandi",
            ErroreUtente::CentralinaNonRisponde(_) => "La centralina non risponde",
            ErroreUtente::CollegamentoPerso(_) => "Collegamento interrotto",
            ErroreUtente::Registrazione(_) => "Errore di registrazione",
            ErroreUtente::CancellazioneNonEseguita(_) => "Codici guasto non cancellati",
            ErroreUtente::Riproduzione(_) => "Non disponibile durante la riproduzione",
            ErroreUtente::Altro(_) => "Errore",
        }
    }

    pub fn messaggio(&self) -> String {
        match self {
            ErroreUtente::BluetoothSpento => {
                "Attiva il Bluetooth in Windows (Impostazioni → Bluetooth e dispositivi) e riprova."
                    .into()
            }
            ErroreUtente::NessunBluetooth => {
                "Windows non segnala alcuna radio Bluetooth. Puoi usare la porta seriale o la demo."
                    .into()
            }
            ErroreUtente::AdattatoreNonRaggiungibile(d) => format!(
                "Il collegamento radio con l'adattatore non si è stabilito. Controlla che sia \
                 inserito nella presa OBD, alimentato e non collegato a un altro dispositivo \
                 (telefono). Dettaglio: {d}"
            ),
            ErroreUtente::ProfiloBleSconosciuto => {
                "Il dispositivo è collegato ma i suoi servizi GATT non corrispondono a un profilo \
                 seriale documentato: l'app non scrive su caratteristiche scelte per tentativi. \
                 Qui sotto trovi nome e UUID da copiare per la diagnosi."
                    .into()
            }
            ErroreUtente::AdattatoreNonRisponde(d) => format!(
                "Il collegamento è aperto ma il dispositivo non risponde come un ELM327. \
                 Dettaglio: {d}"
            ),
            ErroreUtente::CentralinaNonRisponde(d) => format!(
                "L'adattatore funziona ma nessuna centralina ha risposto. Porta il quadro su ON \
                 (o avvia il motore) e riprova. Dettaglio: {d}"
            ),
            ErroreUtente::CollegamentoPerso(d) => format!(
                "Il polling è stato fermato. I dati già registrati sono stati conservati. \
                 Dettaglio: {d}"
            ),
            ErroreUtente::CancellazioneNonEseguita(d) => {
                format!("Nessuna richiesta di cancellazione è stata inviata alla centralina. {d}")
            }
            ErroreUtente::Registrazione(d)
            | ErroreUtente::Riproduzione(d)
            | ErroreUtente::Altro(d) => d.clone(),
        }
    }
}

/// Informazioni sul collegamento stabilito.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InfoConn {
    /// Vero se i dati sono sintetici: simulatore dal vivo, oppure
    /// riproduzione di una sessione registrata in demo.
    pub demo: bool,
    /// Da dove arrivano i dati: veicolo, simulatore o riproduzione.
    pub origine: Origine,
    /// Presente durante la riproduzione di una sessione.
    pub riproduzione: Option<InfoRiproduzione>,
    /// Parametri liberi (fuori catalogo) offerti dalla sorgente: solo in
    /// riproduzione di una sessione importata.
    pub liberi: BTreeSet<String>,
    pub collegamento: String,
    pub adattatore: String,
    pub protocollo: String,
    pub protocollo_numero: u8,
    pub ecu: Vec<String>,
    /// PID del servizio 01 dichiarati da ogni centralina.
    pub supportati: BTreeMap<String, BTreeSet<u8>>,
    /// Unione dei PID di tutte le centraline.
    pub unione: BTreeSet<u8>,
    pub gatt: Option<GattInfo>,
    /// Avvisi sull'interpretazione dei dati (per esempio scale non standard).
    pub avvisi: Vec<String>,
}

/// Sessione in riproduzione.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InfoRiproduzione {
    /// Nome del file della sessione.
    pub file: String,
    /// Quando la sessione è stata registrata (ISO 8601, dall'intestazione).
    pub registrata_il: String,
    /// La sessione era stata registrata in modalità demo.
    pub sessione_demo: bool,
    /// Per una sessione importata: «<formato> da «<file>»».
    pub importata: Option<String>,
}

impl InfoConn {
    pub fn in_riproduzione(&self) -> bool {
        self.origine == Origine::Riproduzione
    }
}

pub enum Evento {
    Stato(StatoConn),
    DispositiviBle(Vec<BleDispositivo>),
    PorteSeriali(Vec<PortaSeriale>),
    /// Servizi GATT di un dispositivo con profilo sconosciuto.
    GattSconosciuto(GattInfo),
    Connesso(Box<InfoConn>),
    Diagnosi(Box<DiagnosisReport>),
    /// Esito della cancellazione dei codici guasto. La diagnosi riletta
    /// dopo la cancellazione arriva subito dopo come [`Evento::Diagnosi`].
    CancellazioneDtc(Box<RisultatoCancellazione>),
    Campioni(Vec<Sample>),
    Polling(bool),
    RegistrazioneAvviata {
        percorso: PathBuf,
        t_inizio: f64,
    },
    RegistrazioneFermata {
        percorso: PathBuf,
        csv: Option<PathBuf>,
        campioni: usize,
    },
    Marcatore(Marker),
    /// `motivo` è `None` se la disconnessione è stata chiesta dall'utente.
    Scollegato {
        motivo: Option<String>,
    },
    Errore(ErroreUtente),
    Registro(Vec<String>),
    /// Stato della pubblicazione MQTT (a ogni cambiamento, al più due volte al secondo).
    Mqtt(StatoMqttInfo),
    /// Stato del server MCP (a ogni cambiamento, al più due volte al secondo).
    Mcp(StatoMcpInfo),
    /// Stato della riproduzione: a ogni cambiamento e alcune volte al secondo
    /// mentre scorre.
    Riproduzione(StatoRiproduzione),
    /// La posizione della riproduzione è saltata (comando «vai a» o ripetizione
    /// dall'inizio): l'interfaccia svuota i grafici dal vivo, così nessuna
    /// linea attraversa il salto. I campioni successivi partono da `posizione`.
    Salto {
        posizione: f64,
    },
}

/// Esito della cancellazione dei codici guasto visto dall'interfaccia.
#[derive(Clone, Debug, PartialEq)]
pub struct RisultatoCancellazione {
    pub esito: EsitoCancellazione,
    /// Vero se la cancellazione è avvenuta sul simulatore.
    pub demo: bool,
    /// Copia della diagnosi salvata prima della cancellazione.
    pub copia: PathBuf,
    pub eseguita_il: String,
}

/// Etichette dei marcatori scritti nella registrazione.
pub const MARCATORE_CANCELLAZIONE: &str = "Codici guasto cancellati";
pub const MARCATORE_CANCELLAZIONE_FALLITA: &str = "Cancellazione dei codici guasto non riuscita";

/// Ordine delle richieste: tutte le prioritarie, poi una delle altre a rotazione.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Piano {
    veloci: Vec<u8>,
    lenti: Vec<u8>,
    passo: usize,
    i_lento: usize,
}

impl Piano {
    pub fn new(chiavi: &[String], prioritarie: &[String], supportati: &BTreeSet<u8>) -> Self {
        // I parametri calcolati dall'app non hanno un PID da richiedere.
        let pid_di = |k: &String| {
            pids::per_key(k)
                .filter(|p| !p.derivato && !p.libero)
                .map(|p| p.pid)
        };
        let mut veloci: Vec<u8> = Vec::new();
        for pid in prioritarie.iter().filter_map(pid_di) {
            if supportati.contains(&pid) && !veloci.contains(&pid) {
                veloci.push(pid);
            }
        }
        let mut lenti: Vec<u8> = Vec::new();
        for pid in chiavi.iter().filter_map(pid_di) {
            if supportati.contains(&pid) && !veloci.contains(&pid) && !lenti.contains(&pid) {
                lenti.push(pid);
            }
        }
        Piano {
            veloci,
            lenti,
            passo: 0,
            i_lento: 0,
        }
    }

    pub fn vuoto(&self) -> bool {
        self.veloci.is_empty() && self.lenti.is_empty()
    }

    pub fn prossimo(&mut self) -> Option<u8> {
        if self.passo < self.veloci.len() {
            self.passo += 1;
            return Some(self.veloci[self.passo - 1]);
        }
        self.passo = 0;
        if self.lenti.is_empty() {
            if self.veloci.is_empty() {
                return None;
            }
            self.passo = 1;
            return Some(self.veloci[0]);
        }
        let pid = self.lenti[self.i_lento % self.lenti.len()];
        self.i_lento = (self.i_lento + 1) % self.lenti.len();
        Some(pid)
    }
}

/// Oltre questo intervallo fra due velocità valide non si integra: la distanza
/// percorsa nel buco è sconosciuta e non viene stimata.
const BUCO_MAX_DISTANZA_S: f64 = 5.0;

/// Distanza percorsa dalla connessione, integrando la velocità nel tempo
/// (regola del trapezio). Un dato mancante non aggiunge nulla.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntegratoreDistanza {
    /// Tempo (s) e velocità (km/h) dell'ultima velocità valida.
    ultima: Option<(f64, f64)>,
    km: f64,
}

impl IntegratoreDistanza {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra la velocità letta al tempo `t`; `None` se la lettura è fallita.
    /// Restituisce la distanza totale (km) solo se il campione è valido.
    pub fn campione(&mut self, t: f64, kmh: Option<f64>) -> Option<f64> {
        let v = kmh?;
        if let Some((t0, v0)) = self.ultima {
            let dt = t - t0;
            // Intervalli nulli, negativi o troppo lunghi non si integrano.
            if dt > 0.0 && dt <= BUCO_MAX_DISTANZA_S {
                self.km += (v0 + v) / 2.0 * dt / 3600.0;
            }
        }
        self.ultima = Some((t, v));
        Some(self.km)
    }
}

struct Link {
    elm: Elm,
    info: InfoConn,
    kind: BusKind,
    t0: Instant,
    piano: Option<Piano>,
    recorder: Option<Recorder>,
    diagnosi: Option<DiagnosisReport>,
    errori_di_fila: u32,
    /// Riparte da zero a ogni connessione.
    distanza: IntegratoreDistanza,
}

/// Esegue la lettura diagnostica completa sul collegamento.
fn leggi_su(l: &mut Link) -> Result<DiagnosisReport, crate::transport::Scollegato> {
    let ctx = Contesto {
        adattatore: &l.info.adattatore,
        protocollo: &l.info.protocollo,
        supportati: &l.info.supportati,
    };
    diagnosis::leggi(&mut l.elm, &ctx)
}

/// Come si è conclusa la sequenza di cancellazione.
enum FineCancellazione {
    Completata {
        errore_file: Option<String>,
    },
    /// Nulla è stato inviato alla centralina.
    NonEseguita(String),
    /// Collegamento caduto.
    Caduta(String),
}

/// Errori consecutivi di comunicazione oltre i quali il collegamento è dato per perso.
const MAX_ERRORI_DI_FILA: u32 = 5;
const DURATA_SCANSIONE: Duration = Duration::from_secs(40);
/// Passo del ciclo durante la riproduzione.
const PASSO_RIPRODUZIONE: Duration = Duration::from_millis(20);
/// Intervallo fra due stati della riproduzione mentre scorre.
const INTERVALLO_STATO_RIPRODUZIONE: Duration = Duration::from_millis(250);
/// Attesa a riposo con il server MCP acceso: entro questo tempo parte
/// un'azione chiesta da un assistente.
const ATTESA_CON_MCP: Duration = Duration::from_millis(40);

/// Riproduzione in corso (al posto di un collegamento).
struct Riproduzione {
    motore: Riproduttore,
    ultimo_passo: Instant,
    inviato: Option<(StatoRiproduzione, Instant)>,
}

struct Worker {
    rx: Receiver<Comando>,
    tx: Sender<Evento>,
    notifica: Box<dyn Fn() + Send>,
    cartella: PathBuf,
    rt: Option<Arc<tokio::runtime::Runtime>>,
    ble: Option<BleManager>,
    scansione: Option<(Instant, Instant)>,
    link: Option<Link>,
    riproduzione: Option<Riproduzione>,
    mqtt: Option<MqttPublisher>,
    /// Ultimo stato MQTT comunicato all'interfaccia e quando.
    mqtt_inviato: Option<(StatoMqttInfo, Instant)>,
    /// Archivio e server MCP (vedi [`crate::mcp`]).
    mcp: Ospite,
}

/// Avvia il thread di lavoro. `notifica` viene chiamata dopo ogni evento
/// (serve a risvegliare l'interfaccia).
pub fn avvia(
    cartella: PathBuf,
    notifica: impl Fn() + Send + 'static,
) -> (Sender<Comando>, Receiver<Evento>, JoinHandle<()>) {
    let (tx_cmd, rx_cmd) = mpsc::channel();
    let (tx_ev, rx_ev) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("obdio-lavoro".into())
        .spawn(move || {
            let mcp = Ospite::new(cartella.clone());
            Worker {
                rx: rx_cmd,
                tx: tx_ev,
                notifica: Box::new(notifica),
                cartella,
                rt: None,
                ble: None,
                scansione: None,
                link: None,
                riproduzione: None,
                mqtt: None,
                mqtt_inviato: None,
                mcp,
            }
            .esegui();
        })
        .expect("impossibile avviare il thread di lavoro");
    (tx_cmd, rx_ev, handle)
}

impl Worker {
    fn invia(&self, e: Evento) {
        // L'archivio del server MCP vede ogni evento prima dell'interfaccia.
        self.mcp.osserva(&e);
        let _ = self.tx.send(e);
        (self.notifica)();
    }

    fn stato(&self, s: StatoConn) {
        self.invia(Evento::Stato(s));
    }

    fn esegui(mut self) {
        loop {
            let occupato =
                self.scansione.is_some() || self.link.as_ref().is_some_and(|l| l.piano.is_some());
            let attesa = if self
                .riproduzione
                .as_ref()
                .is_some_and(|r| r.motore.in_corso())
            {
                PASSO_RIPRODUZIONE
            } else if self.mcp.attivo() {
                // Le azioni chieste da un assistente arrivano su una coda a parte.
                ATTESA_CON_MCP
            } else {
                Duration::from_millis(250)
            };
            let cmd = if occupato {
                match self.rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => Some(Comando::Esci),
                }
            } else {
                match self.rx.recv_timeout(attesa) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => Some(Comando::Esci),
                }
            };
            if let Some(cmd) = cmd
                && !self.gestisci(cmd)
            {
                break;
            }
            // Azioni chieste via MCP: mai `Esci` né la cancellazione dei codici.
            while let Some(azione) = self.mcp.prossima_azione() {
                self.gestisci(azione);
            }
            self.passo_scansione();
            self.passo_polling();
            self.passo_riproduzione();
            self.passo_mqtt();
            self.passo_mcp();
        }
        self.ferma_scansione();
        self.chiudi_link(None, false);
        self.chiudi_riproduzione(false);
        if let Some(m) = self.mqtt.take() {
            m.ferma();
        }
        self.mcp.ferma();
    }

    // ---- MCP -----------------------------------------------------------

    /// Avvia, riavvia o ferma il server MCP e ne comunica lo stato.
    fn configura_mcp(&mut self, config: McpConfig) {
        let info = self.mcp.configura(&config);
        self.invia(Evento::Mcp(info));
    }

    /// Comunica all'interfaccia i cambiamenti di stato del server MCP.
    fn passo_mcp(&mut self) {
        if let Some(info) = self.mcp.stato_cambiato() {
            self.invia(Evento::Mcp(info));
        }
    }

    // ---- MQTT ----------------------------------------------------------

    /// Avvia, riavvia o ferma il pubblicatore. Non blocca oltre la chiusura
    /// pulita del precedente (al più un paio di secondi).
    fn configura_mqtt(&mut self, config: MqttConfig) {
        if let Some(m) = self.mqtt.take() {
            m.ferma();
        }
        self.mqtt_inviato = None;
        if !config.attivo {
            self.invia(Evento::Mqtt(StatoMqttInfo::disattivato()));
            return;
        }
        match MqttPublisher::avvia(&config) {
            Ok(m) => {
                if let Some(l) = &self.link {
                    m.collegato_origine(l.info.origine);
                } else if self.riproduzione.is_some() {
                    m.collegato_origine(Origine::Riproduzione);
                }
                self.mqtt = Some(m);
                self.passo_mqtt();
            }
            Err(e) => self.invia(Evento::Mqtt(StatoMqttInfo {
                stato: StatoMqtt::Errore(e.clone()),
                pubblicati: 0,
                scartati: 0,
                ultimo_errore: Some(e),
            })),
        }
    }

    /// Comunica all'interfaccia i cambiamenti di stato MQTT.
    fn passo_mqtt(&mut self) {
        let Some(m) = &self.mqtt else {
            return;
        };
        let info = m.stato();
        let da_inviare = match &self.mqtt_inviato {
            None => true,
            Some((u, t)) => *u != info && t.elapsed() >= Duration::from_millis(500),
        };
        if da_inviare {
            self.mqtt_inviato = Some((info.clone(), Instant::now()));
            self.invia(Evento::Mqtt(info));
        }
    }

    fn mqtt_diagnosi(&self, report: &DiagnosisReport) {
        if let Some(m) = &self.mqtt {
            m.diagnosi(report);
        }
    }

    fn mqtt_cancellazione(&self, r: &RisultatoCancellazione) {
        if let Some(m) = &self.mqtt {
            m.cancellazione(&r.esito, &r.eseguita_il);
        }
    }

    /// Restituisce `false` quando il thread deve terminare.
    fn gestisci(&mut self, cmd: Comando) -> bool {
        match cmd {
            Comando::Esci => return false,
            Comando::ScansionaBle => self.avvia_scansione(),
            Comando::FermaScansione => {
                self.ferma_scansione();
                if self.link.is_none() {
                    self.stato(StatoConn::Scollegato);
                }
            }
            Comando::ElencaSeriali if !crate::piattaforma::SERIALE_DISPONIBILE => {}
            Comando::ElencaSeriali => match serial::elenca_porte() {
                Ok(p) => self.invia(Evento::PorteSeriali(p)),
                Err(e) => self.invia(Evento::Errore(ErroreUtente::Altro(format!(
                    "Impossibile elencare le porte seriali: {e}"
                )))),
            },
            Comando::Collega(sorgente) => self.collega(sorgente),
            Comando::Scollega => {
                if self.link.is_some() {
                    self.stato(StatoConn::Disconnessione);
                    self.chiudi_link(None, true);
                }
                if self.riproduzione.is_some() {
                    self.stato(StatoConn::Disconnessione);
                    self.chiudi_riproduzione(true);
                }
            }
            Comando::LeggiDiagnosi if self.riproduzione.is_some() => self.rifiuta(
                "Durante la riproduzione non c'è una centralina da leggere: la diagnosi \
                 mostrata è quella registrata nella sessione. Chiudi la riproduzione e collega \
                 l'adattatore per una lettura nuova.",
            ),
            Comando::LeggiDiagnosi => self.leggi_diagnosi(),
            Comando::CancellaDtc(_) if self.riproduzione.is_some() => self.rifiuta(
                "Durante la riproduzione i codici guasto non si possono cancellare: i dati sono \
                 registrati e nessuna centralina è collegata. Nulla è stato inviato.",
            ),
            Comando::CancellaDtc(conferma) => self.cancella_dtc(conferma),
            Comando::AvviaPolling {
                chiavi,
                prioritarie,
            } => {
                if let Some(l) = self.link.as_mut() {
                    let piano = Piano::new(&chiavi, &prioritarie, &l.info.unione);
                    let attivo = !piano.vuoto();
                    l.piano = attivo.then_some(piano);
                    l.elm.traccia = !attivo;
                    self.mcp.archivio().chiavi_lettura(&chiavi, &prioritarie);
                    self.invia(Evento::Polling(attivo));
                }
            }
            Comando::FermaPolling => {
                if let Some(l) = self.link.as_mut() {
                    l.piano = None;
                    l.elm.traccia = true;
                    self.invia(Evento::Polling(false));
                }
            }
            Comando::AvviaRegistrazione if self.riproduzione.is_some() => self.rifiuta(
                "Durante la riproduzione non si registra: la sessione è già registrata. Per \
                 salvarne una copia in un altro formato usa «Esporta…» nella scheda Sessioni.",
            ),
            Comando::AvviaRegistrazione => self.avvia_registrazione(),
            Comando::FermaRegistrazione => self.ferma_registrazione(),
            Comando::ConfiguraMqtt(config) => self.configura_mqtt(config),
            Comando::ConfiguraMcp(config) => self.configura_mcp(config),
            Comando::Riproduzione(c) => self.comando_riproduzione(c),
            Comando::Marcatore(_) if self.riproduzione.is_some() => self.rifiuta(
                "Durante la riproduzione non si aggiungono marcatori: si vedono quelli già \
                 registrati nella sessione.",
            ),
            Comando::Marcatore(etichetta) => {
                let Some(l) = self.link.as_mut() else {
                    return true;
                };
                let m = Marker {
                    t: l.t0.elapsed().as_secs_f64(),
                    ts: adesso_iso(),
                    etichetta,
                };
                let mut errore = None;
                if let Some(r) = l.recorder.as_mut()
                    && let Err(e) = r.marcatore(&m)
                {
                    errore = Some(e.to_string());
                }
                if let Some(mq) = &self.mqtt {
                    mq.marcatore(&m);
                }
                self.invia(Evento::Marcatore(m));
                if let Some(e) = errore {
                    self.errore_registrazione(e);
                }
            }
        }
        true
    }

    // ---- Bluetooth LE -------------------------------------------------

    fn ble(&mut self) -> Result<&BleManager, BleErrore> {
        if self.ble.is_none() {
            let rt = match &self.rt {
                Some(rt) => rt.clone(),
                None => {
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .enable_all()
                        .build()
                        .map_err(|e| BleErrore::Altro(e.to_string()))?;
                    let rt = Arc::new(rt);
                    self.rt = Some(rt.clone());
                    rt
                }
            };
            self.ble = Some(BleManager::new(rt)?);
        }
        Ok(self.ble.as_ref().expect("appena creato"))
    }

    fn errore_ble(&self, e: BleErrore) {
        let errore = match e {
            BleErrore::NessunAdattatore => ErroreUtente::NessunBluetooth,
            BleErrore::BluetoothSpento => ErroreUtente::BluetoothSpento,
            BleErrore::NonRaggiungibile(d) => ErroreUtente::AdattatoreNonRaggiungibile(d),
            BleErrore::ProfiloSconosciuto(info) => {
                self.invia(Evento::GattSconosciuto(info));
                ErroreUtente::ProfiloBleSconosciuto
            }
            BleErrore::Altro(d) => ErroreUtente::Altro(format!("Bluetooth: {d}")),
        };
        self.invia(Evento::Errore(errore));
    }

    fn avvia_scansione(&mut self) {
        if self.link.is_some() || self.riproduzione.is_some() {
            return;
        }
        let esito = self.ble().and_then(|b| b.avvia_scansione());
        match esito {
            Ok(()) => {
                let ora = Instant::now();
                self.scansione = Some((ora, ora));
                self.stato(StatoConn::Scansione);
            }
            Err(e) => {
                self.errore_ble(e);
                self.stato(StatoConn::Scollegato);
            }
        }
    }

    fn ferma_scansione(&mut self) {
        if self.scansione.take().is_some()
            && let Some(b) = &self.ble
        {
            b.ferma_scansione();
        }
    }

    fn passo_scansione(&mut self) {
        let Some((inizio, ultimo)) = self.scansione else {
            return;
        };
        if inizio.elapsed() > DURATA_SCANSIONE {
            self.ferma_scansione();
            self.stato(StatoConn::Scollegato);
            return;
        }
        if ultimo.elapsed() < Duration::from_millis(700) {
            std::thread::sleep(Duration::from_millis(50));
            return;
        }
        if let Some(b) = &self.ble {
            if let Err(e) = b.verifica_radio() {
                self.ferma_scansione();
                self.errore_ble(e);
                self.stato(StatoConn::Scollegato);
                return;
            }
            let dispositivi = b.dispositivi();
            self.invia(Evento::DispositiviBle(dispositivi));
        }
        self.scansione = Some((inizio, Instant::now()));
    }

    // ---- Connessione ---------------------------------------------------

    fn collega(&mut self, sorgente: Sorgente) {
        if self.link.is_some() || self.riproduzione.is_some() {
            return;
        }
        self.ferma_scansione();
        let sorgente = match sorgente {
            Sorgente::Riproduzione(s) => return self.collega_riproduzione(*s),
            altra => altra,
        };
        self.stato(StatoConn::Connessione(
            "Collegamento all'adattatore…".into(),
        ));

        // 1. Collegamento fisico.
        let mut gatt = None;
        let trasporto: Box<dyn Transport> = match sorgente {
            Sorgente::Demo => Box::new(DemoTransport::new()),
            Sorgente::Trasporto(t) => t,
            Sorgente::Riproduzione(_) => unreachable!("gestita sopra"),
            Sorgente::Seriale { porta, baud } => match SerialTransport::apri(&porta, baud) {
                Ok(t) => Box::new(t),
                Err(e) => {
                    self.invia(Evento::Errore(ErroreUtente::AdattatoreNonRaggiungibile(
                        format!("apertura di {porta} fallita: {e}"),
                    )));
                    self.stato(StatoConn::Scollegato);
                    return;
                }
            },
            Sorgente::Ble(indirizzo) => {
                let esito = self.ble().and_then(|b| b.collega(&indirizzo));
                match esito {
                    Ok((t, info)) => {
                        gatt = Some(info);
                        Box::new(t)
                    }
                    Err(e) => {
                        self.errore_ble(e);
                        self.stato(StatoConn::Scollegato);
                        return;
                    }
                }
            }
        };
        let mut elm = Elm::new(trasporto);
        let fallimento = |w: &Worker, elm: &mut Elm, errore: ErroreUtente| {
            w.invia(Evento::Registro(elm.prendi_registro()));
            elm.chiudi();
            w.invia(Evento::Errore(errore));
            w.stato(StatoConn::Scollegato);
        };

        // 2. L'adattatore risponde come un ELM327?
        self.stato(StatoConn::Connessione("Verifica dell'adattatore…".into()));
        let adattatore = match elm.inizializza() {
            Ok(id) => id,
            Err(e) => {
                let errore = match e {
                    ElmError::Scollegato(d) => ErroreUtente::AdattatoreNonRaggiungibile(d),
                    altro => ErroreUtente::AdattatoreNonRisponde(altro.to_string()),
                };
                fallimento(self, &mut elm, errore);
                return;
            }
        };
        self.invia(Evento::Registro(elm.prendi_registro()));

        // 3. La centralina risponde?
        self.stato(StatoConn::Connessione(
            "Ricerca del protocollo e della centralina…".into(),
        ));
        let (protocollo, iniziali) = match elm.collega_centralina() {
            Ok(x) => x,
            Err(e) => {
                let errore = match e {
                    ElmError::Scollegato(d) => ErroreUtente::CollegamentoPerso(d),
                    altro => ErroreUtente::CentralinaNonRisponde(altro.to_string()),
                };
                fallimento(self, &mut elm, errore);
                return;
            }
        };

        // 4. Parametri supportati.
        self.stato(StatoConn::Connessione(
            "Lettura dei parametri supportati…".into(),
        ));
        let supportati = match elm.pid_supportati(&iniziali) {
            Ok(s) => s,
            Err(s) => {
                fallimento(self, &mut elm, ErroreUtente::CollegamentoPerso(s.0));
                return;
            }
        };
        let unione: BTreeSet<u8> = supportati.values().flatten().copied().collect();
        let mut avvisi = Vec::new();
        if unione.contains(&0x4F)
            && let Ok(Esito::Dati(msg)) = elm.interroga(ObdRequest::CurrentData(0x4F))
            && let Some(m) = msg.first()
            && m.data.iter().skip(2).take(3).any(|b| *b != 0)
        {
            avvisi.push(format!(
                "La centralina dichiara fondi scala propri per le sonde a banda larga (PID 4F: \
                 {}). Lambda, tensione e corrente delle sonde sono mostrati con la scala \
                 standard e potrebbero non corrispondere.",
                m.data[2..]
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        self.invia(Evento::Registro(elm.prendi_registro()));

        let demo = elm.is_demo();
        let info = InfoConn {
            demo,
            origine: if demo {
                Origine::Demo
            } else {
                Origine::Veicolo
            },
            riproduzione: None,
            liberi: BTreeSet::new(),
            collegamento: elm.descrizione(),
            adattatore,
            protocollo: format!(
                "{} (protocollo ELM {}{})",
                protocollo.descrizione,
                protocollo.numero,
                if protocollo.automatico {
                    ", rilevato automaticamente"
                } else {
                    ""
                }
            ),
            protocollo_numero: protocollo.numero,
            ecu: supportati.keys().cloned().collect(),
            supportati,
            unione,
            gatt,
            avvisi,
        };
        self.invia(Evento::Connesso(Box::new(info.clone())));
        if let Some(m) = &self.mqtt {
            m.collegato_origine(info.origine);
        }
        self.link = Some(Link {
            elm,
            info,
            kind: protocollo.kind,
            t0: Instant::now(),
            piano: None,
            recorder: None,
            diagnosi: None,
            errori_di_fila: 0,
            distanza: IntegratoreDistanza::new(),
        });
        self.stato(StatoConn::Connesso);
    }

    /// Chiude il collegamento conservando la registrazione in corso.
    /// `motivo` è presente quando la chiusura non è stata chiesta dall'utente.
    fn chiudi_link(&mut self, motivo: Option<String>, notifica: bool) {
        if self.link.is_none() {
            return;
        }
        self.ferma_registrazione();
        if let Some(mut l) = self.link.take() {
            l.elm.chiudi();
        }
        if let Some(m) = &self.mqtt {
            m.scollegato();
        }
        if notifica {
            self.invia(Evento::Polling(false));
            if let Some(m) = &motivo {
                self.invia(Evento::Errore(ErroreUtente::CollegamentoPerso(m.clone())));
            }
            self.invia(Evento::Scollegato { motivo });
            self.stato(StatoConn::Scollegato);
        }
    }

    // ---- Diagnosi ------------------------------------------------------

    fn leggi_diagnosi(&mut self) {
        let Some(l) = self.link.as_mut() else {
            return;
        };
        let _ = l.kind;
        let traccia_prima = l.elm.traccia;
        l.elm.traccia = true;
        let _ = self.tx.send(Evento::Stato(StatoConn::Diagnosi));
        (self.notifica)();
        let esito = leggi_su(l);
        let registro = l.elm.prendi_registro();
        l.elm.traccia = traccia_prima;
        match esito {
            Ok(report) => {
                let mut errore = None;
                if let Some(r) = l.recorder.as_mut()
                    && let Err(e) = r.diagnosi(&report)
                {
                    errore = Some(e.to_string());
                }
                l.diagnosi = Some(report.clone());
                self.mqtt_diagnosi(&report);
                self.invia(Evento::Registro(registro));
                self.invia(Evento::Diagnosi(Box::new(report)));
                self.stato(StatoConn::Connesso);
                if let Some(e) = errore {
                    self.errore_registrazione(e);
                }
            }
            Err(s) => {
                self.invia(Evento::Registro(registro));
                self.chiudi_link(Some(s.0), true);
            }
        }
    }

    // ---- Cancellazione dei codici guasto -------------------------------

    /// Copia della diagnosi, servizio 04, rilettura. Il polling non gira
    /// durante la chiamata: il thread è uno solo.
    fn cancella_dtc(&mut self, conferma: ConfermaCancellazione) {
        let Some(mut l) = self.link.take() else {
            self.invia(Evento::Errore(ErroreUtente::CancellazioneNonEseguita(
                "L'adattatore non è collegato: collegalo, porta il quadro su ON con il motore \
                 spento e riprova."
                    .into(),
            )));
            return;
        };
        self.stato(StatoConn::Cancellazione);
        let traccia_prima = l.elm.traccia;
        l.elm.traccia = true;
        let fine = self.sequenza_cancellazione(&mut l, conferma);
        l.elm.traccia = traccia_prima;
        let registro = l.elm.prendi_registro();
        self.link = Some(l);
        self.invia(Evento::Registro(registro));
        match fine {
            FineCancellazione::Completata { errore_file } => {
                self.stato(StatoConn::Connesso);
                if let Some(e) = errore_file {
                    self.errore_registrazione(e);
                }
            }
            FineCancellazione::NonEseguita(motivo) => {
                self.invia(Evento::Errore(ErroreUtente::CancellazioneNonEseguita(
                    motivo,
                )));
                self.stato(StatoConn::Connesso);
            }
            FineCancellazione::Caduta(m) => self.chiudi_link(Some(m), true),
        }
    }

    fn sequenza_cancellazione(
        &self,
        l: &mut Link,
        conferma: ConfermaCancellazione,
    ) -> FineCancellazione {
        let mut errore_file: Option<String> = None;
        let mut annota = |esito: std::io::Result<()>| {
            if let Err(e) = esito
                && errore_file.is_none()
            {
                errore_file = Some(e.to_string());
            }
        };

        // 1. La copia deve rispecchiare ciò che sta per essere cancellato: la
        //    diagnosi viene sempre riletta, anche se ne esiste una precedente.
        {
            let report = match leggi_su(l) {
                Ok(r) => r,
                Err(s) => return FineCancellazione::Caduta(s.0),
            };
            if let Some(r) = l.recorder.as_mut() {
                annota(r.diagnosi(&report));
            }
            self.mqtt_diagnosi(&report);
            self.invia(Evento::Diagnosi(Box::new(report.clone())));
            l.diagnosi = Some(report);
        }

        // 2. Copia su disco: senza copia non si cancella.
        let Some(prima) = l.diagnosi.as_ref() else {
            return FineCancellazione::NonEseguita("Diagnosi non disponibile.".into());
        };
        let copia = match recording::salva_copia_diagnosi(&self.cartella, prima) {
            Ok(p) => p,
            Err(e) => {
                return FineCancellazione::NonEseguita(format!(
                    "Impossibile salvare la copia della diagnosi in {}: {e}. Senza la copia \
                     l'app non cancella i codici.",
                    self.cartella.display()
                ));
            }
        };

        // 3. Servizio 04.
        let risultato = |esito: EsitoCancellazione| RisultatoCancellazione {
            esito,
            demo: l.info.demo,
            copia: copia.clone(),
            eseguita_il: adesso_iso(),
        };
        let esito = match l.elm.cancella_dtc(conferma) {
            Ok(e) => e,
            Err(s) => {
                let r = risultato(EsitoCancellazione::ErroreComunicazione(format!(
                    "collegamento interrotto durante la cancellazione, esito sconosciuto: {}",
                    s.0
                )));
                self.mqtt_cancellazione(&r);
                self.invia(Evento::CancellazioneDtc(Box::new(r)));
                return FineCancellazione::Caduta(s.0);
            }
        };
        let accettata = esito.accettata();
        let r = risultato(esito);
        self.mqtt_cancellazione(&r);
        self.invia(Evento::CancellazioneDtc(Box::new(r)));
        let m = Marker {
            t: l.t0.elapsed().as_secs_f64(),
            ts: adesso_iso(),
            etichetta: if accettata {
                MARCATORE_CANCELLAZIONE
            } else {
                MARCATORE_CANCELLAZIONE_FALLITA
            }
            .to_string(),
        };
        if let Some(r) = l.recorder.as_mut() {
            annota(r.marcatore(&m));
        }
        self.invia(Evento::Marcatore(m));

        // 4. Rilettura della diagnosi, comunque sia andata.
        self.stato(StatoConn::Diagnosi);
        let report = match leggi_su(l) {
            Ok(r) => r,
            Err(s) => return FineCancellazione::Caduta(s.0),
        };
        if let Some(r) = l.recorder.as_mut() {
            annota(r.diagnosi(&report));
        }
        self.mqtt_diagnosi(&report);
        self.invia(Evento::Diagnosi(Box::new(report.clone())));
        l.diagnosi = Some(report);
        FineCancellazione::Completata { errore_file }
    }

    // ---- Polling -------------------------------------------------------

    fn passo_polling(&mut self) {
        let Some(l) = self.link.as_mut() else {
            return;
        };
        let Some(pid) = l.piano.as_mut().and_then(|p| p.prossimo()) else {
            return;
        };
        let esito = match l.elm.interroga(ObdRequest::CurrentData(pid)) {
            Ok(e) => e,
            Err(s) => {
                self.chiudi_link(Some(s.0), true);
                return;
            }
        };
        let t = l.t0.elapsed().as_secs_f64();
        let ts = adesso_iso();
        let nuovo = |par: &pids::Param, ecu: &str, valore: Option<f64>, stato: Stato| Sample {
            t,
            ts: ts.clone(),
            ecu: ecu.to_string(),
            key: par.key.clone(),
            pid,
            valore,
            unita: par.unita.to_string(),
            stato,
        };
        let mut campioni = Vec::new();
        match &esito {
            Esito::Dati(msg) => {
                l.errori_di_fila = 0;
                for m in msg {
                    for par in pids::per_pid(pid) {
                        match pids::decodifica(par, &m.data[2..]) {
                            Decodifica::Valore(v) => {
                                campioni.push(nuovo(par, &m.ecu, Some(v), Stato::Ok));
                            }
                            Decodifica::NonApplicabile => {}
                            Decodifica::Corta => {
                                campioni.push(nuovo(par, &m.ecu, None, Stato::ErroreComunicazione));
                            }
                        }
                    }
                }
            }
            altro => {
                let stato = match altro {
                    Esito::NonSupportato => Stato::NonSupportato,
                    Esito::Mancante => Stato::RispostaMancante,
                    _ => Stato::ErroreComunicazione,
                };
                if stato == Stato::ErroreComunicazione {
                    l.errori_di_fila += 1;
                } else {
                    l.errori_di_fila = 0;
                }
                for par in pids::per_pid(pid) {
                    campioni.push(nuovo(par, "", None, stato));
                }
            }
        }
        // Distanza parziale: dalla velocità della centralina principale, nello
        // stesso gruppo di campioni così viene registrata come ogni altro dato.
        let principale = l.info.ecu.first().cloned().unwrap_or_default();
        let derivati: Vec<Sample> = campioni
            .iter()
            .filter(|c| c.key == "velocita" && c.ecu == principale)
            .filter_map(|c| {
                let km = l
                    .distanza
                    .campione(c.t, c.valore.filter(|_| c.stato == Stato::Ok))?;
                Some(Sample {
                    t: c.t,
                    ts: c.ts.clone(),
                    ecu: c.ecu.clone(),
                    key: pids::CHIAVE_DISTANZA_PARZIALE.to_string(),
                    pid: 0x00,
                    valore: Some(km),
                    unita: "km".to_string(),
                    stato: Stato::Ok,
                })
            })
            .collect();
        campioni.extend(derivati);
        let mut errore_file = None;
        if let Some(r) = l.recorder.as_mut() {
            for c in &campioni {
                if let Err(e) = r.campione(c) {
                    errore_file = Some(e.to_string());
                    break;
                }
            }
        }
        let troppi_errori = l.errori_di_fila >= MAX_ERRORI_DI_FILA;
        // Non blocca mai: la coda verso il broker scarta se è piena.
        if let Some(m) = &self.mqtt {
            m.campioni(&campioni);
        }
        self.invia(Evento::Campioni(campioni));
        if let Some(e) = errore_file {
            self.errore_registrazione(e);
        }
        if troppi_errori {
            self.chiudi_link(
                Some(format!(
                    "{MAX_ERRORI_DI_FILA} richieste consecutive senza risposta valida"
                )),
                true,
            );
        }
    }

    // ---- Riproduzione --------------------------------------------------

    fn rifiuta(&self, motivo: &str) {
        self.invia(Evento::Errore(ErroreUtente::Riproduzione(
            motivo.to_string(),
        )));
    }

    /// Avvia la riproduzione: un `Connesso` costruito dall'intestazione della
    /// sessione, poi la diagnosi registrata, poi i campioni nel tempo.
    ///
    /// La diagnosi registrata non viene pubblicata su MQTT: l'argomento
    /// trattenuto `diagnosi` descrive lo stato attuale del veicolo.
    fn collega_riproduzione(&mut self, s: SessionData) {
        recording::registra_parametri(&s.intestazione);
        let p = riproduzione::presenti(&s);
        let i = &s.intestazione;
        let file = s
            .percorso
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "sessione".into());
        let mut avvisi = Vec::new();
        let importata = i
            .importata
            .as_ref()
            .map(|imp| format!("{} da «{}»", imp.formato, imp.file));
        if let Some(imp) = &importata {
            avvisi.push(format!("Sessione importata: {imp}."));
        }
        let vuoto = |t: &str| {
            if t.trim().is_empty() {
                "non indicato".to_string()
            } else {
                t.to_string()
            }
        };
        let info = InfoConn {
            demo: i.demo,
            origine: Origine::Riproduzione,
            riproduzione: Some(InfoRiproduzione {
                file: file.clone(),
                registrata_il: i.creata.clone(),
                sessione_demo: i.demo,
                importata,
            }),
            liberi: p.liberi,
            collegamento: format!("Riproduzione della sessione «{file}»"),
            adattatore: vuoto(&i.adattatore),
            protocollo: vuoto(&i.protocollo),
            protocollo_numero: 0,
            ecu: if i.ecu.is_empty() {
                p.ecu
            } else {
                i.ecu.clone()
            },
            supportati: p.supportati,
            unione: p.unione,
            gatt: None,
            avvisi,
        };
        self.invia(Evento::Connesso(Box::new(info)));
        if let Some(m) = &self.mqtt {
            m.collegato_origine(Origine::Riproduzione);
        }
        if let Some(d) = &s.diagnosi {
            self.invia(Evento::Diagnosi(Box::new(d.clone())));
        }
        self.riproduzione = Some(Riproduzione {
            motore: Riproduttore::da_sessione(&s),
            ultimo_passo: Instant::now(),
            inviato: None,
        });
        self.stato(StatoConn::Connesso);
        self.invia_stato_riproduzione(true);
    }

    fn comando_riproduzione(&mut self, c: ComandoRiproduzione) {
        let Some(r) = self.riproduzione.as_mut() else {
            return;
        };
        if c == ComandoRiproduzione::Riprendi {
            // Il tempo passato in pausa non conta.
            r.ultimo_passo = Instant::now();
        }
        if let Some(posizione) = r.motore.comando(c) {
            self.invia(Evento::Salto { posizione });
        }
        self.invia_stato_riproduzione(true);
    }

    /// Consegna i campioni il cui momento è arrivato, per la stessa strada dei
    /// dati dal vivo (interfaccia e MQTT).
    fn passo_riproduzione(&mut self) {
        let Some(r) = self.riproduzione.as_mut() else {
            return;
        };
        let ora = Instant::now();
        let dt = ora.duration_since(r.ultimo_passo).as_secs_f64();
        r.ultimo_passo = ora;
        let u = r.motore.avanza(dt);
        if !u.campioni.is_empty() {
            if let Some(m) = &self.mqtt {
                m.campioni(&u.campioni);
            }
            self.invia(Evento::Campioni(u.campioni));
        }
        for m in u.marcatori {
            if let Some(mq) = &self.mqtt {
                mq.marcatore(&m);
            }
            self.invia(Evento::Marcatore(m));
        }
        if let Some(posizione) = u.salto {
            self.invia(Evento::Salto { posizione });
        }
        self.invia_stato_riproduzione(u.finita_ora || u.salto.is_some());
    }

    /// Invia lo stato della riproduzione se è cambiato (o, mentre scorre,
    /// alcune volte al secondo).
    fn invia_stato_riproduzione(&mut self, forza: bool) {
        let Some(r) = self.riproduzione.as_mut() else {
            return;
        };
        let s = r.motore.stato();
        let da_inviare = forza
            || match &r.inviato {
                None => true,
                Some((u, quando)) => {
                    !u.stesso_salvo_posizione(&s)
                        || (u.posizione != s.posizione
                            && quando.elapsed() >= INTERVALLO_STATO_RIPRODUZIONE)
                }
            };
        if da_inviare {
            r.inviato = Some((s.clone(), Instant::now()));
            self.invia(Evento::Riproduzione(s));
        }
    }

    fn chiudi_riproduzione(&mut self, notifica: bool) {
        if self.riproduzione.take().is_none() {
            return;
        }
        if let Some(m) = &self.mqtt {
            m.scollegato();
        }
        if notifica {
            self.invia(Evento::Scollegato { motivo: None });
            self.stato(StatoConn::Scollegato);
        }
    }

    // ---- Registrazione -------------------------------------------------

    fn avvia_registrazione(&mut self) {
        let Some(l) = self.link.as_mut() else {
            return;
        };
        if l.recorder.is_some() {
            return;
        }
        let intestazione = Intestazione {
            versione: 1,
            creata: adesso_iso(),
            demo: l.info.demo,
            adattatore: l.info.adattatore.clone(),
            collegamento: l.info.collegamento.clone(),
            protocollo: l.info.protocollo.clone(),
            ecu: l.info.ecu.clone(),
            importata: None,
            parametri: Default::default(),
        };
        let t_inizio = l.t0.elapsed().as_secs_f64();
        match Recorder::avvia(&self.cartella, &intestazione, l.diagnosi.as_ref(), t_inizio) {
            Ok(r) => {
                let percorso = r.percorso().to_path_buf();
                l.recorder = Some(r);
                self.invia(Evento::RegistrazioneAvviata { percorso, t_inizio });
            }
            Err(e) => self.invia(Evento::Errore(ErroreUtente::Registrazione(format!(
                "Impossibile creare il file di sessione in {}: {e}",
                self.cartella.display()
            )))),
        }
    }

    fn ferma_registrazione(&mut self) {
        let Some(r) = self.link.as_mut().and_then(|l| l.recorder.take()) else {
            return;
        };
        let campioni = r.campioni;
        match r.chiudi() {
            Ok(percorso) => {
                // Il CSV viene scritto subito accanto alla sessione.
                let csv = recording::carica(&percorso)
                    .and_then(|s| recording::esporta_csv(&s))
                    .ok();
                self.invia(Evento::RegistrazioneFermata {
                    percorso,
                    csv,
                    campioni,
                });
            }
            Err(e) => self.invia(Evento::Errore(ErroreUtente::Registrazione(format!(
                "Chiusura del file di sessione fallita: {e}"
            )))),
        }
    }

    fn errore_registrazione(&mut self, dettaglio: String) {
        self.ferma_registrazione();
        self.invia(Evento::Errore(ErroreUtente::Registrazione(format!(
            "Scrittura della sessione fallita, registrazione fermata: {dettaglio}"
        ))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piano_con_priorita_alle_sonde() {
        let supportati: BTreeSet<u8> = [0x06, 0x07, 0x0C, 0x15, 0x24].into_iter().collect();
        let chiavi: Vec<String> = [
            "o2s1_lambda",
            "o2s2_tensione",
            "stft_b1",
            "ltft_b1",
            "giri",
            "velocita",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let prioritarie = vec!["o2s1_lambda".to_string(), "o2s2_tensione".to_string()];
        let mut p = Piano::new(&chiavi, &prioritarie, &supportati);
        let sequenza: Vec<u8> = (0..9).map(|_| p.prossimo().unwrap()).collect();
        // Sonde a ogni giro, gli altri a rotazione; la velocità (0D) non è supportata.
        assert_eq!(
            sequenza,
            vec![0x24, 0x15, 0x06, 0x24, 0x15, 0x07, 0x24, 0x15, 0x0C]
        );
    }

    #[test]
    fn distanza_a_velocita_costante() {
        let mut d = IntegratoreDistanza::new();
        let mut km = None;
        // 72 km/h per 10 s, un campione ogni 0,5 s: 20 m al secondo, 200 m.
        for i in 0..=20 {
            km = d.campione(i as f64 * 0.5, Some(72.0));
        }
        assert!((km.unwrap() - 0.2).abs() < 1e-9, "{km:?}");
    }

    #[test]
    fn distanza_trapezio_su_rampa() {
        let mut d = IntegratoreDistanza::new();
        assert_eq!(d.campione(0.0, Some(0.0)), Some(0.0));
        // Da 0 a 36 km/h in 2 s: media 18 km/h, 10 m.
        let km = d.campione(2.0, Some(36.0)).unwrap();
        assert!((km - 0.010).abs() < 1e-9);
    }

    #[test]
    fn distanza_non_attraversa_i_buchi() {
        let mut d = IntegratoreDistanza::new();
        d.campione(0.0, Some(72.0));
        let prima = d.campione(1.0, Some(72.0)).unwrap();
        // Silenzio di 30 s: nessuna distanza aggiunta.
        let dopo = d.campione(31.0, Some(72.0)).unwrap();
        assert_eq!(prima, dopo);
        // Ripresa regolare dopo il buco.
        let dopo2 = d.campione(32.0, Some(72.0)).unwrap();
        assert!((dopo2 - prima - 0.02).abs() < 1e-9);
    }

    #[test]
    fn distanza_ignora_i_campioni_mancanti() {
        let mut d = IntegratoreDistanza::new();
        // Senza alcuna velocità valida non c'è un valore, nemmeno zero.
        assert_eq!(d.campione(0.0, None), None);
        d.campione(1.0, Some(36.0));
        assert_eq!(d.campione(2.0, None), None);
        // Un campione mancante nel mezzo non aggiunge nulla; il successivo
        // integra dall'ultimo valido (1 s → 3 s, 2 s ≤ 5 s).
        let km = d.campione(3.0, Some(36.0)).unwrap();
        assert!((km - 0.020).abs() < 1e-9);
    }

    #[test]
    fn piano_ignora_i_parametri_calcolati() {
        let supportati: BTreeSet<u8> = [0x0D].into_iter().collect();
        let chiavi = vec![
            pids::CHIAVE_DISTANZA_PARZIALE.to_string(),
            "velocita".to_string(),
        ];
        let mut p = Piano::new(&chiavi, &[], &supportati);
        assert_eq!(p.prossimo(), Some(0x0D));
        assert_eq!(p.prossimo(), Some(0x0D));
        assert!(Piano::new(&chiavi[..1], &[], &BTreeSet::from([0x00])).vuoto());
    }

    #[test]
    fn piano_senza_sonde_o_vuoto() {
        let supportati: BTreeSet<u8> = [0x0C, 0x0D].into_iter().collect();
        let chiavi = vec!["giri".to_string(), "velocita".to_string()];
        let mut p = Piano::new(&chiavi, &[], &supportati);
        assert_eq!(
            (0..4).map(|_| p.prossimo().unwrap()).collect::<Vec<_>>(),
            vec![0x0C, 0x0D, 0x0C, 0x0D]
        );
        let mut solo_veloci = Piano::new(&chiavi, &chiavi, &supportati);
        assert_eq!(
            (0..4)
                .map(|_| solo_veloci.prossimo().unwrap())
                .collect::<Vec<_>>(),
            vec![0x0C, 0x0D, 0x0C, 0x0D]
        );
        assert!(Piano::new(&chiavi, &[], &BTreeSet::new()).vuoto());
    }
}
