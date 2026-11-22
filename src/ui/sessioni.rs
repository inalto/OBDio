//! Scheda Sessioni: sessioni salvate, importazione (formato riconosciuto dal
//! contenuto, con anteprima prima di salvare), esportazione nei quattro
//! formati e «Rivivi questa sessione».

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Color32, RichText, ScrollArea, Sense, vec2};

use super::componenti;
use super::grafici::{self, Opzioni};
use super::icone::{self, Icona};
use super::navigazione::{altezza_modale, larghezza_finestra};
use super::pagine::{disegna_diagnosi, nome_file, selettore_extra};
use super::scelta_file::{EsitoScelta, SceltaFile, cartella_documenti};
use super::{App, SessioneAperta, apri_cartella, tema};
use crate::model::num_it;
use crate::obd::pids;
use crate::recording;
use crate::riproduzione::{data_leggibile, mm_ss};
use crate::scambio::{self, FormatoEsportazione, Importazione, Riepilogo};
use crate::summary;
use crate::worker::{Comando, StatoConn};

/// Che cosa fare con il file scelto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AzioneSessioni {
    Importa,
    Esporta(FormatoEsportazione),
    /// Scegli una sessione e rivivila (dalla scheda Connessione).
    Riproduci,
}

/// Esito di un'operazione sui file, con la cartella da aprire.
#[derive(Clone, Debug)]
pub(crate) struct EsitoFile {
    pub testo: String,
    pub colore: Color32,
    pub cartella: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct StatoSessioni {
    pub dialogo: Option<(AzioneSessioni, SceltaFile)>,
    /// Lettura in corso del file da importare.
    pub lettura: Option<Receiver<Result<Importazione, String>>>,
    /// Importazione letta, in attesa della conferma.
    pub anteprima: Option<Importazione>,
    pub esito: Option<EsitoFile>,
    /// Appena caricata, la sessione va rivissuta.
    pub riproduci_dopo: bool,
    /// Sul layout stretto: si vede il dettaglio della sessione invece
    /// dell'elenco.
    pub dettaglio: bool,
}

fn finali(f: FormatoEsportazione) -> &'static [&'static str] {
    match f {
        FormatoEsportazione::Sessione => &[".obdio.jsonl"],
        FormatoEsportazione::CsvObdio | FormatoEsportazione::CsvTorque => &[".csv"],
        FormatoEsportazione::DiagnosiJson => &[".json"],
    }
}

/// Riepilogo dell'importazione, mostrato nell'anteprima e sulla sessione.
pub(crate) fn disegna_riepilogo(ui: &mut egui::Ui, r: &Riepilogo) {
    let riga = |ui: &mut egui::Ui, nome: &str, valore: RichText| {
        componenti::testo_composto(
            ui,
            vec![
                RichText::new(format!("{nome}: ")).color(tema::TESTO_2),
                valore,
            ],
        );
    };
    riga(
        ui,
        "Formato riconosciuto",
        RichText::new(r.formato.nome()).strong(),
    );
    riga(ui, "File", RichText::new(&r.file));
    riga(
        ui,
        "Contenuto",
        RichText::new(format!(
            "{} campioni, {} marcatori, durata {}",
            num_it(r.campioni as f64, 0),
            r.marcatori,
            mm_ss(r.durata_s)
        )),
    );
    let elenco = |ui: &mut egui::Ui, titolo: &str, voci: Vec<String>| {
        ui.add_space(tema::SP_1);
        ui.label(RichText::new(format!("{titolo}: {}", voci.len())).color(tema::TESTO_2));
        if voci.is_empty() {
            return;
        }
        ui.add(egui::Label::new(RichText::new(voci.join(" · "))).wrap());
    };
    elenco(
        ui,
        "Parametri riconosciuti",
        r.riconosciuti
            .iter()
            .map(|c| {
                let nome = pids::per_key(&c.chiave).map_or(c.chiave.clone(), |p| p.nome.clone());
                match &c.conversione {
                    Some(conv) => format!("{nome} ({conv})"),
                    None => nome,
                }
            })
            .collect(),
    );
    elenco(
        ui,
        "Conservati come parametri liberi",
        r.liberi
            .iter()
            .map(|c| {
                if c.unita.is_empty() {
                    c.colonna.clone()
                } else {
                    format!("{} [{}]", c.colonna, c.unita)
                }
            })
            .collect(),
    );
    ui.add_space(tema::SP_1);
    riga(
        ui,
        "Righe saltate",
        RichText::new(format!(
            "{} (ora o stato illeggibili, riga incompleta)",
            r.righe_scartate
        ))
        .color(if r.righe_scartate > 0 {
            tema::ATTENZIONE
        } else {
            tema::TESTO
        }),
    );
    if r.righe_irregolari > 0 {
        riga(
            ui,
            "Righe irregolari",
            RichText::new(format!(
                "{}, con un numero di colonne diverso dall'intestazione (importate per quanto \
                 leggibili)",
                r.righe_irregolari
            )),
        );
    }
    if r.intestazioni_ripetute > 0 {
        riga(
            ui,
            "Intestazioni ripetute",
            RichText::new(format!(
                "{} (riconosciute e saltate)",
                r.intestazioni_ripetute
            )),
        );
    }
    if r.valori_non_numerici > 0 {
        riga(
            ui,
            "Valori non numerici",
            RichText::new(format!(
                "{}, importati come «errore di comunicazione» senza valore",
                r.valori_non_numerici
            ))
            .color(tema::ATTENZIONE),
        );
    }
    for a in &r.avvisi {
        ui.add(egui::Label::new(RichText::new(a).small().color(tema::ATTENZIONE)).wrap());
    }
}

impl App {
    // ------------------------------------------------------------------
    // Azioni (usate anche dal collaudo)
    // ------------------------------------------------------------------

    /// Apre la finestra di scelta del file per l'azione.
    pub(crate) fn apri_scelta_sessioni(&mut self, azione: AzioneSessioni) {
        let scelta = match azione {
            AzioneSessioni::Importa => SceltaFile::new(
                "Importa una sessione",
                "Leggi il file",
                &cartella_documenti(),
                "",
            )
            .spiegazione(
                "Sessione OBDio (.obdio.jsonl), CSV esportato da OBDio o CSV in stile Torque \
                 (trackLog.csv). Il formato si riconosce dal contenuto; prima di salvare vedi \
                 che cosa è stato letto.",
            )
            .finali(&[".obdio.jsonl", ".csv", ".txt"]),
            AzioneSessioni::Esporta(f) => {
                let Some(s) = &self.aperta else {
                    return;
                };
                SceltaFile::new(
                    &format!("Esporta: {}", f.nome()),
                    "Esporta",
                    &cartella_documenti(),
                    &scambio::nome_proposto(&s.dati, f),
                )
                .spiegazione("Scrivi dove salvare il file. Se esiste già, viene sostituito.")
                .finali(finali(f))
            }
            AzioneSessioni::Riproduci => SceltaFile::new(
                "Riproduci una sessione",
                "Riproduci",
                &self.cartella,
                "",
            )
            .spiegazione(
                "Scegli una sessione salvata (anche importata): i suoi dati scorrono nel tempo \
                 su cruscotto, sensori, grafici e MQTT come se arrivassero dall'auto.",
            )
            .finali(&[recording::ESTENSIONE]),
        };
        self.sessioni.dialogo = Some((azione, scelta));
    }

    /// Esporta la sessione aperta e annota l'esito.
    pub(crate) fn esporta_sessione(&mut self, f: FormatoEsportazione, percorso: &Path) {
        let Some(s) = &self.aperta else {
            return;
        };
        self.sessioni.esito = Some(match scambio::esporta(&s.dati, f, percorso) {
            Ok(p) => EsitoFile {
                testo: format!("{} scritto in «{}».", f.nome(), p.display()),
                colore: tema::OK,
                cartella: p.parent().map(Path::to_path_buf),
            },
            Err(e) => EsitoFile {
                testo: format!("Esportazione non riuscita: {e}."),
                colore: tema::PERICOLO,
                cartella: None,
            },
        });
    }

    /// Legge il file da importare in un altro thread.
    pub(crate) fn importa_da(&mut self, percorso: PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(scambio::importa_file(&percorso));
        });
        self.sessioni.lettura = Some(rx);
        self.sessioni.anteprima = None;
        self.sessioni.esito = None;
    }

    pub(crate) fn ricevi_importazione(&mut self) {
        let Some(rx) = &self.sessioni.lettura else {
            return;
        };
        let Ok(esito) = rx.try_recv() else {
            return;
        };
        self.sessioni.lettura = None;
        match esito {
            Ok(imp) => self.sessioni.anteprima = Some(imp),
            Err(e) => {
                self.sessioni.esito = Some(EsitoFile {
                    testo: format!("Importazione non riuscita: {e}."),
                    colore: tema::PERICOLO,
                    cartella: None,
                })
            }
        }
    }

    /// Salva l'importazione in anteprima come sessione OBDio e la apre.
    pub(crate) fn conferma_importazione(&mut self) {
        let Some(imp) = self.sessioni.anteprima.take() else {
            return;
        };
        match scambio::salva_importata(&self.cartella, &imp) {
            Ok(s) => {
                self.sessioni.esito = Some(EsitoFile {
                    testo: format!("Sessione importata salvata in «{}».", s.percorso.display()),
                    colore: tema::OK,
                    cartella: Some(self.cartella.clone()),
                });
                self.elenco = recording::elenca(&self.cartella);
                self.errore_sessione = None;
                self.aperta = Some(SessioneAperta::new(s, Some(imp.riepilogo)));
            }
            Err(e) => {
                self.sessioni.esito = Some(EsitoFile {
                    testo: format!("Sessione importata non salvata: {e}."),
                    colore: tema::PERICOLO,
                    cartella: None,
                });
            }
        }
    }

    /// Finestre della scheda (scelta del file, anteprima dell'importazione),
    /// mostrate sopra a qualunque scheda.
    pub(crate) fn finestre_sessioni(&mut self, ctx: &egui::Context) {
        if let Some((azione, scelta)) = &mut self.sessioni.dialogo {
            let azione = *azione;
            match scelta.mostra(ctx, "sessioni") {
                EsitoScelta::Aperta => {}
                EsitoScelta::Annulla => self.sessioni.dialogo = None,
                EsitoScelta::Conferma(p) => {
                    self.sessioni.dialogo = None;
                    match azione {
                        AzioneSessioni::Importa => self.importa_da(p),
                        AzioneSessioni::Esporta(f) => self.esporta_sessione(f, &p),
                        AzioneSessioni::Riproduci => {
                            self.sessioni.riproduci_dopo = true;
                            self.apri_sessione(p);
                        }
                    }
                }
            }
        }
        let Some(imp) = &self.sessioni.anteprima else {
            return;
        };
        let stretto = self.disposizione.stretta();
        let mut salva = false;
        let mut annulla = false;
        let largo = larghezza_finestra(ctx, 720.0);
        let alto = altezza_modale(ctx, if stretto { 290.0 } else { 200.0 });
        egui::Modal::new(egui::Id::new("anteprima_importazione"))
            .frame(componenti::cornice_finestra(ctx))
            .backdrop_color(egui::Color32::from_black_alpha(170))
            .show(ctx, |ui| {
                ui.set_width(largo);
                ui.label(RichText::new("Importazione: che cosa è stato letto").heading());
                componenti::paragrafo(
                    ui,
                    "Nulla è ancora stato salvato. «Salva e apri» crea una nuova sessione OBDio \
                 nella cartella delle sessioni; il file d'origine non viene modificato.",
                );
                ui.add_space(tema::SP_2);
                ScrollArea::vertical()
                    .id_salt("anteprima_importazione_testo")
                    .max_height(alto)
                    .show(ui, |ui| disegna_riepilogo(ui, &imp.riepilogo));
                ui.add_space(tema::SP_3);
                let mut pulsanti = |ui: &mut egui::Ui| {
                    if componenti::pulsante(ui, stretto, true, componenti::primario("Salva e apri"))
                        .clicked()
                    {
                        salva = true;
                    }
                    if componenti::pulsante(ui, stretto, true, egui::Button::new("Annulla"))
                        .clicked()
                    {
                        annulla = true;
                    }
                };
                if stretto {
                    pulsanti(ui);
                } else {
                    ui.horizontal(|ui| pulsanti(ui));
                }
            });
        if salva {
            self.conferma_importazione();
            self.sessioni.dettaglio = true;
            self.scheda = super::Scheda::Sessioni;
        } else if annulla || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.sessioni.anteprima = None;
        }
    }

    // ------------------------------------------------------------------
    // Pagina
    // ------------------------------------------------------------------

    pub(crate) fn pagina_sessioni(&mut self, ui: &mut egui::Ui) {
        if self.disposizione.stretta() {
            // Due passi: elenco → dettaglio, con «indietro».
            let dettaglio = self.sessioni.dettaglio
                && (self.aperta.is_some()
                    || self.caricamento.is_some()
                    || self.errore_sessione.is_some());
            if dettaglio {
                if riga_indietro(ui, "Tutte le sessioni") {
                    self.sessioni.dettaglio = false;
                }
                ui.add_space(tema::SP_1);
                ScrollArea::vertical()
                    .id_salt("sessione_dettaglio")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.sessioni_destra(ui, true);
                        ui.add_space(tema::SP_6);
                    });
            } else {
                self.elenco_sessioni(ui, true);
            }
            return;
        }
        egui::Panel::left("sessioni_sinistra")
            .resizable(false)
            .exact_size(340.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 0,
                right: tema::SP_4 as i8,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| self.elenco_sessioni(ui, false));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: tema::SP_2 as i8,
                right: 0,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt("sessione_dettaglio")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.sessioni_destra(ui, false);
                        ui.add_space(tema::SP_4);
                    });
            });
    }

    /// Elenco delle sessioni salvate con i comandi della cartella.
    fn elenco_sessioni(&mut self, ui: &mut egui::Ui, stretto: bool) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(componenti::primario("Importa…"))
                .on_hover_text("Sessione OBDio, CSV OBDio o CSV in stile Torque")
                .clicked()
            {
                self.apri_scelta_sessioni(AzioneSessioni::Importa);
            }
            if ui.button("Aggiorna").clicked() {
                self.elenco = recording::elenca(&self.cartella);
            }
            if ui.button("Apri la cartella").clicked() {
                apri_cartella(&self.cartella);
            }
        });
        componenti::nota(ui, self.cartella.display().to_string());
        if self.sessioni.lettura.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Lettura del file da importare…");
            });
        }
        if stretto {
            self.riga_esito(ui);
        }
        ui.add_space(tema::SP_2);
        if self.elenco.is_empty() {
            componenti::stato_vuoto(
                ui,
                "Nessuna sessione salvata",
                "Le sessioni si registrano dalla pagina Sensori («Registra») oppure si \
                 importano da un file con «Importa…».",
                None,
            );
            return;
        }
        ScrollArea::vertical()
            .id_salt("sessioni_elenco")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for p in self.elenco.clone() {
                    let scelta = self.aperta.as_ref().is_some_and(|s| s.dati.percorso == p);
                    if riga_sessione(ui, &nome_file(&p), scelta && !stretto) {
                        self.apri_sessione(p);
                        self.sessioni.dettaglio = true;
                    }
                }
                ui.add_space(tema::SP_4);
            });
    }

    fn riga_esito(&mut self, ui: &mut egui::Ui) {
        let Some(e) = self.sessioni.esito.clone() else {
            return;
        };
        componenti::riga_testo(ui, |ui| {
            ui.label(RichText::new(&e.testo).color(e.colore));
            if let Some(c) = &e.cartella
                && ui.small_button("Apri la cartella").clicked()
            {
                apri_cartella(c);
            }
            if ui.small_button("Chiudi").clicked() {
                self.sessioni.esito = None;
            }
        });
    }

    /// Pulsante «Rivivi questa sessione» con le condizioni per usarlo.
    fn pulsante_rivivi(&mut self, ui: &mut egui::Ui, pieno: bool) {
        let dal_vivo = self.collegato_dal_vivo();
        let occupato = !matches!(self.stato, StatoConn::Scollegato | StatoConn::Connesso);
        let in_corso = self.in_riproduzione()
            && self
                .info
                .as_ref()
                .and_then(|i| i.riproduzione.as_ref())
                .is_some_and(|r| {
                    self.aperta
                        .as_ref()
                        .is_some_and(|s| nome_file(&s.dati.percorso) == r.file)
                });
        let testo = if in_corso {
            "Rivivi dall'inizio"
        } else {
            "Rivivi questa sessione"
        };
        let r = componenti::pulsante(
            ui,
            pieno,
            !dal_vivo && !occupato,
            componenti::primario(testo),
        )
        .on_hover_text(
            "I dati della sessione scorrono nel tempo su cruscotto, sensori, grafici e MQTT, \
             come se arrivassero dall'auto.",
        )
        .on_disabled_hover_text(if dal_vivo {
            "Con un adattatore collegato non si può rivivere una sessione: scollegalo prima."
        } else {
            "Attendi la fine dell'operazione in corso."
        });
        if r.clicked()
            && let Some(s) = &self.aperta
        {
            let dati = s.dati.clone();
            self.avvia_riproduzione(dati);
        }
        if dal_vivo {
            let scollega = |ui: &mut egui::Ui| {
                ui.label(
                    RichText::new("Adattatore collegato: per rivivere la sessione scollegalo.")
                        .small()
                        .color(tema::ATTENZIONE),
                );
                if componenti::pulsante(ui, pieno, true, egui::Button::new("Scollega l'adattatore"))
                    .on_hover_text("Chiude il collegamento (e la registrazione in corso)")
                    .clicked()
                {
                    self.invia(Comando::Scollega);
                }
            };
            if pieno {
                scollega(ui);
            } else {
                ui.horizontal_wrapped(|ui| scollega(ui));
            }
        }
    }

    fn menu_esporta(&mut self, ui: &mut egui::Ui) {
        let ha_diagnosi = self
            .aperta
            .as_ref()
            .is_some_and(|s| s.dati.diagnosi.is_some());
        ui.menu_button("Esporta…", |ui| {
            for f in FormatoEsportazione::TUTTI {
                let ok = f != FormatoEsportazione::DiagnosiJson || ha_diagnosi;
                if ui
                    .add_enabled(ok, egui::Button::new(format!("{}…", f.nome())))
                    .on_disabled_hover_text("La sessione non contiene una diagnosi")
                    .clicked()
                {
                    self.apri_scelta_sessioni(AzioneSessioni::Esporta(f));
                    ui.close();
                }
            }
            ui.separator();
            componenti::nota(
                ui,
                "Il VIN non compare in nessun formato. Il CSV in stile Torque non contiene i \
                 marcatori né la diagnosi.",
            );
        });
    }

    /// Dettaglio della sessione aperta: intestazione, azioni, riepilogo,
    /// diagnosi registrata, grafici.
    fn sessioni_destra(&mut self, ui: &mut egui::Ui, stretto: bool) {
        if !stretto {
            self.riga_esito(ui);
        }
        if self.caricamento.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Apertura della sessione…");
            });
            return;
        }
        if let Some(e) = &self.errore_sessione {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Sessione non apribile: {e}")).color(tema::PERICOLO),
                )
                .wrap(),
            );
        }
        if self.aperta.is_none() {
            if componenti::stato_vuoto(
                ui,
                "Nessuna sessione aperta",
                "Scegli una sessione dall'elenco per rivedere i grafici senza collegare il \
                 veicolo, oppure importane una da un file.",
                Some("Importa…"),
            ) {
                self.apri_scelta_sessioni(AzioneSessioni::Importa);
            }
            return;
        }
        let (titolo, demo, importata, righe) = {
            let s = self.aperta.as_ref().expect("controllata sopra");
            let i = &s.dati.intestazione;
            let campo = |t: &str| {
                if t.trim().is_empty() {
                    "non indicato".to_string()
                } else {
                    t.to_string()
                }
            };
            let mut righe = vec![
                ("Registrata il", data_leggibile(&i.creata)),
                ("Adattatore", campo(&i.adattatore)),
                ("Protocollo", campo(&i.protocollo)),
                (
                    "Centraline",
                    if i.ecu.is_empty() {
                        "non indicate".to_string()
                    } else {
                        i.ecu.join(", ")
                    },
                ),
                (
                    "Contenuto",
                    format!(
                        "{} campioni, {} marcatori, durata {}",
                        num_it(s.dati.campioni.len() as f64, 0),
                        s.dati.marcatori.len(),
                        mm_ss(s.archivio.t_max)
                    ),
                ),
            ];
            if s.dati.righe_scartate > 0 {
                righe.push((
                    "Righe ignorate",
                    format!("{} illeggibili (file interrotto)", s.dati.righe_scartate),
                ));
            }
            (
                nome_file(&s.dati.percorso),
                i.demo,
                i.importata.clone(),
                righe,
            )
        };
        let (data, etichette, origine) = descrivi_sessione(&titolo);
        componenti::scheda(ui, |ui| {
            componenti::riga_testo(ui, |ui| {
                ui.label(RichText::new(&data).heading().color(tema::TESTO));
                if demo {
                    componenti::pastiglia(ui, "SESSIONE DEMO — dati sintetici", tema::DEMO);
                }
                for e in etichette.iter().filter(|e| **e != "demo") {
                    componenti::pastiglia(ui, e, tema::TESTO_2);
                }
            });
            componenti::nota(ui, titolo.clone());
            if let Some(imp) = &importata {
                componenti::nota(ui, format!("Importata da «{}» ({})", imp.file, imp.formato));
            } else if let Some(o) = origine {
                componenti::nota(ui, o);
            }
            ui.add_space(tema::SP_2);
            for (e, v) in righe {
                componenti::riga_valore(ui, stretto, e, RichText::new(v));
            }
            ui.add_space(tema::SP_3);
            if stretto {
                self.pulsante_rivivi(ui, true);
                ui.horizontal_wrapped(|ui| {
                    self.menu_esporta(ui);
                    self.pulsante_copia_diagnosi(ui);
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    self.pulsante_rivivi(ui, false);
                    self.menu_esporta(ui);
                    self.pulsante_copia_diagnosi(ui);
                });
            }
            if let Some(m) = self.aperta.as_ref().and_then(|s| s.messaggio.clone()) {
                ui.label(RichText::new(m).color(tema::OK));
            }
        });
        let Some(s) = self.aperta.as_mut() else {
            return;
        };
        if let Some(r) = &s.riepilogo {
            componenti::scheda(ui, |ui| {
                egui::CollapsingHeader::new("Riepilogo dell'importazione")
                    .default_open(!stretto)
                    .show(ui, |ui| disegna_riepilogo(ui, r));
            });
        }
        if let Some(d) = &s.dati.diagnosi {
            componenti::scheda(ui, |ui| {
                ui.collapsing("Diagnosi registrata nella sessione", |ui| {
                    if stretto {
                        disegna_diagnosi(ui, d, true);
                    } else {
                        ScrollArea::vertical()
                            .id_salt("sessione_diagnosi")
                            .max_height(420.0)
                            .show(ui, |ui| disegna_diagnosi(ui, d, false));
                    }
                });
            });
        }
        ui.horizontal(|ui| {
            selettore_extra(
                ui,
                "sessione_extra",
                &mut s.extra,
                &s.archivio,
                (!stretto).then_some(210.0),
            );
        });
        ui.add_space(tema::SP_1);
        let disegnato = grafici::disegna(
            ui,
            &s.archivio,
            &Opzioni {
                id: "sessione",
                b1s1: s.preset.b1s1.as_deref(),
                b1s2: s.preset.b1s2.as_deref(),
                extra: s.extra.as_deref(),
                finestra: None,
                sintetico: demo,
                stretto,
            },
        );
        if !disegnato {
            componenti::stato_vuoto(
                ui,
                "Nessun segnale in questa vista",
                "La sessione non contiene le sonde né le correzioni carburante: scegli un \
                 grafico aggiuntivo, oppure rivivila e guarda la pagina Sensori.",
                None,
            );
        }
    }

    fn pulsante_copia_diagnosi(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.aperta.as_mut() else {
            return;
        };
        if let Some(d) = &s.dati.diagnosi
            && ui.button("Copia riepilogo diagnosi").clicked()
        {
            ui.ctx().copy_text(summary::riepilogo(d, false));
            s.messaggio = Some("Riepilogo copiato negli appunti (senza VIN).".into());
        }
    }
}

/// Descrive il nome di un file di sessione: data leggibile, etichette
/// («demo», «importata») e, per le importate, il nome d'origine.
/// `2026-11-22_01-12-49_DEMO.obdio.jsonl` → («22/11/2026 01:12:49», [«demo»], None).
pub(crate) fn descrivi_sessione(nome: &str) -> (String, Vec<&'static str>, Option<String>) {
    let stem = nome.strip_suffix(recording::ESTENSIONE).unwrap_or(nome);
    let (Some(testa), Some(resto)) = (stem.get(..19), stem.get(19..)) else {
        return (stem.to_string(), Vec::new(), None);
    };
    let Ok(data) = chrono::NaiveDateTime::parse_from_str(testa, "%Y-%m-%d_%H-%M-%S") else {
        return (stem.to_string(), Vec::new(), None);
    };
    let mut etichette = Vec::new();
    let mut origine: Vec<&str> = Vec::new();
    let mut importata = false;
    for parte in resto.split('_').filter(|p| !p.is_empty()) {
        match parte {
            "DEMO" if !importata => etichette.push("demo"),
            "IMPORTATA" if !importata => {
                importata = true;
                etichette.push("importata");
            }
            altro if importata => origine.push(altro),
            // Suffisso numerico dei nomi doppi: non dice nulla all'utente.
            _ => {}
        }
    }
    (
        data.format("%d/%m/%Y %H:%M:%S").to_string(),
        etichette,
        (!origine.is_empty()).then(|| format!("Origine: {}", origine.join("_"))),
    )
}

/// Riga «indietro» (layout stretto): freccia e testo in accento, a sinistra.
/// Restituisce `true` se toccata.
fn riga_indietro(ui: &mut egui::Ui, testo: &str) -> bool {
    let alto = ui.spacing().interact_size.y;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), alto), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(
            rect,
            egui::CornerRadius::same(tema::RAGGIO_CONTROLLO),
            tema::SUPERFICIE_ALTA,
        );
    }
    let x = rect.left() + tema::SP_1 + tema::LATO_ICONA / 2.0;
    icone::disegna(
        p,
        egui::pos2(x, rect.center().y),
        tema::LATO_ICONA,
        tema::ACCENTO,
        Icona::Indietro,
    );
    p.text(
        egui::pos2(x + tema::LATO_ICONA / 2.0 + tema::SP_1, rect.center().y),
        egui::Align2::LEFT_CENTER,
        testo,
        tema::font_testo(tema::CARATTERE_CORPO),
        tema::ACCENTO,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// Riga toccabile dell'elenco delle sessioni: data in chiaro ed etichette.
/// Restituisce `true` se è stata scelta.
fn riga_sessione(ui: &mut egui::Ui, nome: &str, scelta: bool) -> bool {
    let (data, etichette, origine) = descrivi_sessione(nome);
    let alto = tema::BERSAGLIO_STRETTO + tema::SP_2;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), alto), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let raggio = egui::CornerRadius::same(tema::RAGGIO_CONTROLLO);
        if scelta {
            p.rect(
                rect,
                raggio,
                tema::velatura(tema::ACCENTO, 0.14),
                egui::Stroke::new(tema::BORDO_SOTTILE, tema::ACCENTO),
                egui::StrokeKind::Inside,
            );
        } else if resp.hovered() {
            p.rect_filled(rect, raggio, tema::SUPERFICIE_ALTA);
        } else {
            p.rect(
                rect,
                raggio,
                tema::SUPERFICIE,
                tema::tratto_bordo(),
                egui::StrokeKind::Inside,
            );
        }
        let x = rect.left() + tema::SP_3;
        let alto_riga = rect.top() + tema::SP_2 + 1.0;
        let g = p.layout_no_wrap(
            data,
            tema::font_testo(tema::CARATTERE_CORPO),
            if scelta { tema::ACCENTO } else { tema::TESTO },
        );
        let mut x_etichetta = x + g.size().x + tema::SP_2;
        p.galley(egui::pos2(x, alto_riga), g, tema::TESTO);
        for e in &etichette {
            let colore = if *e == "demo" {
                tema::DEMO
            } else {
                tema::TESTO_2
            };
            let g = p.layout_no_wrap(
                e.to_string(),
                tema::font_testo(tema::CARATTERE_NOTA),
                colore,
            );
            let r = egui::Rect::from_min_size(
                egui::pos2(x_etichetta, alto_riga + 1.0),
                g.size() + vec2(2.0 * tema::SP_1 + 2.0, 2.0),
            );
            if r.right() > rect.right() - tema::SP_2 {
                break;
            }
            p.rect_filled(r, egui::CornerRadius::same(4), tema::velatura(colore, 0.16));
            p.galley(r.min + vec2(tema::SP_1 + 1.0, 1.0), g, colore);
            x_etichetta = r.right() + tema::SP_1;
        }
        let sotto = origine.unwrap_or_else(|| nome.to_string());
        let g = componenti::testo_troncato(
            p,
            &sotto,
            tema::font_testo(tema::CARATTERE_NOTA),
            tema::TESTO_3,
            rect.width() - 2.0 * tema::SP_3,
        );
        p.galley(
            egui::pos2(x, rect.bottom() - tema::SP_2 - g.size().y),
            g,
            tema::TESTO_3,
        );
    }
    ui.add_space(tema::SP_1);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nomi_delle_sessioni_in_chiaro() {
        assert_eq!(
            descrivi_sessione("2026-11-22_01-12-49_DEMO.obdio.jsonl"),
            ("22/11/2026 01:12:49".to_string(), vec!["demo"], None)
        );
        assert_eq!(
            descrivi_sessione("2026-11-22_01-12-49.obdio.jsonl"),
            ("22/11/2026 01:12:49".to_string(), vec![], None)
        );
        assert_eq!(
            descrivi_sessione("2026-11-22_01-12-49_2.obdio.jsonl").1,
            Vec::<&str>::new()
        );
        assert_eq!(
            descrivi_sessione(
                "2026-11-22_01-12-49_DEMO_IMPORTATA_sessione_demo_torque.obdio.jsonl"
            ),
            (
                "22/11/2026 01:12:49".to_string(),
                vec!["demo", "importata"],
                Some("Origine: sessione_demo_torque".to_string())
            )
        );
        // Un nome scritto a mano resta com'è.
        assert_eq!(
            descrivi_sessione("prova.obdio.jsonl"),
            ("prova".to_string(), vec![], None)
        );
        assert_eq!(descrivi_sessione("x").0, "x");
    }
}
