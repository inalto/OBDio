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
use crate::riproduzione::data_leggibile;
use crate::scambio;
use crate::transport::ble::{BleDispositivo, GattInfo};
use crate::transport::serial::PortaSeriale;
use crate::worker::{
    self, Comando, ErroreUtente, Evento, InfoConn, RisultatoCancellazione, Sorgente, StatoConn,
    StatoRiproduzione,
};

#[cfg(feature = "collaudo")]
mod collaudo;
mod cruscotto;
pub mod dati;
mod grafici;
mod impostazioni;
mod pagine;
pub(crate) mod scelta_file;
mod sensori;
mod sessioni;
pub(crate) mod tema;
mod trasporto;

use dati::Archivio;
pub(crate) use sensori::VistaSensori;

// Nomi storici dei colori, ora alias dei token di `tema` (unica fonte).
pub(crate) use tema::{
    ATTENZIONE as GIALLO, DEMO as ARANCIO_DEMO, OK as VERDE, PERICOLO as ROSSO, TESTO_2 as GRIGIO,
};

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

    #[cfg(feature = "collaudo")]
    collaudo: Option<collaudo::Collaudo>,
}

/// Avvia l'applicazione.
pub fn esegui() -> eframe::Result {
    let opzioni = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OBDio — Diagnostica OBD-II")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([980.0, 620.0]),
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
        App {
            tx,
            rx,
            lavoro: Some(lavoro),
            elenco: recording::elenca(&cartella),
            cartella,
            scheda: Scheda::Connessione,
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

    fn testo_stato(&self) -> (String, Color32) {
        match &self.stato {
            StatoConn::Scollegato => ("Non collegato".into(), GRIGIO),
            StatoConn::Scansione => ("Scansione Bluetooth in corso…".into(), GIALLO),
            StatoConn::Connessione(fase) => (fase.clone(), GIALLO),
            StatoConn::Connesso if self.in_riproduzione() => {
                let testo = match &self.riproduzione {
                    Some(r) if r.finita => "Riproduzione finita",
                    Some(r) if r.in_pausa => "Riproduzione in pausa",
                    _ => "Riproduzione in corso",
                };
                (testo.into(), ARANCIO_DEMO)
            }
            StatoConn::Connesso => ("Collegato".into(), VERDE),
            StatoConn::Diagnosi => ("Lettura della diagnosi in corso…".into(), GIALLO),
            StatoConn::Cancellazione => ("Cancellazione dei codici guasto in corso…".into(), ROSSO),
            StatoConn::Disconnessione => ("Disconnessione in corso…".into(), GIALLO),
        }
    }

    fn barra_superiore(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading("OBDio");
            ui.separator();
            for (scheda, nome) in [
                (Scheda::Connessione, "Connessione"),
                (Scheda::Diagnosi, "Codici guasto e fermo immagine"),
                (Scheda::Cruscotto, "Cruscotto"),
                (Scheda::Sensori, "Sensori"),
                (Scheda::Sessioni, "Sessioni salvate"),
                (Scheda::Impostazioni, "Impostazioni"),
            ] {
                ui.selectable_value(&mut self.scheda, scheda, nome);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (testo, colore) = self.testo_stato();
                ui.label(RichText::new(testo).color(colore).strong());
                if let Some(r) = &self.registrazione {
                    let s = (self.live.t_max - r.t_inizio).max(0.0) as u64;
                    ui.label(
                        RichText::new(format!(
                            "REGISTRAZIONE IN CORSO {:02}:{:02}",
                            s / 60,
                            s % 60
                        ))
                        .color(ROSSO)
                        .strong(),
                    );
                } else if self.polling {
                    ui.label(RichText::new("Lettura dati attiva").color(VERDE));
                }
                if let Some((testo, colore)) = self.impostazioni.testo_barra() {
                    ui.label(RichText::new(testo).color(colore));
                }
            });
        });
        ui.add_space(2.0);

        let demo_in_vista = match self.scheda {
            Scheda::Sessioni => self
                .aperta
                .as_ref()
                .is_some_and(|s| s.dati.intestazione.demo),
            Scheda::Sensori | Scheda::Cruscotto => {
                self.info.as_ref().is_some_and(|i| i.demo)
                    || (self.live_demo && !self.live.serie.is_empty())
            }
            Scheda::Diagnosi => {
                self.info.as_ref().is_some_and(|i| i.demo)
                    || self.diagnosi.as_ref().is_some_and(|d| d.demo)
            }
            Scheda::Connessione => self.info.as_ref().is_some_and(|i| i.demo),
            Scheda::Impostazioni => false,
        };
        if let Some(r) = self.info.as_ref().and_then(|i| i.riproduzione.clone()) {
            // Per tutta la durata della riproduzione, su ogni pagina.
            egui::Frame::new()
                .fill(ARANCIO_DEMO)
                .inner_margin(egui::Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "RIPRODUZIONE — dati registrati il {}{}",
                                data_leggibile(&r.registrata_il),
                                if r.sessione_demo {
                                    ", sessione demo"
                                } else {
                                    ""
                                }
                            ))
                            .color(Color32::BLACK)
                            .strong(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "· non sono letture attuali · {}",
                                r.importata
                                    .clone()
                                    .map(|i| format!("importata: {i}"))
                                    .unwrap_or_else(|| r.file.clone())
                            ))
                            .color(Color32::BLACK),
                        );
                    });
                });
            self.barra_trasporto(ui);
        } else if demo_in_vista {
            egui::Frame::new()
                .fill(ARANCIO_DEMO)
                .inner_margin(egui::Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(
                            "MODALITÀ DEMO — tutti i dati sono simulati; i segnali delle sonde \
                             lambda sono sintetici e non provengono da un veicolo",
                        )
                        .color(Color32::BLACK)
                        .strong(),
                    );
                });
        }
        if let Some(e) = self.errore.clone() {
            egui::Frame::new()
                .fill(Color32::from_rgb(90, 25, 25))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(e.titolo()).strong().color(Color32::WHITE));
                        ui.label(RichText::new(e.messaggio()).color(Color32::WHITE));
                        if ui.button("Chiudi").clicked() {
                            self.errore = None;
                        }
                    });
                });
        }
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
        let _ = ctx;
        self.ricevi();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(feature = "collaudo")]
        collaudo::passo(self, ui.ctx());
        self.governa_polling_cruscotto();
        egui::Panel::top("barra").show(ui, |ui| self.barra_superiore(ui));
        egui::CentralPanel::default().show(ui, |ui| match self.scheda {
            Scheda::Connessione => self.pagina_connessione(ui),
            Scheda::Diagnosi => self.pagina_diagnosi(ui),
            Scheda::Cruscotto => self.pagina_cruscotto(ui),
            Scheda::Sensori => self.pagina_sensori(ui),
            Scheda::Sessioni => self.pagina_sessioni(ui),
            Scheda::Impostazioni => self.pagina_impostazioni(ui),
        });
        // Sopra a tutto, qualunque sia la scheda.
        let ctx = ui.ctx().clone();
        self.finestra_cancellazione(&ctx);
        self.finestre_sessioni(&ctx);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Il thread di lavoro chiude la registrazione in corso prima di uscire.
        let _ = self.tx.send(Comando::Esci);
        if let Some(h) = self.lavoro.take() {
            let _ = h.join();
        }
    }
}
