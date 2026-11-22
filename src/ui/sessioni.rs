//! Scheda Sessioni: sessioni salvate, importazione (formato riconosciuto dal
//! contenuto, con anteprima prima di salvare), esportazione nei quattro
//! formati e «Rivivi questa sessione».

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Color32, RichText, ScrollArea};

use super::grafici::{self, Opzioni};
use super::pagine::{disegna_diagnosi, nome_file, selettore_extra};
use super::scelta_file::{EsitoScelta, SceltaFile, cartella_documenti};
use super::{ARANCIO_DEMO, App, GRIGIO, ROSSO, SessioneAperta, apri_cartella, tema};
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
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{nome}:")).color(tema::TESTO_2));
            ui.label(valore);
        });
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
        let mut salva = false;
        let mut annulla = false;
        let largo = (ctx.content_rect().width() - 2.0 * tema::SP_6).clamp(320.0, 720.0);
        egui::Modal::new(egui::Id::new("anteprima_importazione")).show(ctx, |ui| {
            ui.set_width(largo);
            ui.label(RichText::new("Importazione: che cosa è stato letto").heading());
            ui.label(
                RichText::new(
                    "Nulla è ancora stato salvato. «Salva e apri» crea una nuova sessione OBDio \
                     nella cartella delle sessioni; il file d'origine non viene modificato.",
                )
                .color(tema::TESTO_2),
            );
            ui.add_space(tema::SP_2);
            disegna_riepilogo(ui, &imp.riepilogo);
            ui.add_space(tema::SP_3);
            ui.horizontal(|ui| {
                let pulsante = egui::Button::new(
                    RichText::new("Salva e apri")
                        .color(tema::TESTO_SU_COLORE)
                        .strong(),
                )
                .fill(tema::ACCENTO);
                if ui.add(pulsante).clicked() {
                    salva = true;
                }
                if ui.button("Annulla").clicked() {
                    annulla = true;
                }
            });
        });
        if salva {
            self.conferma_importazione();
            self.scheda = super::Scheda::Sessioni;
        } else if annulla || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.sessioni.anteprima = None;
        }
    }

    // ------------------------------------------------------------------
    // Pagina
    // ------------------------------------------------------------------

    pub(crate) fn pagina_sessioni(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("sessioni_sinistra")
            .resizable(false)
            .exact_size(340.0)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Aggiorna").clicked() {
                        self.elenco = recording::elenca(&self.cartella);
                    }
                    if ui.button("Apri la cartella").clicked() {
                        apri_cartella(&self.cartella);
                    }
                    if ui
                        .button("Importa…")
                        .on_hover_text("Sessione OBDio, CSV OBDio o CSV in stile Torque")
                        .clicked()
                    {
                        self.apri_scelta_sessioni(AzioneSessioni::Importa);
                    }
                });
                ui.label(
                    RichText::new(self.cartella.display().to_string())
                        .small()
                        .color(GRIGIO),
                );
                if self.sessioni.lettura.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Lettura del file da importare…");
                    });
                }
                ui.separator();
                if self.elenco.is_empty() {
                    ui.label("Nessuna sessione salvata.");
                }
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for p in self.elenco.clone() {
                            let nome = nome_file(&p);
                            let breve = nome
                                .strip_suffix(recording::ESTENSIONE)
                                .unwrap_or(&nome)
                                .to_string();
                            let scelta = self.aperta.as_ref().is_some_and(|s| s.dati.percorso == p);
                            if ui.selectable_label(scelta, breve).clicked() {
                                self.apri_sessione(p);
                            }
                        }
                    });
            });
        egui::CentralPanel::default().show(ui, |ui| self.sessioni_destra(ui));
    }

    fn riga_esito(&mut self, ui: &mut egui::Ui) {
        let Some(e) = self.sessioni.esito.clone() else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
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
    fn pulsante_rivivi(&mut self, ui: &mut egui::Ui) {
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
        let pulsante =
            egui::Button::new(RichText::new(testo).color(tema::TESTO_SU_COLORE).strong())
                .fill(tema::ACCENTO);
        let r = ui
            .add_enabled(!dal_vivo && !occupato, pulsante)
            .on_hover_text(
                "I dati della sessione scorrono nel tempo su cruscotto, sensori, grafici e \
                 MQTT, come se arrivassero dall'auto.",
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
            ui.label(
                RichText::new("Adattatore collegato:")
                    .small()
                    .color(tema::ATTENZIONE),
            );
            if ui
                .small_button("Scollega l'adattatore")
                .on_hover_text("Chiude il collegamento (e la registrazione in corso)")
                .clicked()
            {
                self.invia(Comando::Scollega);
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
            ui.label(
                RichText::new(
                    "Il VIN non compare in nessun formato. Il CSV in stile Torque non contiene \
                     i marcatori né la diagnosi.",
                )
                .small()
                .color(tema::TESTO_2),
            );
        });
    }

    fn sessioni_destra(&mut self, ui: &mut egui::Ui) {
        self.riga_esito(ui);
        if self.caricamento.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Apertura della sessione…");
            });
            return;
        }
        if let Some(e) = &self.errore_sessione {
            ui.label(RichText::new(format!("Sessione non apribile: {e}")).color(ROSSO));
        }
        if self.aperta.is_none() {
            ui.label(
                "Scegli una sessione dall'elenco per rivedere i grafici senza collegare il \
                 veicolo, oppure importane una con «Importa…».",
            );
            return;
        }
        let (titolo, demo, importata, info, conteggi) = {
            let s = self.aperta.as_ref().expect("controllata sopra");
            let i = &s.dati.intestazione;
            let campo = |t: &str| {
                if t.trim().is_empty() {
                    "non indicato".to_string()
                } else {
                    t.to_string()
                }
            };
            (
                nome_file(&s.dati.percorso),
                i.demo,
                i.importata.clone(),
                format!(
                    "Registrata il {} — {} — {} — ECU {}",
                    data_leggibile(&i.creata),
                    campo(&i.adattatore),
                    campo(&i.protocollo),
                    if i.ecu.is_empty() {
                        "non indicate".to_string()
                    } else {
                        i.ecu.join(", ")
                    }
                ),
                format!(
                    "{} campioni, {} marcatori, durata {}{}",
                    num_it(s.dati.campioni.len() as f64, 0),
                    s.dati.marcatori.len(),
                    mm_ss(s.archivio.t_max),
                    if s.dati.righe_scartate > 0 {
                        format!(
                            " — {} righe illeggibili ignorate (file interrotto)",
                            s.dati.righe_scartate
                        )
                    } else {
                        String::new()
                    }
                ),
            )
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(titolo).strong());
            if demo {
                ui.label(
                    RichText::new(" SESSIONE DEMO — dati sintetici ")
                        .background_color(ARANCIO_DEMO)
                        .color(Color32::BLACK)
                        .strong(),
                );
            }
            if let Some(imp) = &importata {
                ui.label(
                    RichText::new(format!("importata da «{}» ({})", imp.file, imp.formato))
                        .color(tema::TESTO_2),
                );
            }
        });
        ui.label(info);
        ui.label(conteggi);
        ui.horizontal_wrapped(|ui| {
            self.pulsante_rivivi(ui);
            self.menu_esporta(ui);
            let Some(s) = self.aperta.as_mut() else {
                return;
            };
            if let Some(d) = &s.dati.diagnosi
                && ui.button("Copia riepilogo diagnosi").clicked()
            {
                ui.ctx().copy_text(summary::riepilogo(d, false));
                s.messaggio = Some("Riepilogo copiato negli appunti (senza VIN).".into());
            }
            ui.separator();
            selettore_extra(ui, "sessione_extra", &mut s.extra, &s.archivio);
        });
        let Some(s) = self.aperta.as_mut() else {
            return;
        };
        if let Some(m) = &s.messaggio {
            ui.label(RichText::new(m).color(tema::OK));
        }
        if let Some(r) = &s.riepilogo {
            egui::CollapsingHeader::new("Riepilogo dell'importazione")
                .default_open(true)
                .show(ui, |ui| disegna_riepilogo(ui, r));
        }
        if let Some(d) = &s.dati.diagnosi {
            ui.collapsing("Diagnosi registrata nella sessione", |ui| {
                ScrollArea::vertical()
                    .id_salt("sessione_diagnosi")
                    .max_height(320.0)
                    .show(ui, |ui| disegna_diagnosi(ui, d));
            });
        }
        ui.separator();
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
            },
        );
        if !disegnato {
            ui.label(
                "La sessione non contiene segnali da tracciare in questa vista: scegli un \
                 grafico aggiuntivo, oppure rivivila e guarda la pagina Sensori.",
            );
        }
    }
}
