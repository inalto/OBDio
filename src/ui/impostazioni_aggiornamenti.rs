//! Scheda «Aggiornamenti» della pagina Impostazioni e avviso nella barra di
//! stato (solo desktop; vedi docs/AGGIORNAMENTI.md).
//!
//! Il controllo e lo scaricamento girano nel thread di
//! [`crate::aggiornamenti::Servizio`]: qui si chiedono e se ne mostrano gli
//! esiti. Nessuna finestra si apre da sola: una versione nuova compare come
//! avviso nella barra di stato e come riquadro in questa scheda.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use super::componenti::{self, nota, paragrafo, riga_valore};
use super::impostazioni_mcp::interruttore;
use super::{App, GIALLO, ROSSO, VERDE, tema};
use crate::aggiornamenti::rete::{self, ImpostazioniRete};
use crate::aggiornamenti::{
    self, ConfigAggiornamenti, ErroreAggiornamento, Installato, Notizia, Rilascio, Servizio,
    Valutazione, Versione, rilascio, sostituzione,
};
use crate::config;
use crate::piattaforma;
use crate::worker::StatoConn;

/// Attesa dopo l'avvio prima del controllo automatico: l'avvio non aspetta
/// mai la rete.
const RITARDO_AVVIO: Duration = Duration::from_secs(8);

/// Che cosa sta facendo il thread degli aggiornamenti.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InCorso {
    Controllo,
    Scaricamento { scaricati: u64, totale: u64 },
}

/// Stato della scheda.
pub(crate) struct StatoAggiornamentiUi {
    pub config: ConfigAggiornamenti,
    servizio: Servizio,
    /// Eseguibile in uso.
    exe: Option<PathBuf>,
    avvio: Instant,
    /// Il controllo automatico di questo avvio è già stato deciso.
    automatico_deciso: bool,
    /// L'ultimo controllo l'ha chiesto l'utente.
    manuale: bool,
    pub in_corso: Option<InCorso>,
    /// Versione nuova trovata (o ignorata ma mostrata dopo un controllo manuale).
    pub rilascio: Option<Box<Rilascio>>,
    pub valutazione: Option<Valutazione>,
    /// «Più tardi»: niente avviso nella barra, riquadro chiuso.
    pub piu_tardi: bool,
    /// Installazione riuscita, in attesa di riavvio.
    pub installato: Option<Installato>,
    /// Esito dell'ultima azione e suo colore.
    pub messaggio: Option<(String, Color32)>,
    /// «Riavvia ora»: alla chiusura parte la versione nuova.
    pub riavvio_richiesto: bool,
    /// Posizione della scheda nella pagina (per scorrere fin lì).
    pub y_scheda: f32,
    /// Scorre una volta fino alla scheda (clic sull'avviso della barra).
    pub scorri_alla_scheda: bool,
    /// Tiene la pagina ferma sulla scheda (collaudo).
    pub fissa_alla_scheda: bool,
    /// Posizione del riquadro della versione nuova nella pagina.
    pub y_riquadro: f32,
    /// Tiene la pagina ferma sul riquadro della versione nuova (collaudo).
    pub fissa_al_riquadro: bool,
    /// Bordo superiore del riquadro nell'ultimo fotogramma (coordinate dello schermo).
    riquadro_sullo_schermo: Option<f32>,
    /// Collaudo: mostra la scheda come in una copia installata, senza il
    /// blocco della build di sviluppo. Non cambia ciò che succede premendo.
    #[cfg(feature = "collaudo")]
    pub come_installata: bool,
}

impl StatoAggiornamentiUi {
    /// Legge `aggiornamenti.json` e avvia il thread (che non contatta la rete
    /// finché non glielo si chiede).
    pub fn carica(ctx: &egui::Context) -> Self {
        let (config, messaggio) =
            match ConfigAggiornamenti::carica(&config::cartella_configurazione()) {
                Ok(c) => (c, None),
                Err(e) => (
                    ConfigAggiornamenti::default(),
                    Some((
                        format!("Preferenze degli aggiornamenti non lette: {e}"),
                        ROSSO,
                    )),
                ),
            };
        let exe = std::env::current_exe().ok();
        let ctx = ctx.clone();
        let servizio = Servizio::avvia(ImpostazioniRete::github(), exe.clone(), move || {
            ctx.request_repaint()
        });
        StatoAggiornamentiUi {
            config,
            servizio,
            exe,
            avvio: Instant::now(),
            automatico_deciso: false,
            manuale: false,
            in_corso: None,
            rilascio: None,
            valutazione: None,
            piu_tardi: false,
            installato: None,
            messaggio,
            riavvio_richiesto: false,
            y_scheda: 0.0,
            scorri_alla_scheda: false,
            fissa_alla_scheda: false,
            y_riquadro: 0.0,
            fissa_al_riquadro: false,
            riquadro_sullo_schermo: None,
            #[cfg(feature = "collaudo")]
            come_installata: false,
        }
    }

    fn salva(&mut self) {
        if let Err(e) = self.config.salva(&config::cartella_configurazione()) {
            self.messaggio = Some((format!("Preferenza non salvata su disco: {e}"), ROSSO));
        }
    }

    /// Avviso compatto per la barra di stato: testo largo, testo breve, colore.
    pub fn testo_barra(&self) -> Option<(String, String, Color32)> {
        if let Some(i) = &self.installato {
            return Some((
                format!("Riavvia per usare la {}", i.versione),
                "Riavvia".into(),
                VERDE,
            ));
        }
        if self.piu_tardi || self.valutazione != Some(Valutazione::Disponibile) {
            return None;
        }
        let v = self.rilascio.as_ref()?.versione_testo();
        Some((format!("Aggiornamento {v}"), format!("v{v}"), tema::ACCENTO))
    }

    /// Chiede un controllo.
    fn avvia_controllo(&mut self, manuale: bool) {
        let ora = rete::adesso();
        if let Some(t) = self.config.limitato(ora) {
            self.messaggio = Some((
                ErroreAggiornamento::LimiteRichieste {
                    riprova_alle: Some(t),
                }
                .messaggio(),
                GIALLO,
            ));
            return;
        }
        self.manuale = manuale;
        self.in_corso = Some(InCorso::Controllo);
        if manuale {
            self.messaggio = None;
            self.piu_tardi = false;
        }
        self.config.ultimo_controllo = Some(ora);
        self.salva();
        self.servizio.controlla();
    }

    /// Smaltisce le notizie del thread e decide il controllo automatico.
    pub fn passo(&mut self, ctx: &egui::Context) {
        while let Some(n) = self.servizio.ricevi() {
            self.notizia(n);
        }
        if self.automatico_deciso {
            return;
        }
        if !piattaforma::CONTROLLO_AUTOMATICO_CONSENTITO || !self.config.automatico {
            self.automatico_deciso = true;
            return;
        }
        let trascorso = self.avvio.elapsed();
        if trascorso < RITARDO_AVVIO {
            ctx.request_repaint_after(RITARDO_AVVIO - trascorso);
            return;
        }
        self.automatico_deciso = true;
        if self.in_corso.is_none() && self.config.controllo_dovuto(rete::adesso()) {
            self.avvia_controllo(false);
        }
    }

    fn notizia(&mut self, n: Notizia) {
        match n {
            Notizia::Controllo { esito, quando } => {
                self.in_corso = None;
                self.config.ultimo_controllo = Some(quando);
                match esito {
                    Ok(r) => self.rilascio_trovato(r),
                    Err(e) => {
                        let breve = match &e {
                            ErroreAggiornamento::NessunaVersione => {
                                "nessuna versione pubblicata".to_string()
                            }
                            ErroreAggiornamento::LimiteRichieste { riprova_alle } => {
                                self.config.non_prima_di =
                                    Some(riprova_alle.unwrap_or(quando + 3600));
                                "GitHub ha limitato le richieste".into()
                            }
                            ErroreAggiornamento::NonRaggiungibile(_)
                            | ErroreAggiornamento::TempoScaduto => {
                                "GitHub non raggiungibile".into()
                            }
                            _ => "controllo non riuscito".into(),
                        };
                        self.config.ultimo_esito = Some(breve);
                        // Un controllo automatico fallito non disturba: resta
                        // scritto nella scheda. Quello manuale lo dice subito.
                        let colore = match e {
                            ErroreAggiornamento::NessunaVersione => tema::TESTO_2,
                            ErroreAggiornamento::NonRaggiungibile(_)
                            | ErroreAggiornamento::TempoScaduto
                            | ErroreAggiornamento::LimiteRichieste { .. } => GIALLO,
                            _ => ROSSO,
                        };
                        self.messaggio = Some((e.messaggio(), colore));
                    }
                }
                self.salva();
            }
            Notizia::Progresso { scaricati, totale } => {
                self.in_corso = Some(InCorso::Scaricamento { scaricati, totale });
            }
            Notizia::Installazione(esito) => {
                self.in_corso = None;
                match esito {
                    Ok(i) => {
                        self.messaggio = None;
                        self.installato = Some(i);
                    }
                    Err(e) => {
                        let colore = if e == ErroreAggiornamento::Annullato {
                            tema::TESTO_2
                        } else {
                            ROSSO
                        };
                        self.messaggio = Some((e.messaggio(), colore));
                    }
                }
            }
        }
    }

    fn rilascio_trovato(&mut self, r: Box<Rilascio>) {
        let installata = Versione::corrente();
        let v = aggiornamenti::valuta(&installata, &r, &self.config);
        let testo = r.versione_testo();
        self.config.ultima_versione_vista = Some(testo.clone());
        self.config.non_prima_di = None;
        self.config.ultimo_esito = Some(match v {
            Valutazione::Disponibile => format!("disponibile la versione {testo}"),
            Valutazione::NessunaNovita => format!("nessuna versione nuova (ultima: {testo})"),
            Valutazione::Ignorata => format!("versione {testo} ignorata su tua richiesta"),
            Valutazione::NonProposta => "nessuna versione stabile nuova".into(),
            Valutazione::Illeggibile => {
                format!("etichetta «{}» non riconosciuta: ignorata", r.etichetta)
            }
        });
        if self.manuale {
            self.messaggio = match v {
                Valutazione::NessunaNovita | Valutazione::NonProposta => Some((
                    format!("Hai già la versione più recente ({installata})."),
                    VERDE,
                )),
                Valutazione::Illeggibile => Some((
                    format!(
                        "L'ultima versione su GitHub ha un'etichetta non riconosciuta («{}»): \
                         non viene proposta. Controlla a mano la pagina delle versioni.",
                        r.etichetta
                    ),
                    GIALLO,
                )),
                _ => None,
            };
        }
        let mostra = v == Valutazione::Disponibile || (v == Valutazione::Ignorata && self.manuale);
        self.valutazione = Some(v);
        self.rilascio = mostra.then_some(r);
    }

    /// Lancia la versione nuova se l'utente ha chiesto «Riavvia ora». Va
    /// chiamata alla chiusura, dopo che il thread di lavoro ha chiuso
    /// registrazione e collegamenti.
    pub fn riavvia_se_richiesto(&self) {
        if !self.riavvio_richiesto {
            return;
        }
        let Some(i) = &self.installato else {
            return;
        };
        let mut c = std::process::Command::new(&i.eseguibile);
        c.args(std::env::args_os().skip(1));
        if let Ok(d) = std::env::current_dir() {
            c.current_dir(d);
        }
        if let Err(e) = c.spawn() {
            eprintln!("aggiornamenti: riavvio non riuscito: {e}");
        }
    }
}

/// Data leggibile in italiano: «1 dicembre 2026».
fn data_italiana(iso: &str) -> String {
    const MESI: [&str; 12] = [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ];
    use chrono::Datelike;
    match chrono::DateTime::parse_from_rfc3339(iso.trim()) {
        Ok(d) => {
            let d = d.with_timezone(&chrono::Local);
            format!("{} {} {}", d.day(), MESI[d.month0() as usize], d.year())
        }
        Err(_) => iso.trim().to_string(),
    }
}

/// Momento in secondi Unix come «22/11/2026 alle 14:05», ora locale.
fn momento(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d/%m/%Y alle %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "data sconosciuta".into())
}

/// Ciò che l'utente ha chiesto in questo fotogramma.
enum Gesto {
    Automatico(bool),
    ControllaOra,
    Installa,
    ApriPagina,
    Ignora,
    PiuTardi,
    Mostra,
    Annulla,
    RiavviaOra,
    AlProssimoAvvio,
}

impl App {
    /// Perché l'installazione non si può fare adesso, se non si può.
    fn motivo_niente_installazione(&self, r: &Rilascio) -> Option<String> {
        let a = &self.aggiornamenti;
        let Some(piattaforma) = rilascio::piattaforma_corrente() else {
            return Some(
                "L'installazione con un clic esiste solo per Windows a 64 bit: scarica la \
                 versione dalla sua pagina."
                    .into(),
            );
        };
        #[cfg(feature = "collaudo")]
        let salta_blocco = a.come_installata;
        #[cfg(not(feature = "collaudo"))]
        let salta_blocco = false;
        if !salta_blocco {
            match &a.exe {
                None => return Some("Percorso dell'eseguibile sconosciuto.".into()),
                Some(e) => {
                    if let Some(m) = sostituzione::blocco_sviluppo_corrente(e) {
                        return Some(m);
                    }
                }
            }
        }
        if r.eseguibile(piattaforma).is_none() {
            return Some(
                ErroreAggiornamento::RisorsaMancante(
                    r.versione
                        .as_ref()
                        .map(|v| rilascio::nome_eseguibile(v, piattaforma))
                        .unwrap_or_else(|| r.etichetta.clone()),
                )
                .messaggio(),
            );
        }
        if self.registrazione.is_some() {
            return Some("Ferma la registrazione prima di installare l'aggiornamento.".into());
        }
        if self.stato == StatoConn::Cancellazione || self.conferma_cancellazione.is_some() {
            return Some("Attendi la fine della cancellazione dei codici guasto.".into());
        }
        None
    }

    /// Smaltisce le notizie del thread degli aggiornamenti.
    pub(super) fn passo_aggiornamenti(&mut self, ctx: &egui::Context) {
        self.aggiornamenti.passo(ctx);
    }

    /// La scheda intera; `origine` è il bordo superiore del contenuto della
    /// pagina, per ricordare dove stanno scheda e riquadro.
    pub(super) fn scheda_aggiornamenti(&mut self, ui: &mut egui::Ui, stretto: bool, origine: f32) {
        let mut gesto: Option<Gesto> = None;
        self.aggiornamenti.riquadro_sullo_schermo = None;
        let r = componenti::scheda_titolata(ui, "Aggiornamenti", |ui| {
            self.contenuto_aggiornamenti(ui, stretto, &mut gesto);
        });
        let a = &mut self.aggiornamenti;
        a.y_scheda = (r.response.rect.top() - origine).max(1.0);
        if let Some(y) = a.riquadro_sullo_schermo {
            a.y_riquadro = (y - origine).max(1.0);
        }
        if let Some(g) = gesto {
            self.gesto_aggiornamenti(ui.ctx(), g);
        }
    }

    fn contenuto_aggiornamenti(
        &mut self,
        ui: &mut egui::Ui,
        stretto: bool,
        gesto: &mut Option<Gesto>,
    ) {
        paragrafo(
            ui,
            "OBDio può controllare se su GitHub, dove è pubblicata, c'è una versione nuova. Il \
             controllo contatta solo GitHub (api.github.com) e chiede qual è l'ultima versione: \
             non invia nulla su di te, sul veicolo o sulle sessioni. GitHub vede soltanto ciò \
             che vede ogni sito, cioè l'indirizzo IP e il nome «OBDio» con la versione. Nulla \
             viene scaricato senza che tu lo chieda.",
        );
        ui.add_space(tema::SP_3);

        let a = &self.aggiornamenti;
        let acceso = a.config.automatico;
        let etichetta = if acceso {
            "Controllo automatico attivo"
        } else {
            "Controllo automatico spento"
        };
        if interruttore(ui, acceso, etichetta)
            .on_hover_text(if acceso {
                "Spegni il controllo: OBDio non contatterà più GitHub da sola"
            } else {
                "Controlla da sola, al più una volta al giorno"
            })
            .clicked()
        {
            *gesto = Some(Gesto::Automatico(!acceso));
        }
        nota(
            ui,
            if acceso {
                "Al più una volta ogni 24 ore, qualche secondo dopo l'avvio. Spento, OBDio non \
                 contatta GitHub se non premi «Controlla ora»."
            } else {
                "OBDio non contatta GitHub da sola: controlla solo quando premi «Controlla ora»."
            },
        );
        ui.add_space(tema::SP_2);

        riga_valore(
            ui,
            stretto,
            "Versione installata",
            RichText::new(env!("CARGO_PKG_VERSION")),
        );
        let ultimo = match (a.config.ultimo_controllo, &a.config.ultimo_esito) {
            (None, _) => "mai".to_string(),
            (Some(t), Some(e)) => format!("{}: {e}", momento(t)),
            (Some(t), None) => momento(t),
        };
        riga_valore(ui, stretto, "Ultimo controllo", RichText::new(ultimo));
        ui.add_space(tema::SP_2);

        let controllo = a.in_corso == Some(InCorso::Controllo);
        let occupato = a.in_corso.is_some();
        if stretto {
            if componenti::pulsante(ui, true, !occupato, egui::Button::new("Controlla ora"))
                .clicked()
            {
                *gesto = Some(Gesto::ControllaOra);
            }
            if controllo {
                componenti::riga_testo(ui, |ui| {
                    ui.spinner();
                    ui.label(RichText::new("Controllo in corso…").color(tema::TESTO_2));
                });
            }
        } else {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!occupato, egui::Button::new("Controlla ora"))
                    .clicked()
                {
                    *gesto = Some(Gesto::ControllaOra);
                }
                if controllo {
                    ui.spinner();
                    ui.label(RichText::new("Controllo in corso…").color(tema::TESTO_2));
                }
            });
        }
        if let Some((testo, colore)) = &a.messaggio {
            ui.add(egui::Label::new(RichText::new(testo).color(*colore)).wrap());
        }

        if let Some(i) = a.installato.clone() {
            ui.add_space(tema::SP_3);
            self.aggiornamenti.riquadro_sullo_schermo = Some(ui.cursor().top());
            self.riquadro_installato(ui, stretto, &i, gesto);
        } else if let Some(r) = a.rilascio.clone() {
            ui.add_space(tema::SP_3);
            if a.piu_tardi {
                componenti::riga_testo(ui, |ui| {
                    ui.label(
                        RichText::new(format!("Versione {} disponibile.", r.versione_testo()))
                            .color(tema::TESTO_2),
                    );
                });
                if componenti::pulsante(ui, stretto, true, egui::Button::new("Mostra")).clicked() {
                    *gesto = Some(Gesto::Mostra);
                }
            } else {
                self.aggiornamenti.riquadro_sullo_schermo = Some(ui.cursor().top());
                self.riquadro_versione(ui, stretto, &r, gesto);
            }
        }
        ui.add_space(tema::SP_2);
        nota(
            ui,
            "Come funziona, che cosa viene verificato e come aggiornare a mano: \
             docs/AGGIORNAMENTI.md.",
        );
    }

    /// Riquadro della versione nuova: note, pulsanti, avanzamento.
    fn riquadro_versione(
        &self,
        ui: &mut egui::Ui,
        stretto: bool,
        r: &Rilascio,
        gesto: &mut Option<Gesto>,
    ) {
        let motivo = self.motivo_niente_installazione(r);
        let a = &self.aggiornamenti;
        let ignorata = a.valutazione == Some(Valutazione::Ignorata);
        cornice_riquadro().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("Disponibile la versione {}", r.versione_testo()))
                    .strong()
                    .color(tema::ACCENTO),
            );
            // Il carattere di egui non ha la freccia: si scrive a parole.
            let mut riga = format!("Da {} a {}", env!("CARGO_PKG_VERSION"), r.versione_testo());
            if let Some(p) = &r.pubblicato {
                riga.push_str(&format!(" · pubblicata il {}", data_italiana(p)));
            }
            ui.label(RichText::new(riga).color(tema::TESTO));
            if !r.nome.trim().is_empty() && r.nome.trim() != r.etichetta {
                nota(ui, format!("«{}»", r.nome.trim()));
            }
            if ignorata {
                nota(
                    ui,
                    "Avevi scelto di ignorare questa versione: la vedi perché hai controllato a \
                     mano.",
                );
            }
            ui.add_space(tema::SP_2);
            ui.label(RichText::new("Note di rilascio").color(tema::TESTO_2));
            egui::Frame::new()
                .fill(tema::SUPERFICIE)
                .stroke(tema::tratto_bordo())
                .corner_radius(egui::CornerRadius::same(tema::RAGGIO_CONTROLLO))
                .inner_margin(egui::Margin::same(tema::SP_2 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let alto = if stretto { 200.0 } else { 160.0 };
                    egui::ScrollArea::vertical()
                        .id_salt("note_rilascio")
                        .max_height(alto)
                        .min_scrolled_height(alto)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let testo = if r.note.trim().is_empty() {
                                "Nessuna nota per questa versione.".to_string()
                            } else {
                                r.note.trim().to_string()
                            };
                            // Testo semplice: i collegamenti non sono cliccabili.
                            ui.add(
                                egui::Label::new(
                                    RichText::new(testo)
                                        .size(tema::CARATTERE_NOTA)
                                        .color(tema::TESTO),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        });
                });
            nota(
                ui,
                "Testo come scritto su GitHub; i collegamenti non si aprono da qui.",
            );
            ui.add_space(tema::SP_3);

            if let Some(InCorso::Scaricamento { scaricati, totale }) = a.in_corso {
                let frazione = if totale > 0 {
                    scaricati as f32 / totale as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(frazione.clamp(0.0, 1.0))
                        .desired_height(if stretto { 12.0 } else { 8.0 })
                        .fill(tema::ACCENTO),
                );
                ui.label(
                    RichText::new(format!(
                        "Scaricati {} di {}",
                        aggiornamenti::byte_leggibili(scaricati),
                        aggiornamenti::byte_leggibili(totale)
                    ))
                    .color(tema::TESTO_2),
                );
                if componenti::pulsante(ui, stretto, true, egui::Button::new("Annulla")).clicked() {
                    *gesto = Some(Gesto::Annulla);
                }
            } else {
                let occupato = a.in_corso.is_some();
                let mut pulsanti = |ui: &mut egui::Ui| {
                    if componenti::pulsante(
                        ui,
                        stretto,
                        motivo.is_none() && !occupato,
                        componenti::primario("Scarica e installa"),
                    )
                    .clicked()
                    {
                        *gesto = Some(Gesto::Installa);
                    }
                    if componenti::pulsante(
                        ui,
                        stretto,
                        true,
                        egui::Button::new("Apri la pagina della versione"),
                    )
                    .on_hover_text(r.pagina.as_str())
                    .clicked()
                    {
                        *gesto = Some(Gesto::ApriPagina);
                    }
                    if !ignorata
                        && componenti::pulsante(
                            ui,
                            stretto,
                            true,
                            egui::Button::new("Ignora questa versione"),
                        )
                        .clicked()
                    {
                        *gesto = Some(Gesto::Ignora);
                    }
                    if componenti::pulsante(ui, stretto, true, egui::Button::new("Più tardi"))
                        .clicked()
                    {
                        *gesto = Some(Gesto::PiuTardi);
                    }
                };
                if stretto {
                    pulsanti(ui);
                } else {
                    ui.horizontal_wrapped(|ui| pulsanti(ui));
                }
                if let Some(m) = &motivo {
                    ui.add(egui::Label::new(RichText::new(m).color(GIALLO)).wrap());
                }
            }
            ui.add_space(tema::SP_2);
            nota(
                ui,
                "Prima di installare OBDio controlla che il file abbia la dimensione dichiarata \
                 da GitHub e la somma SHA-256 pubblicata nello stesso rilascio: così si accorge \
                 di un file danneggiato o alterato durante lo scaricamento, non di un rilascio \
                 pubblicato da chi avesse preso il controllo dell'account o del repository. \
                 L'eseguibile non è firmato: al primo avvio Windows SmartScreen potrebbe \
                 avvisare. Sessioni e configurazione non vengono toccate.",
            );
        });
    }

    /// Riquadro dopo un'installazione riuscita.
    fn riquadro_installato(
        &self,
        ui: &mut egui::Ui,
        stretto: bool,
        i: &Installato,
        gesto: &mut Option<Gesto>,
    ) {
        let occupato = self.registrazione.is_some() || self.stato == StatoConn::Cancellazione;
        cornice_riquadro().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("Versione {} installata", i.versione))
                    .strong()
                    .color(VERDE),
            );
            paragrafo(
                ui,
                format!(
                    "Parte al prossimo avvio di OBDio. Fino ad allora resta in uso la {}; la \
                     copia precedente ({}) viene eliminata al prossimo avvio.",
                    env!("CARGO_PKG_VERSION"),
                    i.vecchio
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ),
            );
            ui.add_space(tema::SP_2);
            let mut pulsanti = |ui: &mut egui::Ui| {
                if componenti::pulsante(ui, stretto, !occupato, componenti::primario("Riavvia ora"))
                    .clicked()
                {
                    *gesto = Some(Gesto::RiavviaOra);
                }
                if componenti::pulsante(ui, stretto, true, egui::Button::new("Al prossimo avvio"))
                    .clicked()
                {
                    *gesto = Some(Gesto::AlProssimoAvvio);
                }
            };
            if stretto {
                pulsanti(ui);
            } else {
                ui.horizontal(|ui| pulsanti(ui));
            }
            if occupato {
                nota(
                    ui,
                    "Riavvia dopo aver fermato la registrazione o finita la cancellazione dei \
                     codici.",
                );
            } else {
                nota(
                    ui,
                    "«Riavvia ora» chiude OBDio come quando la chiudi tu (registrazione e \
                     impostazioni vengono salvate) e apre la versione nuova.",
                );
            }
        });
    }

    fn gesto_aggiornamenti(&mut self, ctx: &egui::Context, g: Gesto) {
        match g {
            Gesto::Automatico(acceso) => {
                let a = &mut self.aggiornamenti;
                a.config.automatico = acceso;
                a.messaggio = None;
                a.salva();
            }
            Gesto::ControllaOra => self.aggiornamenti.avvia_controllo(true),
            Gesto::Installa => {
                let Some(r) = self.aggiornamenti.rilascio.clone() else {
                    return;
                };
                if let Some(m) = self.motivo_niente_installazione(&r) {
                    self.aggiornamenti.messaggio = Some((m, GIALLO));
                    return;
                }
                let a = &mut self.aggiornamenti;
                let totale = rilascio::piattaforma_corrente()
                    .and_then(|p| r.eseguibile(p))
                    .map(|x| x.dimensione)
                    .unwrap_or(0);
                a.messaggio = None;
                a.in_corso = Some(InCorso::Scaricamento {
                    scaricati: 0,
                    totale,
                });
                a.servizio.installa(*r);
            }
            Gesto::ApriPagina => {
                let a = &mut self.aggiornamenti;
                let Some(r) = &a.rilascio else {
                    return;
                };
                let ammessa =
                    rete::controlla_indirizzo(&r.pagina, &["github.com".to_string()], false)
                        .is_ok();
                if !ammessa || !piattaforma::apri_indirizzo(&r.pagina) {
                    a.messaggio = Some((
                        format!(
                            "La pagina non si è aperta. Indirizzo: {}",
                            if ammessa {
                                r.pagina.as_str()
                            } else {
                                "https://github.com/inalto/OBDio/releases"
                            }
                        ),
                        GIALLO,
                    ));
                }
            }
            Gesto::Ignora => {
                let a = &mut self.aggiornamenti;
                if let Some(r) = a.rilascio.take() {
                    let v = r.versione_testo();
                    a.config.ignora_versione = Some(v.clone());
                    a.config.ultimo_esito = Some(format!("versione {v} ignorata su tua richiesta"));
                    a.valutazione = Some(Valutazione::Ignorata);
                    a.messaggio = Some((
                        format!(
                            "La versione {v} non verrà più proposta; una più nuova sì. «Controlla \
                             ora» la mostra comunque."
                        ),
                        tema::TESTO_2,
                    ));
                    a.salva();
                }
            }
            Gesto::PiuTardi => self.aggiornamenti.piu_tardi = true,
            Gesto::Mostra => self.aggiornamenti.piu_tardi = false,
            Gesto::Annulla => self.aggiornamenti.servizio.annulla(),
            Gesto::RiavviaOra => {
                self.aggiornamenti.riavvio_richiesto = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Gesto::AlProssimoAvvio => {
                let a = &mut self.aggiornamenti;
                a.riavvio_richiesto = false;
                if let Some(i) = &a.installato {
                    a.messaggio = Some((
                        format!(
                            "La versione {} partirà la prossima volta che apri OBDio.",
                            i.versione
                        ),
                        tema::TESTO_2,
                    ));
                }
            }
        }
    }
}

/// Cornice del riquadro della versione nuova, dentro la scheda.
fn cornice_riquadro() -> egui::Frame {
    egui::Frame::new()
        .fill(tema::SFONDO)
        .stroke(egui::Stroke::new(
            tema::BORDO_SOTTILE,
            tema::velatura(tema::ACCENTO, 0.5),
        ))
        .corner_radius(egui::CornerRadius::same(tema::RAGGIO_CONTROLLO))
        .inner_margin(egui::Margin::same(tema::SP_3 as i8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_in_italiano() {
        // Mezzogiorno UTC: lo stesso giorno in ogni fuso europeo.
        assert_eq!(data_italiana("2026-12-01T12:00:00Z"), "1 dicembre 2026");
        assert_eq!(data_italiana("non una data"), "non una data");
        assert!(momento(1_795_000_000).contains(" alle "));
    }
}
