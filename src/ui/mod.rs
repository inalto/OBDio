//! Interfaccia egui. Non esegue mai I/O verso l'adattatore: invia comandi al
//! thread di lavoro e ne mostra gli eventi.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use eframe::egui::{self, Color32, RichText};

use crate::diagnosis::DiagnosisReport;
use crate::model::Stato;
use crate::obd::pids::PresetP0420;
use crate::recording::{self, SessionData};
use crate::scambio;
use crate::transport::ble::{BleDispositivo, GattInfo};
use crate::transport::serial::PortaSeriale;
use crate::worker::{
    self, Comando, ErroreUtente, Evento, InfoConn, RisultatoCancellazione, Sorgente, StatoConn,
    StatoRiproduzione,
};

#[cfg(feature = "collaudo")]
mod collaudo;
mod componenti;
mod cruscotto;
pub mod dati;
mod grafici;
mod icone;
mod impostazioni;
#[cfg(not(target_os = "android"))]
mod impostazioni_aggiornamenti;
mod impostazioni_mcp;
mod navigazione;
mod pagine;
pub(crate) mod scelta_file;
mod sensori;
mod sessioni;
pub(crate) mod tema;
mod trasporto;

use dati::Archivio;
pub(crate) use sensori::{PannelloSensori, VistaSensori};

// Nomi storici dei colori, ora alias dei token di `tema` (unica fonte).
pub(crate) use tema::{ATTENZIONE as GIALLO, OK as VERDE, PERICOLO as ROSSO, TESTO_2 as GRIGIO};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scheda {
    Connessione,
    Diagnosi,
    Cruscotto,
    Sensori,
    Sessioni,
    Impostazioni,
}

pub(crate) struct Registrazione {
    pub percorso: PathBuf,
    /// Tempo sull'asse dal vivo in cui è iniziata.
    pub t_inizio: f64,
}

pub(crate) struct UltimaSessione {
    // Letto solo dal collaudo automatico.
    #[cfg_attr(not(feature = "collaudo"), allow(dead_code))]
    pub percorso: PathBuf,
    pub csv: Option<PathBuf>,
    pub campioni: usize,
}

pub(crate) struct SessioneAperta {
    pub dati: SessionData,
    pub archivio: Archivio,
    pub preset: PresetP0420,
    pub extra: Option<String>,
    pub messaggio: Option<String>,
    /// Riepilogo dell'importazione, se la sessione è appena stata importata.
    pub riepilogo: Option<scambio::Riepilogo>,
}

impl SessioneAperta {
    pub fn new(dati: SessionData, riepilogo: Option<scambio::Riepilogo>) -> Self {
        let archivio = Archivio::da_sessione(&dati);
        let preset = PresetP0420::calcola(&archivio.pid_presenti());
        SessioneAperta {
            dati,
            archivio,
            preset,
            extra: None,
            messaggio: None,
            riepilogo,
        }
    }
}

pub struct App {
    pub(crate) tx: Sender<Comando>,
    rx: Receiver<Evento>,
    lavoro: Option<JoinHandle<()>>,
    pub(crate) cartella: PathBuf,
    pub(crate) scheda: Scheda,
    /// Disposizione del fotogramma corrente (larga o stretta).
    pub(crate) disposizione: tema::Disposizione,
    /// Finestra «Salvare le modifiche al cruscotto?» aperta.
    pub(crate) chiedi_chiusura: bool,
    /// L'utente ha scelto di chiudere: la prossima richiesta passa.
    pub(crate) chiusura_confermata: bool,
    pub(crate) stato: StatoConn,
    pub(crate) info: Option<InfoConn>,
    pub(crate) errore: Option<ErroreUtente>,
    pub(crate) registro: Vec<String>,

    // Connessione
    pub(crate) dispositivi: Vec<BleDispositivo>,
    pub(crate) ble_scelto: Option<String>,
    pub(crate) mostra_senza_nome: bool,
    pub(crate) gatt_sconosciuto: Option<GattInfo>,
    pub(crate) porte: Vec<PortaSeriale>,
    pub(crate) porta_scelta: String,
    pub(crate) baud: u32,

    // Diagnosi
    pub(crate) diagnosi: Option<DiagnosisReport>,
    pub(crate) includi_vin: bool,
    /// Scorrimento da applicare una volta alla pagina della diagnosi.
    pub(crate) scorri_diagnosi: Option<f32>,
    /// Finestra di conferma della cancellazione aperta; il valore è la spunta
    /// «Ho letto e voglio cancellare i codici».
    pub(crate) conferma_cancellazione: Option<bool>,
    /// Esito dell'ultima cancellazione dei codici guasto.
    pub(crate) esito_cancellazione: Option<RisultatoCancellazione>,

    // Sensori
    pub(crate) live: Archivio,
    pub(crate) live_demo: bool,
    pub(crate) preset: PresetP0420,
    /// Unica fonte di verità dei parametri da leggere (chiave → attivo).
    pub(crate) selezione: BTreeMap<String, bool>,
    /// Testo del campo di ricerca nell'elenco dei sensori.
    pub(crate) ricerca: String,
    pub(crate) vista: VistaSensori,
    /// Elenco o grafici, sul layout stretto.
    pub(crate) pannello_sensori: PannelloSensori,
    /// Scorrimento da applicare una volta alla griglia dei grafici per sensore.
    pub(crate) scorri_sensori: Option<f32>,
    pub(crate) polling: bool,
    pub(crate) registrazione: Option<Registrazione>,
    pub(crate) ultima: Option<UltimaSessione>,
    pub(crate) testo_marcatore: String,
    pub(crate) finestra: Option<f64>,
    pub(crate) extra: Option<String>,

    // Sessioni
    pub(crate) elenco: Vec<PathBuf>,
    pub(crate) aperta: Option<SessioneAperta>,
    pub(crate) caricamento: Option<Receiver<Result<SessionData, String>>>,
    pub(crate) errore_sessione: Option<String>,
    /// Importazione, esportazione e scelta della sessione da rivivere.
    pub(crate) sessioni: sessioni::StatoSessioni,

    // Riproduzione
    /// Stato della riproduzione in corso (barra di trasporto).
    pub(crate) riproduzione: Option<StatoRiproduzione>,
    /// Posizione mostrata mentre si trascina il cursore della barra.
    pub(crate) trascina_posizione: Option<f64>,

    // Cruscotto
    pub(crate) cruscotto: cruscotto::StatoCruscotto,
    // Impostazioni (MQTT)
    pub(crate) impostazioni: impostazioni::Impostazioni,
    // Impostazioni (server MCP per assistenti AI)
    pub(crate) mcp: impostazioni_mcp::StatoMcpUi,
    // Impostazioni (aggiornamenti da GitHub; non su Android)
    #[cfg(not(target_os = "android"))]
    pub(crate) aggiornamenti: impostazioni_aggiornamenti::StatoAggiornamentiUi,

    #[cfg(feature = "collaudo")]
    collaudo: Option<collaudo::Collaudo>,
}

/// Avvia l'applicazione.
pub fn esegui() -> eframe::Result {
    let opzioni = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OBDio — Diagnostica OBD-II")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size(tema::FINESTRA_MIN),
        ..Default::default()
    };
    eframe::run_native("OBDio", opzioni, Box::new(|cc| Ok(crea_app(cc))))
}

/// Costruisce l'app: condivisa tra l'ingresso desktop e `android_main`.
pub fn crea_app(cc: &eframe::CreationContext<'_>) -> Box<dyn eframe::App> {
    Box::new(App::new(cc))
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        tema::applica(&cc.egui_ctx);
        let cartella = recording::cartella_sessioni();
        let ctx = cc.egui_ctx.clone();
        let (tx, rx, lavoro) = worker::avvia(cartella.clone(), move || ctx.request_repaint());
        if crate::piattaforma::SERIALE_DISPONIBILE {
            let _ = tx.send(Comando::ElencaSeriali);
        }
        let impostazioni = impostazioni::Impostazioni::carica();
        if impostazioni.form.attivo {
            let _ = tx.send(Comando::ConfiguraMqtt(impostazioni.form.clone()));
        }
        let mcp = impostazioni_mcp::StatoMcpUi::carica();
        if mcp.config.attivo {
            let _ = tx.send(Comando::ConfiguraMcp(mcp.config.clone()));
        }
        App {
            tx,
            rx,
            lavoro: Some(lavoro),
            elenco: recording::elenca(&cartella),
            cartella,
            scheda: Scheda::Connessione,
            disposizione: tema::Disposizione::Larga,
            chiedi_chiusura: false,
            chiusura_confermata: false,
            stato: StatoConn::Scollegato,
            info: None,
            errore: None,
            registro: Vec::new(),
            dispositivi: Vec::new(),
            ble_scelto: None,
            mostra_senza_nome: false,
            gatt_sconosciuto: None,
            porte: Vec::new(),
            porta_scelta: String::new(),
            baud: 38400,
            diagnosi: None,
            includi_vin: false,
            scorri_diagnosi: None,
            conferma_cancellazione: None,
            esito_cancellazione: None,
            live: Archivio::default(),
            live_demo: false,
            preset: PresetP0420::default(),
            selezione: BTreeMap::new(),
            ricerca: String::new(),
            vista: VistaSensori::Confronto,
            pannello_sensori: PannelloSensori::Elenco,
            scorri_sensori: None,
            polling: false,
            registrazione: None,
            ultima: None,
            testo_marcatore: String::new(),
            finestra: Some(60.0),
            extra: None,
            aperta: None,
            caricamento: None,
            errore_sessione: None,
            sessioni: Default::default(),
            riproduzione: None,
            trascina_posizione: None,
            cruscotto: cruscotto::StatoCruscotto::carica(),
            impostazioni,
            mcp,
            #[cfg(not(target_os = "android"))]
            aggiornamenti: impostazioni_aggiornamenti::StatoAggiornamentiUi::carica(&cc.egui_ctx),
            #[cfg(feature = "collaudo")]
            collaudo: collaudo::Collaudo::da_ambiente(),
        }
    }

    pub(crate) fn invia(&self, c: Comando) {
        let _ = self.tx.send(c);
    }

    pub(crate) fn collegato(&self) -> bool {
        self.info.is_some()
    }

    /// Vero durante la riproduzione di una sessione registrata.
    pub(crate) fn in_riproduzione(&self) -> bool {
        self.info.as_ref().is_some_and(|i| i.in_riproduzione())
    }

    /// Vero se è collegato un adattatore (o il simulatore dal vivo).
    pub(crate) fn collegato_dal_vivo(&self) -> bool {
        self.info.as_ref().is_some_and(|i| !i.in_riproduzione())
    }

    /// Avvia la riproduzione della sessione (chiude quella in corso). Non va
    /// chiamata con un adattatore collegato: il thread di lavoro la ignorerebbe.
    pub(crate) fn avvia_riproduzione(&mut self, dati: SessionData) {
        if self.in_riproduzione() {
            self.invia(Comando::Scollega);
        }
        self.errore = None;
        self.trascina_posizione = None;
        self.invia(Comando::Collega(Sorgente::Riproduzione(Box::new(dati))));
        self.scheda = Scheda::Cruscotto;
    }

    /// Vero se il collegamento è libero per un nuovo comando di lettura.
    pub(crate) fn pronto(&self) -> bool {
        self.collegato() && self.stato == StatoConn::Connesso
    }

    /// Chiavi da interrogare secondo la selezione corrente e, fra queste,
    /// quelle prioritarie (le sonde del preset, se selezionate).
    pub(crate) fn chiavi_polling(&self) -> (Vec<String>, Vec<String>) {
        if self.scheda == Scheda::Cruscotto {
            return self.chiavi_polling_cruscotto();
        }
        sensori::chiavi_da_selezione(&self.selezione, &self.preset)
    }

    pub(crate) fn avvia_polling(&self) {
        let (chiavi, prioritarie) = self.chiavi_polling();
        self.invia(Comando::AvviaPolling {
            chiavi,
            prioritarie,
        });
    }

    pub(crate) fn apri_sessione(&mut self, percorso: PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(recording::carica(&percorso));
        });
        self.caricamento = Some(rx);
        self.errore_sessione = None;
    }

    fn ricevi(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Evento::Stato(s) => self.stato = s,
                Evento::DispositiviBle(d) => self.dispositivi = d,
                Evento::PorteSeriali(p) => {
                    if !p.iter().any(|x| x.nome == self.porta_scelta) {
                        self.porta_scelta = p
                            .iter()
                            .find(|x| x.tipo == "Bluetooth")
                            .or(p.first())
                            .map(|x| x.nome.clone())
                            .unwrap_or_default();
                    }
                    self.porte = p;
                }
                Evento::GattSconosciuto(g) => self.gatt_sconosciuto = Some(g),
                Evento::Connesso(info) => {
                    self.preset = PresetP0420::calcola(&info.unione);
                    self.selezione = sensori::selezione_p0420(&self.preset);
                    self.live = Archivio::default();
                    self.live_demo = info.demo;
                    self.diagnosi = None;
                    self.esito_cancellazione = None;
                    self.errore = None;
                    self.gatt_sconosciuto = None;
                    self.extra = None;
                    self.riproduzione = None;
                    self.trascina_posizione = None;
                    let riproduzione = info.in_riproduzione();
                    self.info = Some(*info);
                    if riproduzione {
                        // In riproduzione scorrono tutti i dati della sessione:
                        // la pagina Sensori li mostra tutti.
                        self.selezione = self
                            .parametri_disponibili()
                            .iter()
                            .map(|p| (p.key.clone(), true))
                            .collect();
                    }
                }
                Evento::Diagnosi(d) => self.diagnosi = Some(*d),
                Evento::CancellazioneDtc(r) => {
                    self.esito_cancellazione = Some(*r);
                    self.scheda = Scheda::Diagnosi;
                    self.scorri_diagnosi = Some(0.0);
                }
                Evento::Campioni(v) => {
                    for c in &v {
                        self.live.aggiungi(c);
                    }
                }
                Evento::Polling(attivo) => self.polling = attivo,
                Evento::RegistrazioneAvviata { percorso, t_inizio } => {
                    self.registrazione = Some(Registrazione { percorso, t_inizio });
                }
                Evento::RegistrazioneFermata {
                    percorso,
                    csv,
                    campioni,
                } => {
                    self.registrazione = None;
                    self.ultima = Some(UltimaSessione {
                        percorso,
                        csv,
                        campioni,
                    });
                    self.elenco = recording::elenca(&self.cartella);
                }
                Evento::Marcatore(m) => self.live.marcatori.push(m),
                Evento::Scollegato { .. } => {
                    if self.in_riproduzione() {
                        // Chiusa la riproduzione, i suoi dati non restano a
                        // confondersi con quelli di un collegamento futuro.
                        self.live = Archivio::default();
                        self.diagnosi = None;
                    }
                    self.info = None;
                    self.conferma_cancellazione = None;
                    self.polling = false;
                    self.registrazione = None;
                    self.riproduzione = None;
                    self.trascina_posizione = None;
                }
                Evento::Riproduzione(s) => self.riproduzione = Some(s),
                Evento::Salto { .. } => {
                    // Nessuna linea attraversa il salto: i grafici ripartono.
                    self.live = Archivio::default();
                }
                Evento::Errore(e) => {
                    if !matches!(
                        e,
                        ErroreUtente::Registrazione(_)
                            | ErroreUtente::CancellazioneNonEseguita(_)
                            | ErroreUtente::Riproduzione(_)
                    ) {
                        self.scheda = Scheda::Connessione;
                    }
                    self.errore = Some(e);
                }
                Evento::Mqtt(info) => self.impostazioni.info = info,
                Evento::Mcp(info) => self.mcp.info = info,
                Evento::Registro(righe) => {
                    self.registro.extend(righe);
                    if self.registro.len() > 600 {
                        let eccesso = self.registro.len() - 600;
                        self.registro.drain(..eccesso);
                    }
                }
            }
        }
        if let Some(rx) = &self.caricamento
            && let Ok(esito) = rx.try_recv()
        {
            self.caricamento = None;
            let riproduci = std::mem::take(&mut self.sessioni.riproduci_dopo);
            match esito {
                Ok(dati) => {
                    if riproduci && !self.collegato_dal_vivo() {
                        self.avvia_riproduzione(dati.clone());
                    }
                    self.aperta = Some(SessioneAperta::new(dati, None));
                }
                Err(e) => {
                    if riproduci {
                        self.errore = Some(ErroreUtente::Altro(format!(
                            "Sessione non riproducibile: {e}"
                        )));
                    }
                    self.errore_sessione = Some(e);
                }
            }
        }
        self.ricevi_importazione();
    }
}

/// Colore associato allo stato di una lettura.
pub(crate) fn colore_stato(stato: Stato) -> Color32 {
    match stato {
        Stato::Ok => VERDE,
        Stato::NonSupportato => GRIGIO,
        Stato::RispostaMancante => GIALLO,
        Stato::ErroreComunicazione => ROSSO,
    }
}

/// Etichetta colorata per lo stato di una lettura.
pub(crate) fn etichetta_stato(ui: &mut egui::Ui, stato: Stato) {
    ui.label(RichText::new(stato.etichetta()).color(colore_stato(stato)));
}

pub(crate) fn apri_cartella(percorso: &std::path::Path) {
    crate::piattaforma::apri_cartella(percorso);
}

impl eframe::App for App {
    /// Chiamata anche a finestra ridotta a icona: gli eventi vengono smaltiti
    /// comunque. La registrazione, in ogni caso, vive nel thread di lavoro.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ricevi();
        #[cfg(not(target_os = "android"))]
        self.passo_aggiornamenti(ctx);
        #[cfg(target_os = "android")]
        let _ = ctx;
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Le finestre di sistema per i file si legano a questa finestra.
        #[cfg(not(target_os = "android"))]
        scelta_file::registra_finestra(frame);
        #[cfg(target_os = "android")]
        let _ = frame;
        #[cfg(feature = "collaudo")]
        collaudo::passo(self, ui.ctx());
        let ctx = ui.ctx().clone();
        let d = tema::Disposizione::per_larghezza(ctx.content_rect().width());
        self.disposizione = d;
        if tema::adatta(&ctx, d) {
            ctx.request_repaint();
        }
        // Lo stile del contesto vale dal prossimo fotogramma: questo lo adegua subito.
        ui.spacing_mut().interact_size.y = d.bersaglio();
        self.scorciatoie(&ctx);
        self.governa_polling_cruscotto();

        let barra_nav = egui::Frame::new()
            .fill(tema::SUPERFICIE)
            .inner_margin(egui::Margin::same(tema::SP_2 as i8));
        if d.stretta() {
            egui::Panel::bottom("navigazione")
                .resizable(false)
                .exact_size(tema::ALTEZZA_BARRA_NAV)
                .frame(barra_nav.inner_margin(egui::Margin::ZERO))
                .show(ui, |ui| self.barra_inferiore(ui));
        } else {
            egui::Panel::left("navigazione")
                .resizable(false)
                .exact_size(tema::LARGHEZZA_BARRA_LATERALE)
                .frame(barra_nav)
                .show(ui, |ui| self.barra_laterale(ui));
        }
        let margine = d.margine() as i8;
        egui::Panel::top("barra")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(tema::SFONDO)
                    .inner_margin(egui::Margin {
                        left: margine,
                        right: margine,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| self.barra_superiore(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(tema::SFONDO)
                    .inner_margin(egui::Margin {
                        left: margine,
                        right: margine,
                        top: tema::SP_3 as i8,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| match self.scheda {
                Scheda::Connessione => self.pagina_connessione(ui),
                Scheda::Diagnosi => self.pagina_diagnosi(ui),
                Scheda::Cruscotto => self.pagina_cruscotto(ui),
                Scheda::Sensori => self.pagina_sensori(ui),
                Scheda::Sessioni => self.pagina_sessioni(ui),
                Scheda::Impostazioni => self.pagina_impostazioni(ui),
            });
        // Sopra a tutto, qualunque sia la scheda.
        self.finestra_cancellazione(&ctx);
        self.finestre_sessioni(&ctx);
        self.governa_chiusura(&ctx);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Il thread di lavoro chiude la registrazione in corso prima di uscire.
        let _ = self.tx.send(Comando::Esci);
        if let Some(h) = self.lavoro.take() {
            let _ = h.join();
        }
        // «Riavvia ora» dopo un aggiornamento: la versione nuova parte solo
        // ora che registrazione, collegamenti e server sono chiusi.
        #[cfg(not(target_os = "android"))]
        self.aggiornamenti.riavvia_se_richiesto();
    }
}
