//! Pagina Cruscotto: pagine di strumenti su una griglia personalizzabile
//! (docs/DESIGN.md, sezioni 3 e 4).
//!
//! - `modello`: formato dei file, griglia, valori predefiniti, modelli pronti;
//! - `sorgenti`: risoluzione delle chiavi, stato del dato, chiavi da leggere;
//! - `strumenti`: disegno dei sette tipi di strumento;
//! - `editor`: modo Modifica, ispettore, pagine, importazione ed esportazione.

use std::collections::BTreeSet;
use std::path::PathBuf;

use eframe::egui::{
    self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2,
};

pub mod editor;
pub mod modello;
pub mod sorgenti;
pub mod strumenti;

use self::modello::{COLONNE_LARGO, COLONNE_STRETTO, Cruscotti, NOME_FILE, Strumento};
use self::sorgenti::{Risolta, SorgenteSpia, StatoValore};
use self::strumenti::{StatoSpia, Vista};
use super::{App, Scheda, tema};
use crate::config;
use crate::obd::pids;
use crate::worker::Comando;

/// Trascinamento in corso nel modo Modifica.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Trascinamento {
    pub id: u64,
    pub ridimensiona: bool,
    pub origine: modello::Cella,
    pub inizio: egui::Pos2,
}

/// Stato della pagina Cruscotto, conservato fra un cambio di scheda e l'altro.
pub(crate) struct StatoCruscotto {
    pub cartella: PathBuf,
    /// Ultimo stato salvato su disco: «Annulla» torna qui.
    pub salvati: Cruscotti,
    /// Stato mostrato e modificato.
    pub correnti: Cruscotti,
    pub pagina: usize,
    pub modifica: bool,
    pub selezionato: Option<u64>,
    pub trascina: Option<Trascinamento>,
    /// Messaggio per l'utente (esito di salvataggi e importazioni).
    pub messaggio: Option<(String, Color32)>,
    pub ricerca: String,
    pub rinomina: Option<String>,
    pub dialogo: Option<editor::DialogoFile>,
    // Lettura automatica.
    era_attiva: bool,
    auto_tentato: bool,
    avviato_da_cruscotto: bool,
    inviato: Option<(Vec<String>, Vec<String>)>,
}

impl StatoCruscotto {
    /// Legge `cruscotti.json`; al primo avvio crea i tre modelli pronti. Un
    /// file illeggibile viene conservato con un altro nome, mai sovrascritto.
    pub fn carica() -> Self {
        Self::carica_da(config::cartella_configurazione())
    }

    pub fn carica_da(cartella: PathBuf) -> Self {
        let mut messaggio = None;
        let cruscotti = match config::carica::<Cruscotti>(&cartella, NOME_FILE) {
            Ok(Some(mut c)) => {
                c.normalizza();
                c
            }
            Ok(None) => {
                let m = modello::modelli();
                if let Err(e) = config::salva(&cartella, NOME_FILE, &m) {
                    messaggio = Some((format!("Cruscotti non salvati: {e}"), tema::PERICOLO));
                }
                m
            }
            Err(e) => {
                let conservato = cartella.join("cruscotti.non_valido.json");
                let _ = std::fs::rename(cartella.join(NOME_FILE), &conservato);
                messaggio = Some((
                    format!(
                        "Il file dei cruscotti non è leggibile ({e}). È stato conservato come \
                         «{}» e sono stati ripristinati i modelli pronti.",
                        conservato.display()
                    ),
                    tema::ATTENZIONE,
                ));
                let m = modello::modelli();
                let _ = config::salva(&cartella, NOME_FILE, &m);
                m
            }
        };
        StatoCruscotto {
            cartella,
            salvati: cruscotti.clone(),
            correnti: cruscotti,
            pagina: 0,
            modifica: false,
            selezionato: None,
            trascina: None,
            messaggio,
            ricerca: String::new(),
            rinomina: None,
            dialogo: None,
            era_attiva: false,
            auto_tentato: false,
            avviato_da_cruscotto: false,
            inviato: None,
        }
    }

    /// Vero se ci sono modifiche non salvate.
    pub fn non_salvato(&self) -> bool {
        self.correnti != self.salvati
    }

    pub fn pagina_corrente(&self) -> Option<&modello::Pagina> {
        self.correnti.pagine.get(self.pagina)
    }

    pub fn salva(&mut self) {
        self.correnti.normalizza();
        match config::salva(&self.cartella, NOME_FILE, &self.correnti) {
            Ok(_) => {
                self.salvati = self.correnti.clone();
                self.modifica = false;
                self.selezionato = None;
                self.messaggio = Some(("Cruscotti salvati.".into(), tema::OK));
            }
            Err(e) => {
                self.messaggio = Some((format!("Salvataggio non riuscito: {e}"), tema::PERICOLO));
            }
        }
    }

    pub fn annulla(&mut self) {
        self.correnti = self.salvati.clone();
        self.modifica = false;
        self.selezionato = None;
        self.trascina = None;
        self.rinomina = None;
        self.pagina = self
            .pagina
            .min(self.correnti.pagine.len().saturating_sub(1));
        self.messaggio = None;
    }
}

/// Dati di uno strumento pronti per il disegno.
struct Preparato {
    strumento: Strumento,
    titolo: String,
    chiave: String,
    unita: String,
    valore: StatoValore,
    picco: Option<f64>,
    min_sessione: Option<f64>,
    max_sessione: Option<f64>,
    parziale: bool,
    spia: Option<StatoSpia>,
}

impl App {
    fn unione(&self) -> Option<&BTreeSet<u8>> {
        self.info.as_ref().map(|i| &i.unione)
    }

    fn risolvi(&self, s: &Strumento) -> Risolta {
        sorgenti::risolvi(&s.chiave, s.tipo, self.unione(), &self.preset, |k| {
            self.live.serie.contains_key(k)
        })
    }

    /// Parametri (risolti) usati dalla pagina visibile del cruscotto.
    pub(crate) fn chiavi_pagina_cruscotto(&self) -> Vec<String> {
        let Some(p) = self.cruscotto.pagina_corrente() else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for s in &p.strumenti {
            if let Some(k) = self.risolvi(s).chiave()
                && !out.iter().any(|c| c == k)
            {
                out.push(k.to_string());
            }
        }
        out
    }

    /// Chiavi da leggere con il cruscotto in vista.
    pub(crate) fn chiavi_polling_cruscotto(&self) -> (Vec<String>, Vec<String>) {
        sorgenti::chiavi_cruscotto(
            &self.selezione,
            &self.preset,
            &self.chiavi_pagina_cruscotto(),
        )
    }

    /// Lettura automatica: chiamata a ogni fotogramma.
    ///
    /// Aprendo il cruscotto da collegati e fermi la lettura parte da sola; con
    /// la lettura attiva le chiavi diventano l'unione di Sensori e pagina
    /// visibile. Lasciando il cruscotto si torna alla selezione dei Sensori e,
    /// se la lettura era partita solo per il cruscotto e non si registra, si
    /// ferma.
    pub(crate) fn governa_polling_cruscotto(&mut self) {
        let attiva = self.scheda == Scheda::Cruscotto;
        if attiva && !self.cruscotto.era_attiva {
            self.cruscotto.auto_tentato = false;
            self.cruscotto.inviato = None;
        }
        if attiva {
            // In riproduzione i dati scorrono tutti: nulla da chiedere.
            if !self.collegato() || self.in_riproduzione() {
                self.cruscotto.inviato = None;
                self.cruscotto.avviato_da_cruscotto = false;
                self.cruscotto.auto_tentato = false;
            } else {
                let voluto = self.chiavi_polling_cruscotto();
                if self.polling {
                    if self.cruscotto.inviato.as_ref() != Some(&voluto) && !voluto.0.is_empty() {
                        self.invia(Comando::AvviaPolling {
                            chiavi: voluto.0.clone(),
                            prioritarie: voluto.1.clone(),
                        });
                        self.cruscotto.inviato = Some(voluto);
                    }
                } else if !self.cruscotto.auto_tentato && self.pronto() && !voluto.0.is_empty() {
                    self.cruscotto.auto_tentato = true;
                    self.cruscotto.avviato_da_cruscotto = true;
                    self.invia(Comando::AvviaPolling {
                        chiavi: voluto.0.clone(),
                        prioritarie: voluto.1.clone(),
                    });
                    self.cruscotto.inviato = Some(voluto);
                }
            }
        } else if self.cruscotto.era_attiva {
            if self.polling || self.cruscotto.inviato.is_some() {
                let (chiavi, _) = self.chiavi_polling();
                if (self.cruscotto.avviato_da_cruscotto && self.registrazione.is_none())
                    || chiavi.is_empty()
                {
                    if self.registrazione.is_none() {
                        self.invia(Comando::FermaPolling);
                    }
                } else {
                    self.avvia_polling();
                }
            }
            self.cruscotto.inviato = None;
            self.cruscotto.avviato_da_cruscotto = false;
        }
        self.cruscotto.era_attiva = attiva;
    }

    fn prepara(&self, s: &Strumento) -> Preparato {
        let risolta = self.risolvi(s);
        let chiave = risolta.chiave().unwrap_or_default().to_string();
        let param = pids::per_key(&chiave);
        let strumento = s.con_scala_per(&chiave);
        let nome_param = param.map(|p| p.nome.clone());
        let parziale = matches!(risolta, Risolta::Parametro { parziale: true, .. });
        let titolo_pred = match &risolta {
            Risolta::Spia(SorgenteSpia::Mil) => "Spia MIL".to_string(),
            Risolta::Spia(SorgenteSpia::Registrazione) => "Registrazione".to_string(),
            Risolta::Assente => match s.chiave.as_str() {
                modello::SONDA_A_MONTE => "Sonda a monte".to_string(),
                modello::SONDA_A_VALLE => "Sonda a valle".to_string(),
                altro => altro.to_string(),
            },
            Risolta::Sconosciuta => format!("Parametro sconosciuto «{}»", s.chiave),
            Risolta::Parametro { .. }
                if parziale && s.tipo == modello::TipoStrumento::Contachilometri =>
            {
                "Distanza dalla connessione".to_string()
            }
            Risolta::Parametro { .. } => nome_param.clone().unwrap_or_default(),
        };
        let titolo = s
            .titolo
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or(titolo_pred);
        let unita = param
            .map(|p| p.unita.to_string())
            .or_else(|| self.live.serie.get(&chiave).map(|x| x.unita.clone()))
            .unwrap_or_default();
        let valore = match &risolta {
            Risolta::Parametro { chiave, .. } => sorgenti::classifica(
                sorgenti::supportato_con(
                    chiave,
                    self.unione(),
                    self.info.as_ref().map_or(&BTreeSet::new(), |i| &i.liberi),
                ),
                self.live.ultimo(chiave),
                self.live.eta_ultimo(chiave),
            ),
            // Le sorgenti di stato hanno senso solo per una spia.
            Risolta::Spia(_) if s.tipo == modello::TipoStrumento::Spia => StatoValore::InAttesa,
            Risolta::Spia(_) => StatoValore::NonSupportato,
            Risolta::Assente | Risolta::Sconosciuta => StatoValore::NonSupportato,
        };
        let serie = self.live.serie.get(&chiave);
        let spia = (s.tipo == modello::TipoStrumento::Spia).then(|| match &risolta {
            Risolta::Spia(SorgenteSpia::Mil) => {
                let accesa = self
                    .diagnosi
                    .as_ref()
                    .filter(|d| !d.prontezza.is_empty())
                    .map(|d| d.prontezza.iter().any(|p| p.mil_accesa));
                StatoSpia {
                    accesa,
                    colore: tema::ATTENZIONE,
                    testo: match accesa {
                        Some(true) => "MIL accesa".into(),
                        Some(false) => "MIL spenta".into(),
                        None => "Leggi la diagnosi".into(),
                    },
                }
            }
            Risolta::Spia(SorgenteSpia::Registrazione) => {
                let accesa = self.registrazione.is_some();
                StatoSpia {
                    accesa: Some(accesa),
                    colore: tema::PERICOLO,
                    testo: if accesa {
                        "In corso".into()
                    } else {
                        "Non attiva".into()
                    },
                }
            }
            _ => match valore.valore() {
                Some(v) => {
                    let numero = strumenti::formatta(&chiave, v, s.decimali);
                    let testo_v = if unita.is_empty() {
                        numero
                    } else {
                        format!("{numero} {unita}")
                    };
                    let livello = strumento.livello(v);
                    let nessuna =
                        strumento.soglia_attenzione.is_none() && strumento.soglia_critica.is_none();
                    StatoSpia {
                        accesa: Some(livello != modello::Livello::Normale),
                        colore: if livello == modello::Livello::Critico {
                            tema::PERICOLO
                        } else {
                            tema::ATTENZIONE
                        },
                        testo: if nessuna {
                            format!("{testo_v} · nessuna soglia")
                        } else if livello != modello::Livello::Normale {
                            format!("Soglia superata · {testo_v}")
                        } else {
                            format!("Nella norma · {testo_v}")
                        },
                    }
                }
                None => StatoSpia {
                    accesa: None,
                    colore: tema::TESTO_3,
                    testo: strumenti::nota_stato(&valore)
                        .map(|n| n.0)
                        .unwrap_or_else(|| "—".into()),
                },
            },
        });
        Preparato {
            strumento,
            titolo,
            chiave,
            unita,
            valore,
            picco: serie.and_then(|x| x.picchi.max),
            min_sessione: serie.and_then(|x| x.stat.min),
            max_sessione: serie.and_then(|x| x.stat.max),
            parziale,
            spia,
        }
    }

    pub(crate) fn pagina_cruscotto(&mut self, ui: &mut egui::Ui) {
        let stretto = ui.available_width() < tema::LARGHEZZA_STRETTA;
        if stretto {
            ui.spacing_mut().interact_size.y = tema::BERSAGLIO_STRETTO;
        } else {
            ui.spacing_mut().interact_size.y = tema::BERSAGLIO_LARGO;
        }
        self.cruscotto.pagina = self
            .cruscotto
            .pagina
            .min(self.cruscotto.correnti.pagine.len().saturating_sub(1));
        self.barra_pagine(ui, stretto);
        if self.cruscotto.modifica {
            self.barra_modifica(ui, stretto);
        }
        self.riga_messaggi(ui);
        let ctx = ui.ctx().clone();
        self.dialogo_file(&ctx);

        if self.cruscotto.modifica && self.cruscotto.selezionato.is_some() {
            if stretto {
                let alto = (ui.available_height() * 0.48).max(220.0);
                egui::Panel::bottom("cruscotto_ispettore")
                    .resizable(false)
                    .exact_size(alto)
                    .show(ui, |ui| self.ispettore(ui, true));
            } else {
                egui::Panel::right("cruscotto_ispettore")
                    .resizable(false)
                    .exact_size(340.0)
                    .show(ui, |ui| self.ispettore(ui, false));
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(0)))
            .show(ui, |ui| self.griglia(ui, stretto));
    }

    fn riga_messaggi(&mut self, ui: &mut egui::Ui) {
        let mut chiudi = false;
        if let Some((m, colore)) = &self.cruscotto.messaggio {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(m).color(*colore));
                if ui.small_button("Chiudi").clicked() {
                    chiudi = true;
                }
            });
        }
        if chiudi {
            self.cruscotto.messaggio = None;
        }
        if !self.collegato() && !self.cruscotto.modifica {
            ui.label(
                RichText::new(
                    "Non collegato: collega l'adattatore o avvia la demo dalla scheda \
                     «Connessione». Gli strumenti mostrano i dati solo quando arrivano.",
                )
                .small()
                .color(tema::ATTENZIONE),
            );
        }
    }

    /// Griglia con gli strumenti; nel modo Modifica gestisce selezione,
    /// spostamento e ridimensionamento.
    fn griglia(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let Some(pagina) = self.cruscotto.pagina_corrente() else {
            return;
        };
        let colonne = if stretto {
            COLONNE_STRETTO
        } else {
            COLONNE_LARGO
        };
        let disposti: Vec<Strumento> = if stretto {
            modello::ricomponi(&pagina.strumenti, colonne)
        } else {
            pagina.strumenti.clone()
        };
        let preparati: Vec<Preparato> = disposti.iter().map(|s| self.prepara(s)).collect();
        let modifica = self.cruscotto.modifica;
        let vuota = disposti.is_empty();
        let barra = 14.0;
        let larghezza = (ui.available_width() - barra).max(120.0);
        let cella = (larghezza / colonne as f32).floor();
        let visibili = (ui.available_height() / cella).floor() as u32;
        let righe = (modello::righe_usate(&disposti) + if modifica { 2 } else { 0 })
            .max(if modifica { visibili } else { 1 })
            .max(1);
        if preparati
            .iter()
            .any(|p| matches!(p.valore, StatoValore::Vecchio { .. }))
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }

        egui::ScrollArea::vertical()
            .id_salt(("cruscotto_griglia", self.cruscotto.pagina))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (area, risposta) = ui.allocate_exact_size(
                    vec2(cella * colonne as f32, cella * righe as f32),
                    Sense::click(),
                );
                let rett = |c: modello::Cella| {
                    Rect::from_min_size(
                        area.min + vec2(c.x as f32 * cella, c.y as f32 * cella),
                        vec2(c.w as f32 * cella, c.h as f32 * cella),
                    )
                    .shrink(tema::SP_1)
                };
                let p = ui.painter_at(area);
                if modifica {
                    for y in 0..=righe {
                        for x in 0..=colonne {
                            p.circle_filled(
                                area.min + vec2(x as f32 * cella, y as f32 * cella),
                                1.4,
                                tema::BORDO_FORTE,
                            );
                        }
                    }
                }
                if vuota {
                    p.text(
                        area.min + vec2(area.width() / 2.0, (cella * 1.5).min(area.height() / 2.0)),
                        egui::Align2::CENTER_CENTER,
                        if modifica {
                            "Pagina vuota: premi «Aggiungi strumento»."
                        } else {
                            "Pagina vuota: premi «Modifica» per aggiungere strumenti."
                        },
                        tema::font_testo(tema::CARATTERE_CORPO),
                        tema::TESTO_2,
                    );
                }
                if modifica && risposta.clicked() {
                    self.cruscotto.selezionato = None;
                }

                // Gesti del modo Modifica (solo su layout largo: su quello
                // stretto la posizione si cambia dall'ispettore).
                let mut trascinato: Option<(u64, modello::Cella, bool)> = None;
                if modifica {
                    for s in &disposti {
                        let r = rett(s.cella());
                        let id = ui.id().with(("strumento", s.id));
                        let corpo = ui.interact(r, id, Sense::click_and_drag()).on_hover_cursor(
                            if stretto {
                                egui::CursorIcon::PointingHand
                            } else {
                                egui::CursorIcon::Grab
                            },
                        );
                        if corpo.clicked() || corpo.drag_started() {
                            self.cruscotto.selezionato = Some(s.id);
                        }
                        if stretto {
                            continue;
                        }
                        let lato = 18.0;
                        let maniglia =
                            Rect::from_min_size(r.max - vec2(lato, lato), vec2(lato, lato));
                        let resp_m = ui
                            .interact(maniglia, id.with("maniglia"), Sense::drag())
                            .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
                        let inizio = |resp: &egui::Response| {
                            resp.interact_pointer_pos().unwrap_or(r.center())
                        };
                        if resp_m.drag_started() {
                            self.cruscotto.selezionato = Some(s.id);
                            self.cruscotto.trascina = Some(Trascinamento {
                                id: s.id,
                                ridimensiona: true,
                                origine: s.cella(),
                                inizio: inizio(&resp_m),
                            });
                        } else if corpo.drag_started() {
                            self.cruscotto.trascina = Some(Trascinamento {
                                id: s.id,
                                ridimensiona: false,
                                origine: s.cella(),
                                inizio: inizio(&corpo),
                            });
                        }
                    }
                    if let Some(t) = self.cruscotto.trascina
                        && let Some(s) = disposti.iter().find(|s| s.id == t.id)
                    {
                        let (puntatore, giu) =
                            ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down()));
                        let d = puntatore.map_or(egui::Vec2::ZERO, |q| q - t.inizio) / cella;
                        let (dx, dy) = (d.x.round() as i64, d.y.round() as i64);
                        let o = t.origine;
                        let somma = |a: u32, b: i64| (a as i64 + b).max(0) as u32;
                        let candidata = if t.ridimensiona {
                            modello::Cella::new(
                                o.x,
                                o.y,
                                somma(o.w, dx).max(1),
                                somma(o.h, dy).max(1),
                            )
                        } else {
                            modello::Cella::new(somma(o.x, dx), somma(o.y, dy), o.w, o.h)
                        };
                        let candidata = modello::limita(candidata, s.tipo, colonne);
                        let valida = !modello::collisione(&disposti, candidata, Some(t.id));
                        if giu {
                            trascinato = Some((t.id, candidata, valida));
                            ui.ctx().request_repaint();
                        } else {
                            self.cruscotto.trascina = None;
                            if valida && candidata != o {
                                self.applica_cella(t.id, candidata);
                            }
                        }
                    }
                }

                // Strumenti (quello trascinato per ultimo, sopra gli altri).
                for (s, pr) in disposti.iter().zip(&preparati) {
                    if trascinato.is_some_and(|t| t.0 == s.id) {
                        continue;
                    }
                    let r = rett(s.cella());
                    self.disegna_strumento(ui, r, s.id, pr);
                }
                if let Some((id, cand, valida)) = trascinato
                    && let Some((s, pr)) = disposti.iter().zip(&preparati).find(|(s, _)| s.id == id)
                {
                    let origine = rett(s.cella());
                    p.rect_stroke(
                        origine,
                        CornerRadius::same(tema::RAGGIO_SCHEDA),
                        Stroke::new(1.0, tema::BORDO_FORTE),
                        StrokeKind::Inside,
                    );
                    let r = rett(cand);
                    let mut copia = pr.strumento.clone();
                    copia.imposta_cella(cand);
                    let pr2 = Preparato {
                        strumento: copia,
                        titolo: pr.titolo.clone(),
                        chiave: pr.chiave.clone(),
                        unita: pr.unita.clone(),
                        valore: pr.valore,
                        picco: pr.picco,
                        min_sessione: pr.min_sessione,
                        max_sessione: pr.max_sessione,
                        parziale: pr.parziale,
                        spia: pr.spia.clone(),
                    };
                    self.disegna_strumento(ui, r, s.id, &pr2);
                    let colore = if valida {
                        tema::ACCENTO
                    } else {
                        tema::PERICOLO
                    };
                    let sopra = ui.painter_at(area);
                    sopra.rect(
                        r,
                        CornerRadius::same(tema::RAGGIO_SCHEDA),
                        tema::velatura(colore, if valida { 0.10 } else { 0.30 }),
                        Stroke::new(tema::BORDO_SELEZIONE, colore),
                        StrokeKind::Inside,
                    );
                    if !valida {
                        sopra.text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            "Posto occupato",
                            tema::font_testo(tema::CARATTERE_CORPO),
                            tema::TESTO,
                        );
                    }
                }
                // Selezione: bordo 2 px e maniglia.
                if modifica
                    && let Some(sel) = self.cruscotto.selezionato
                    && trascinato.is_none()
                    && let Some(s) = disposti.iter().find(|s| s.id == sel)
                {
                    let r = rett(s.cella());
                    let sopra = ui.painter_at(area);
                    sopra.rect_stroke(
                        r,
                        CornerRadius::same(tema::RAGGIO_SCHEDA),
                        Stroke::new(tema::BORDO_SELEZIONE, tema::ACCENTO),
                        StrokeKind::Inside,
                    );
                    if !stretto {
                        let a = r.max - vec2(5.0, 5.0);
                        for k in [6.0, 11.0, 16.0] {
                            sopra.line_segment(
                                [pos2(a.x - k, a.y), pos2(a.x, a.y - k)],
                                Stroke::new(2.0, tema::ACCENTO),
                            );
                        }
                    }
                }
            });
    }

    fn disegna_strumento(&self, ui: &egui::Ui, r: Rect, id: u64, pr: &Preparato) {
        let vista = Vista {
            id: egui::Id::new(("strumento_cruscotto", id)),
            s: &pr.strumento,
            titolo: &pr.titolo,
            chiave: &pr.chiave,
            unita: &pr.unita,
            valore: pr.valore,
            picco: pr.picco,
            min_sessione: pr.min_sessione,
            max_sessione: pr.max_sessione,
            parziale: pr.parziale,
            spia: pr.spia.clone(),
            archivio: &self.live,
        };
        strumenti::disegna(ui, r, &vista);
    }

    /// Sposta o ridimensiona uno strumento della pagina corrente.
    pub(crate) fn applica_cella(&mut self, id: u64, c: modello::Cella) {
        let pagina = self.cruscotto.pagina;
        if let Some(p) = self.cruscotto.correnti.pagine.get_mut(pagina)
            && let Some(s) = p.strumenti.iter_mut().find(|s| s.id == id)
        {
            s.imposta_cella(c);
        }
    }
}
