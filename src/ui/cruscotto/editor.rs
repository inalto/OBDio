//! Modo Modifica del cruscotto: schede delle pagine, barra «Modifica del
//! cruscotto», ispettore dello strumento selezionato, importazione ed
//! esportazione in JSON.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, DragValue, RichText, ScrollArea};

use super::modello::{
    self, Cella, Pagina, SONDA_A_MONTE, SONDA_A_VALLE, SPIA_MIL, SPIA_REGISTRAZIONE, Soglia,
    Strumento, TipoStrumento, Verso,
};
use super::sorgenti;
use crate::obd::pids;
use crate::ui::scelta_file::{EsitoScelta, SceltaFile, cartella_documenti};
use crate::ui::{App, tema};

/// Cosa fare con il file scelto nella finestra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AzioneFile {
    EsportaPagina(usize),
    EsportaTutto,
    Importa,
}

/// Finestra di importazione o esportazione (vedi [`SceltaFile`]).
pub(crate) struct DialogoFile {
    azione: AzioneFile,
    scelta: SceltaFile,
}

/// Nome di file sicuro a partire dal nome di una pagina.
pub fn nome_file_pagina(nome: &str) -> String {
    let pulito: String = nome
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let pulito = pulito.trim_matches('_');
    if pulito.is_empty() {
        "cruscotto.json".into()
    } else {
        format!("cruscotto_{pulito}.json")
    }
}

/// Aggiunge «.json» se il nome non ha estensione.
fn con_estensione(p: &Path) -> PathBuf {
    if p.extension().is_none() {
        p.with_extension("json")
    } else {
        p.to_path_buf()
    }
}

/// Pulsante dell'azione primaria (accento pieno).
fn pulsante_primario(ui: &mut egui::Ui, testo: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(testo).color(tema::TESTO_SU_COLORE).strong())
            .fill(tema::ACCENTO),
    )
}

/// Pulsante di un'azione distruttiva.
fn pulsante_pericolo(ui: &mut egui::Ui, testo: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(testo).color(tema::PERICOLO))
            .stroke(egui::Stroke::new(1.0, tema::velatura(tema::PERICOLO, 0.6))),
    )
}

/// Campo numerico intero con «−» e «+» accanto.
fn passo(ui: &mut egui::Ui, valore: &mut u32, min: u32, max: u32) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(*valore > min, egui::Button::new("−"))
            .clicked()
        {
            *valore -= 1;
        }
        ui.add(DragValue::new(valore).range(min..=max).speed(0.1));
        if ui
            .add_enabled(*valore < max, egui::Button::new("+"))
            .clicked()
        {
            *valore += 1;
        }
    });
}

/// Velocità di trascinamento adatta all'ampiezza della scala.
fn velocita_scala(s: &Strumento) -> f64 {
    ((s.max - s.min).abs() / 200.0).max(0.001)
}

/// Riga dell'ispettore per una soglia (attivazione, valore, verso).
fn riga_soglia(
    ui: &mut egui::Ui,
    id: &str,
    nome: &str,
    colore: Color32,
    soglia: &mut Option<Soglia>,
    predefinita: f64,
    velocita: f64,
) {
    let mut attiva = soglia.is_some();
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut attiva, RichText::new(nome).color(colore))
            .changed()
        {
            *soglia = attiva.then(|| Soglia::sopra(predefinita));
        }
    });
    if let Some(s) = soglia {
        ui.horizontal(|ui| {
            ui.add_space(tema::SP_6);
            egui::ComboBox::from_id_salt(id)
                .selected_text(s.verso.nome())
                .width(150.0)
                .show_ui(ui, |ui| {
                    for v in Verso::TUTTI {
                        ui.selectable_value(&mut s.verso, v, v.nome());
                    }
                });
            ui.add(DragValue::new(&mut s.valore).speed(velocita));
        });
    }
}

impl App {
    // ------------------------------------------------------------------
    // Schede delle pagine e barra dei comandi
    // ------------------------------------------------------------------

    pub(crate) fn barra_pagine(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let mut nuova_pagina = self.cruscotto.pagina;
        ui.horizontal(|ui| {
            ScrollArea::horizontal()
                .id_salt("cruscotto_schede")
                .max_width(if stretto {
                    ui.available_width()
                } else {
                    ui.available_width() - 420.0
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, p) in self.cruscotto.correnti.pagine.iter().enumerate() {
                            ui.selectable_value(&mut nuova_pagina, i, p.nome.as_str());
                        }
                    });
                });
            if stretto {
                return;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.comandi_pagina(ui, true);
            });
        });
        if stretto {
            ui.horizontal_wrapped(|ui| self.comandi_pagina(ui, false));
        }
        if nuova_pagina != self.cruscotto.pagina {
            self.cruscotto.pagina = nuova_pagina;
            self.cruscotto.selezionato = None;
            self.cruscotto.trascina = None;
            self.cruscotto.rinomina = None;
        }
    }

    /// Comandi a destra delle schede (modo Uso) o gestione delle pagine
    /// (modo Modifica).
    fn comandi_pagina(&mut self, ui: &mut egui::Ui, rtl: bool) {
        // Da destra a sinistra il primo comando finisce all'estremo destro.
        let ordine = if rtl { [0, 1, 2] } else { [2, 1, 0] };
        for i in ordine {
            self.comando_pagina(ui, i);
        }
    }

    /// Comando `i` della riga delle schede: 0 è il più importante.
    fn comando_pagina(&mut self, ui: &mut egui::Ui, i: usize) {
        match (self.cruscotto.modifica, i) {
            (true, 0) => {
                let sola = self.cruscotto.correnti.pagine.len() <= 1;
                if ui
                    .add_enabled(
                        !sola,
                        egui::Button::new(RichText::new("Elimina pagina").color(tema::PERICOLO)),
                    )
                    .on_disabled_hover_text("Il cruscotto ha almeno una pagina")
                    .clicked()
                {
                    let i = self.cruscotto.pagina;
                    self.cruscotto.correnti.pagine.remove(i);
                    self.cruscotto.pagina = i.saturating_sub(1);
                    self.cruscotto.selezionato = None;
                }
            }
            (true, 1) => {
                if ui.button("Rinomina").clicked() {
                    self.cruscotto.rinomina =
                        self.cruscotto.pagina_corrente().map(|p| p.nome.clone());
                }
            }
            (true, _) => {
                if ui.button("Nuova pagina").clicked() {
                    let n = self.cruscotto.correnti.pagine.len() + 1;
                    self.cruscotto.correnti.pagine.push(Pagina {
                        nome: format!("Pagina {n}"),
                        strumenti: Vec::new(),
                    });
                    self.cruscotto.pagina = n - 1;
                    self.cruscotto.selezionato = None;
                }
            }
            (false, 0) => {
                if pulsante_primario(ui, "Modifica").clicked() {
                    self.cruscotto.modifica = true;
                    self.cruscotto.messaggio = None;
                }
            }
            (false, 1) => self.menu_file(ui),
            (false, _) => {
                if ui
                    .button("Azzera picchi")
                    .on_hover_text(
                        "Riparte da zero con i marcatori di picco di tutti gli strumenti",
                    )
                    .clicked()
                {
                    self.live.azzera_picchi();
                }
            }
        }
    }

    fn menu_file(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Importa / esporta", |ui| {
            if ui.button("Esporta questa pagina…").clicked() {
                let i = self.cruscotto.pagina;
                self.apri_dialogo(AzioneFile::EsportaPagina(i));
                ui.close();
            }
            if ui.button("Esporta tutti i cruscotti…").clicked() {
                self.apri_dialogo(AzioneFile::EsportaTutto);
                ui.close();
            }
            if ui.button("Importa da file…").clicked() {
                self.apri_dialogo(AzioneFile::Importa);
                ui.close();
            }
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "I cruscotti sono salvati in {}",
                    self.cruscotto.cartella.join(modello::NOME_FILE).display()
                ))
                .small()
                .color(tema::TESTO_2),
            );
        });
    }

    pub(crate) fn barra_modifica(&mut self, ui: &mut egui::Ui, stretto: bool) {
        egui::Frame::new()
            .fill(tema::SUPERFICIE)
            .stroke(egui::Stroke::new(
                tema::BORDO_SOTTILE,
                tema::velatura(tema::ACCENTO, 0.7),
            ))
            .corner_radius(egui::CornerRadius::same(tema::RAGGIO_SCHEDA))
            .inner_margin(egui::Margin::symmetric(tema::SP_3 as i8, tema::SP_2 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if let Some(mut nome) = self.cruscotto.rinomina.take() {
                    let mut chiudi = false;
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Nome della pagina:");
                        let r = ui.add(egui::TextEdit::singleline(&mut nome).desired_width(220.0));
                        let invio = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.button("Applica").clicked() || invio {
                            let i = self.cruscotto.pagina;
                            if let Some(p) = self.cruscotto.correnti.pagine.get_mut(i)
                                && !nome.trim().is_empty()
                            {
                                p.nome = nome.trim().to_string();
                            }
                            chiudi = true;
                        }
                        if ui.button("Lascia com'è").clicked() {
                            chiudi = true;
                        }
                    });
                    if !chiudi {
                        self.cruscotto.rinomina = Some(nome);
                    }
                    ui.separator();
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Modifica del cruscotto")
                            .strong()
                            .color(tema::ACCENTO),
                    );
                    if self.cruscotto.non_salvato() {
                        ui.label(RichText::new("Modifiche non salvate").color(tema::ATTENZIONE));
                    } else {
                        ui.label(RichText::new("Nessuna modifica").color(tema::TESTO_3));
                    }
                    if !stretto {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.pulsanti_modifica(ui);
                        });
                    }
                });
                if stretto {
                    ui.horizontal_wrapped(|ui| self.pulsanti_modifica_ltr(ui));
                }
                ui.label(
                    RichText::new(if stretto {
                        "Tocca uno strumento per cambiarlo; posizione e misura si cambiano \
                         dall'ispettore (si riferiscono alla griglia a 12 colonne)."
                    } else {
                        "Trascina uno strumento per spostarlo, l'angolo in basso a destra per \
                         ridimensionarlo; un clic lo seleziona e apre l'ispettore. «Annulla» \
                         torna all'ultimo salvataggio."
                    })
                    .small()
                    .color(tema::TESTO_2),
                );
            });
        ui.add_space(tema::SP_1);
    }

    /// Pulsanti della barra di modifica in ordine da destra a sinistra.
    fn pulsanti_modifica(&mut self, ui: &mut egui::Ui) {
        if ui.button("Annulla").clicked() {
            self.cruscotto.annulla();
        }
        if pulsante_primario(ui, "Salva").clicked() {
            self.cruscotto.salva();
        }
        if ui.button("Aggiungi strumento").clicked() {
            self.aggiungi_strumento();
        }
    }

    fn pulsanti_modifica_ltr(&mut self, ui: &mut egui::Ui) {
        if ui.button("Aggiungi strumento").clicked() {
            self.aggiungi_strumento();
        }
        if pulsante_primario(ui, "Salva").clicked() {
            self.cruscotto.salva();
        }
        if ui.button("Annulla").clicked() {
            self.cruscotto.annulla();
        }
    }

    /// Parametri disponibili per l'ispettore: supportati prima, poi gli altri.
    /// I parametri liberi (sessioni importate) compaiono solo se presenti
    /// nella sorgente collegata o nei dati già ricevuti.
    fn candidati(&self) -> Vec<(&'static pids::Param, bool)> {
        let unione = self.info.as_ref().map(|i| &i.unione);
        let vuoto = Default::default();
        let liberi = self.info.as_ref().map_or(&vuoto, |i| &i.liberi);
        let mut v: Vec<(&'static pids::Param, bool)> = pids::catalogo()
            .iter()
            .chain(
                pids::liberi()
                    .into_iter()
                    .filter(|p| liberi.contains(&p.key) || self.live.serie.contains_key(&p.key)),
            )
            .map(|p| {
                let ok = match sorgenti::supportato_con(&p.key, unione, liberi) {
                    Some(s) => s,
                    None => self.live.serie.contains_key(&p.key),
                };
                (p, ok)
            })
            .collect();
        v.sort_by_key(|(p, ok)| (!*ok, p.gruppo, p.derivato, p.pid));
        v
    }

    /// Aggiunge uno strumento sensato nel primo posto libero e lo seleziona.
    pub(crate) fn aggiungi_strumento(&mut self) {
        let presenti: Vec<String> = self
            .cruscotto
            .pagina_corrente()
            .map(|p| p.strumenti.iter().map(|s| s.chiave.clone()).collect())
            .unwrap_or_default();
        let candidati = self.candidati();
        let preferiti = [
            "giri",
            "velocita",
            "temp_refrigerante",
            "carico",
            "map",
            "farfalla",
            "tensione_modulo",
        ];
        let chiave = preferiti
            .iter()
            .find(|k| {
                !presenti.iter().any(|p| p == *k)
                    && candidati.iter().any(|(p, ok)| *ok && p.key == **k)
            })
            .map(|k| k.to_string())
            .or_else(|| {
                candidati
                    .iter()
                    .find(|(p, ok)| *ok && !presenti.contains(&p.key))
                    .map(|(p, _)| p.key.clone())
            })
            .unwrap_or_else(|| "giri".into());
        let tipo = modello::tipo_suggerito(&chiave);
        let id = self.cruscotto.correnti.nuovo_id();
        let pagina = self.cruscotto.pagina;
        let Some(p) = self.cruscotto.correnti.pagine.get_mut(pagina) else {
            return;
        };
        let mut s = Strumento::predefinito(&chiave, tipo);
        s.id = id;
        let (x, y) = modello::primo_libero(&p.strumenti, s.w, s.h, modello::COLONNE_LARGO);
        s.x = x;
        s.y = y;
        p.strumenti.push(s);
        self.cruscotto.selezionato = Some(id);
    }

    // ------------------------------------------------------------------
    // Ispettore
    // ------------------------------------------------------------------

    pub(crate) fn ispettore(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let Some(id) = self.cruscotto.selezionato else {
            return;
        };
        let pagina = self.cruscotto.pagina;
        let Some(originale) = self
            .cruscotto
            .correnti
            .pagine
            .get(pagina)
            .and_then(|p| p.strumenti.iter().find(|s| s.id == id))
            .cloned()
        else {
            self.cruscotto.selezionato = None;
            return;
        };
        let mut s = originale.clone();
        let candidati = self.candidati();
        let mut duplica = false;
        let mut elimina = false;
        let mut ripristina = false;
        let mut chiudi = false;

        ui.add_space(tema::SP_2);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Strumento").heading());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Chiudi").clicked() {
                    chiudi = true;
                }
            });
        });
        ScrollArea::vertical()
            .id_salt("cruscotto_ispettore_scorri")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Parametro
                ui.label(RichText::new("Parametro").strong());
                let nome_attuale = match s.chiave.as_str() {
                    SPIA_MIL => "Spia MIL (ultima diagnosi)".to_string(),
                    SPIA_REGISTRAZIONE => "Registrazione in corso".to_string(),
                    SONDA_A_MONTE => "Sonda a monte (scelta automatica)".to_string(),
                    SONDA_A_VALLE => "Sonda a valle (scelta automatica)".to_string(),
                    k => pids::per_key(k)
                        .map(|p| p.nome.clone())
                        .unwrap_or_else(|| format!("Sconosciuto «{k}»")),
                };
                ui.label(RichText::new(nome_attuale).color(tema::ACCENTO));
                ui.add(
                    egui::TextEdit::singleline(&mut self.cruscotto.ricerca)
                        .hint_text("Cerca un parametro…")
                        .desired_width(ui.available_width()),
                );
                let filtro = self.cruscotto.ricerca.trim().to_lowercase();
                let corrisponde =
                    |nome: &str| filtro.is_empty() || nome.to_lowercase().contains(&filtro);
                ScrollArea::vertical()
                    .id_salt("cruscotto_parametri")
                    .max_height(if stretto { 140.0 } else { 200.0 })
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let speciali: &[(&str, &str)] = &[
                            (SONDA_A_MONTE, "Sonda a monte (scelta automatica)"),
                            (SONDA_A_VALLE, "Sonda a valle (scelta automatica)"),
                            (SPIA_MIL, "Spia MIL (ultima diagnosi)"),
                            (SPIA_REGISTRAZIONE, "Registrazione in corso"),
                        ];
                        for (k, nome) in speciali {
                            let spia = matches!(*k, SPIA_MIL | SPIA_REGISTRAZIONE);
                            if spia && s.tipo != TipoStrumento::Spia {
                                continue;
                            }
                            if corrisponde(nome)
                                && ui.selectable_label(s.chiave == *k, *nome).clicked()
                            {
                                s.chiave = k.to_string();
                            }
                        }
                        for (p, ok) in &candidati {
                            if !corrisponde(&p.nome) {
                                continue;
                            }
                            let testo = if *ok {
                                RichText::new(p.nome.as_str())
                            } else {
                                RichText::new(format!("{} — non supportato", p.nome))
                                    .color(tema::TESTO_3)
                            };
                            if ui.selectable_label(s.chiave == p.key, testo).clicked() {
                                s.chiave = p.key.clone();
                            }
                        }
                    });
                ui.add_space(tema::SP_2);

                egui::Grid::new("cruscotto_ispettore_griglia")
                    .num_columns(2)
                    .spacing([tema::SP_3, tema::SP_2])
                    .show(ui, |ui| {
                        ui.label("Tipo");
                        egui::ComboBox::from_id_salt("cruscotto_tipo")
                            .selected_text(s.tipo.nome())
                            .show_ui(ui, |ui| {
                                for t in TipoStrumento::TUTTI {
                                    ui.selectable_value(&mut s.tipo, t, t.nome());
                                }
                            });
                        ui.end_row();

                        ui.label("Titolo");
                        let mut titolo = s.titolo.clone().unwrap_or_default();
                        let suggerito = pids::per_key(&s.chiave)
                            .map(|p| p.nome.clone())
                            .unwrap_or_else(|| "Nome del parametro".into());
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut titolo)
                                    .hint_text(suggerito)
                                    .desired_width(180.0),
                            )
                            .changed()
                        {
                            s.titolo = (!titolo.trim().is_empty()).then_some(titolo);
                        }
                        ui.end_row();

                        let unita = pids::per_key(&s.chiave).map_or("", |p| p.unita);
                        if !unita.is_empty() {
                            ui.label("Unità");
                            ui.label(RichText::new(unita).color(tema::TESTO_2));
                            ui.end_row();
                        }
                    });

                let senza_scala =
                    matches!(s.tipo, TipoStrumento::Spia | TipoStrumento::Contachilometri)
                        && s.soglia_attenzione.is_none()
                        && s.soglia_critica.is_none();
                ui.add_space(tema::SP_2);
                ui.label(RichText::new("Scala e soglie").strong());
                if matches!(s.chiave.as_str(), SONDA_A_MONTE | SONDA_A_VALLE) {
                    ui.checkbox(
                        &mut s.scala_automatica,
                        "Scala dal segnale esposto dalla centralina",
                    );
                }
                ui.add_enabled_ui(!s.scala_automatica, |ui| {
                    let velocita = velocita_scala(&s);
                    ui.horizontal(|ui| {
                        ui.label("Minimo");
                        ui.add(DragValue::new(&mut s.min).speed(velocita));
                        ui.label("Massimo");
                        ui.add(DragValue::new(&mut s.max).speed(velocita));
                    });
                    if s.max <= s.min {
                        ui.label(
                            RichText::new(
                                "Il massimo deve superare il minimo: vale la scala predefinita.",
                            )
                            .small()
                            .color(tema::ATTENZIONE),
                        );
                    }
                    let meta = s.min + (s.max - s.min) * 0.75;
                    riga_soglia(
                        ui,
                        "cruscotto_verso_att",
                        "Soglia di attenzione",
                        tema::ATTENZIONE,
                        &mut s.soglia_attenzione,
                        meta,
                        velocita,
                    );
                    let alta = s.min + (s.max - s.min) * 0.9;
                    riga_soglia(
                        ui,
                        "cruscotto_verso_crit",
                        "Soglia critica",
                        tema::PERICOLO,
                        &mut s.soglia_critica,
                        alta,
                        velocita,
                    );
                });
                if senza_scala {
                    ui.label(
                        RichText::new(
                            "Per una spia su un parametro, la soglia decide quando si accende.",
                        )
                        .small()
                        .color(tema::TESTO_3),
                    );
                }

                ui.add_space(tema::SP_2);
                ui.label(RichText::new("Visualizzazione").strong());
                let mut automatici = s.decimali.is_none();
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut automatici, "Decimali del parametro")
                        .changed()
                    {
                        s.decimali = if automatici {
                            None
                        } else {
                            Some(pids::per_key(&s.chiave).map_or(1, |p| p.decimali))
                        };
                    }
                    if let Some(d) = &mut s.decimali {
                        ui.add(DragValue::new(d).range(0..=6).prefix("decimali: "));
                    }
                });
                if matches!(
                    s.tipo,
                    TipoStrumento::Quadrante | TipoStrumento::Arco | TipoStrumento::Barra
                ) {
                    ui.checkbox(&mut s.mostra_picco, "Marcatore di picco");
                }
                if s.tipo == TipoStrumento::Grafico {
                    ui.horizontal(|ui| {
                        ui.label("Durata");
                        ui.add(
                            DragValue::new(&mut s.secondi)
                                .range(5.0..=900.0)
                                .speed(1.0)
                                .suffix(" s"),
                        );
                    });
                }

                ui.add_space(tema::SP_2);
                ui.label(RichText::new("Posizione e misura").strong());
                if stretto {
                    ui.label(
                        RichText::new("Valori della griglia a 12 colonne del layout largo.")
                            .small()
                            .color(tema::TESTO_3),
                    );
                }
                egui::Grid::new("cruscotto_posizione")
                    .num_columns(2)
                    .spacing([tema::SP_3, tema::SP_1])
                    .show(ui, |ui| {
                        let colonne = modello::COLONNE_LARGO;
                        ui.label("Colonna");
                        passo(ui, &mut s.x, 0, colonne.saturating_sub(s.w));
                        ui.end_row();
                        ui.label("Riga");
                        passo(ui, &mut s.y, 0, modello::RIGHE_MAX);
                        ui.end_row();
                        ui.label("Larghezza");
                        passo(ui, &mut s.w, 1, colonne.saturating_sub(s.x).max(1));
                        ui.end_row();
                        ui.label("Altezza");
                        passo(ui, &mut s.h, 1, 12);
                        ui.end_row();
                    });

                ui.add_space(tema::SP_3);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button("Ripristina predefiniti")
                        .on_hover_text("Scala, soglie, decimali e picco secondo il parametro")
                        .clicked()
                    {
                        ripristina = true;
                    }
                    if ui.button("Duplica").clicked() {
                        duplica = true;
                    }
                    if pulsante_pericolo(ui, "Elimina").clicked() {
                        elimina = true;
                    }
                });
                ui.add_space(tema::SP_4);
            });

        if chiudi {
            self.cruscotto.selezionato = None;
        }
        if s.chiave != originale.chiave {
            // Un parametro nuovo porta con sé scala e soglie sensate.
            s.ripristina_scala();
            s.scala_automatica = matches!(s.chiave.as_str(), SONDA_A_MONTE | SONDA_A_VALLE);
        }
        if ripristina {
            s.ripristina_scala();
        }
        self.salva_ispettore(&originale, s, duplica, elimina);
    }

    /// Applica le modifiche dell'ispettore rispettando la griglia.
    fn salva_ispettore(
        &mut self,
        originale: &Strumento,
        mut s: Strumento,
        duplica: bool,
        elimina: bool,
    ) {
        let pagina = self.cruscotto.pagina;
        let nuovo_id = self.cruscotto.correnti.nuovo_id();
        let Some(p) = self.cruscotto.correnti.pagine.get_mut(pagina) else {
            return;
        };
        if elimina {
            p.strumenti.retain(|x| x.id != originale.id);
            self.cruscotto.selezionato = None;
            return;
        }
        if s != *originale {
            let c = modello::limita(s.cella(), s.tipo, modello::COLONNE_LARGO);
            s.imposta_cella(c);
            if modello::collisione(&p.strumenti, c, Some(s.id)) {
                if s.tipo != originale.tipo {
                    // Il tipo nuovo chiede più spazio: si sposta nel primo posto libero.
                    let altri: Vec<Strumento> = p
                        .strumenti
                        .iter()
                        .filter(|x| x.id != s.id)
                        .cloned()
                        .collect();
                    let (x, y) = modello::primo_libero(&altri, c.w, c.h, modello::COLONNE_LARGO);
                    s.x = x;
                    s.y = y;
                } else {
                    s.imposta_cella(originale.cella());
                    self.cruscotto.messaggio = Some((
                        "Posto occupato da un altro strumento: posizione e misura non cambiate."
                            .into(),
                        tema::PERICOLO,
                    ));
                }
            }
            if let Some(x) = p.strumenti.iter_mut().find(|x| x.id == s.id) {
                *x = s.clone();
            }
        }
        if duplica {
            let mut copia = s.clone();
            copia.id = nuovo_id;
            let (x, y) =
                modello::primo_libero(&p.strumenti, copia.w, copia.h, modello::COLONNE_LARGO);
            copia.imposta_cella(Cella::new(x, y, copia.w, copia.h));
            p.strumenti.push(copia);
            self.cruscotto.selezionato = Some(nuovo_id);
        }
    }

    // ------------------------------------------------------------------
    // Importazione ed esportazione
    // ------------------------------------------------------------------

    fn apri_dialogo(&mut self, azione: AzioneFile) {
        let nome = match azione {
            AzioneFile::EsportaPagina(i) => self
                .cruscotto
                .correnti
                .pagine
                .get(i)
                .map_or("cruscotto.json".into(), |p| nome_file_pagina(&p.nome)),
            AzioneFile::EsportaTutto => "cruscotti_obdio.json".into(),
            AzioneFile::Importa => String::new(),
        };
        let (titolo, pulsante, spiegazione) = match azione {
            AzioneFile::Importa => (
                "Importa un cruscotto",
                "Importa",
                "Scrivi il percorso di un file JSON esportato da OBDio (una pagina o l'intero \
                 insieme), oppure scegline uno dall'elenco. Le pagine importate si aggiungono a \
                 quelle esistenti.",
            ),
            AzioneFile::EsportaPagina(_) => (
                "Esporta questa pagina",
                "Esporta",
                "Scrivi dove salvare il file JSON. Se esiste già, viene sostituito.",
            ),
            AzioneFile::EsportaTutto => (
                "Esporta tutti i cruscotti",
                "Esporta",
                "Scrivi dove salvare il file JSON. Se esiste già, viene sostituito.",
            ),
        };
        let scelta = SceltaFile::new(titolo, pulsante, &cartella_documenti(), &nome)
            .spiegazione(spiegazione)
            .finali(&[".json"]);
        self.cruscotto.dialogo = Some(DialogoFile { azione, scelta });
    }

    pub(crate) fn dialogo_file(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.cruscotto.dialogo else {
            return;
        };
        match d.scelta.mostra(ctx, "cruscotto") {
            EsitoScelta::Aperta => {}
            EsitoScelta::Annulla => self.cruscotto.dialogo = None,
            EsitoScelta::Conferma(percorso) => {
                let azione = d.azione;
                self.cruscotto.dialogo = None;
                self.esegui_file(azione, &percorso);
            }
        }
    }

    fn esegui_file(&mut self, azione: AzioneFile, percorso: &Path) {
        let esito = match azione {
            AzioneFile::EsportaPagina(i) => match self.cruscotto.correnti.pagine.get(i) {
                Some(p) => scrivi_json(&con_estensione(percorso), p),
                None => Err("pagina inesistente".into()),
            },
            AzioneFile::EsportaTutto => {
                scrivi_json(&con_estensione(percorso), &self.cruscotto.correnti)
            }
            AzioneFile::Importa => {
                match std::fs::read_to_string(percorso)
                    .map_err(|e| format!("impossibile leggere il file ({e})"))
                    .and_then(|t| modello::leggi_importazione(&t))
                {
                    Ok(pagine) => {
                        let n = pagine.len();
                        let primo = self.cruscotto.correnti.aggiungi_pagine(pagine);
                        self.cruscotto.pagina = primo;
                        self.cruscotto.modifica = true;
                        self.cruscotto.selezionato = None;
                        self.cruscotto.messaggio = Some((
                            format!(
                                "{} importat{} da «{}»: premi «Salva» per tenerl{}, «Annulla» per \
                                 scartarl{}.",
                                if n == 1 {
                                    "Pagina".to_string()
                                } else {
                                    format!("{n} pagine")
                                },
                                if n == 1 { "a" } else { "e" },
                                percorso.display(),
                                if n == 1 { "a" } else { "e" },
                                if n == 1 { "a" } else { "e" },
                            ),
                            tema::OK,
                        ));
                        return;
                    }
                    Err(e) => Err(e),
                }
            }
        };
        self.cruscotto.messaggio = Some(match (azione, esito) {
            (AzioneFile::Importa, Err(e)) => (
                format!("Importazione non riuscita: {e}. Il cruscotto non è stato cambiato."),
                tema::PERICOLO,
            ),
            (_, Err(e)) => (format!("Esportazione non riuscita: {e}"), tema::PERICOLO),
            (_, Ok(p)) => (format!("Esportato in «{}».", p.display()), tema::OK),
        });
    }
}

fn scrivi_json<T: serde::Serialize>(percorso: &Path, valore: &T) -> Result<PathBuf, String> {
    let testo = serde_json::to_string_pretty(valore).map_err(|e| e.to_string())?;
    std::fs::write(percorso, testo).map_err(|e| format!("impossibile scrivere il file ({e})"))?;
    Ok(percorso.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nomi_dei_file_esportati() {
        assert_eq!(nome_file_pagina("Guida"), "cruscotto_guida.json");
        assert_eq!(
            nome_file_pagina("Diagnosi P0420"),
            "cruscotto_diagnosi_p0420.json"
        );
        assert_eq!(nome_file_pagina("  "), "cruscotto.json");
        assert_eq!(con_estensione(Path::new("a/b")), PathBuf::from("a/b.json"));
        assert_eq!(
            con_estensione(Path::new("a/b.txt")),
            PathBuf::from("a/b.txt")
        );
    }

    #[test]
    fn esportazione_e_reimportazione() {
        let dir = std::env::temp_dir().join(format!("obdio_cruscotto_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let m = modello::modelli();
        let f = scrivi_json(&dir.join("tutto.json"), &m).unwrap();
        let pagine = modello::leggi_importazione(&std::fs::read_to_string(f).unwrap()).unwrap();
        assert_eq!(pagine, m.pagine);
        let f = scrivi_json(&dir.join("una.json"), &m.pagine[1]).unwrap();
        let pagine = modello::leggi_importazione(&std::fs::read_to_string(f).unwrap()).unwrap();
        assert_eq!(pagine, vec![m.pagine[1].clone()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
