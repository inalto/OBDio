//! Pagina Sensori: tutti i parametri dichiarati dalla centralina, con ricerca,
//! spunta per attivare la lettura, valori correnti, statistiche e grafici
//! (confronto delle sonde oppure un grafico per sensore).
//!
//! Sul largo elenco e grafici stanno affiancati; sullo stretto un controllo
//! «Elenco | Grafici» sceglie che cosa vedere. Lettura, registrazione e
//! marcatori stanno in una striscia di comandi visibile in entrambi i casi.

use std::collections::BTreeMap;

use eframe::egui::{self, RichText, ScrollArea, Sense, vec2};

use super::componenti::{self, nota, primario, testo_troncato};
use super::grafici::{self, Opzioni};
use super::pagine::{nome_file, selettore_extra};
use super::{App, Scheda, apri_cartella, colore_stato, tema};
use crate::model::num_it;
use crate::obd::pids::{self, Gruppo, Param, PresetP0420};
use crate::summary::{numero_param, valore_con_unita};
use crate::worker::Comando;

const MARCATORI_RAPIDI: [&str; 3] = ["Minimo", "2.500 giri", "Velocità costante"];

/// Larghezza minima di una scheda nella vista «Un grafico per sensore».
const LARGHEZZA_MIN_SCHEDA: f32 = 300.0;
const ALTEZZA_GRAFICO: f32 = 110.0;
const ALTEZZA_SCHEDA: f32 = 200.0;
/// Margine interno della scheda (per lato) più il bordo: 2 × (12 + 1).
const INGOMBRO_SCHEDA: f32 = 26.0;
/// Larghezza del pannello dell'elenco sul layout largo.
const LARGHEZZA_ELENCO: f32 = 372.0;

/// Come sono mostrati i grafici.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VistaSensori {
    /// Grafici impilati delle sonde, con asse del tempo condiviso.
    Confronto,
    /// Una scheda con il proprio grafico per ogni sensore selezionato.
    PerSensore,
}

impl VistaSensori {
    fn nome(self) -> &'static str {
        match self {
            VistaSensori::Confronto => "Confronto",
            VistaSensori::PerSensore => "Un grafico per sensore",
        }
    }

    /// Nome per la casella a discesa del layout stretto.
    fn nome_breve(self) -> &'static str {
        match self {
            VistaSensori::Confronto => "Confronto",
            VistaSensori::PerSensore => "Per sensore",
        }
    }
}

/// Che cosa mostra la pagina sul layout stretto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PannelloSensori {
    Elenco,
    Grafici,
}

const FINESTRE: [(Option<f64>, &str); 5] = [
    (Some(30.0), "30 s"),
    (Some(60.0), "60 s"),
    (Some(120.0), "2 min"),
    (Some(300.0), "5 min"),
    (None, "Tutto"),
];

/// Colonne della griglia «Un grafico per sensore» per la larghezza data.
pub(crate) fn colonne_schede(disponibile: f32, spazio: f32) -> usize {
    (((disponibile + spazio) / (LARGHEZZA_MIN_SCHEDA + spazio)).floor() as usize).max(1)
}

/// Selezione «Diagnosi P0420»: esattamente le sonde e i parametri di base
/// supportati del preset.
pub(crate) fn selezione_p0420(preset: &PresetP0420) -> BTreeMap<String, bool> {
    let mut s = BTreeMap::new();
    for k in preset.sonde() {
        s.insert(k, true);
    }
    for (k, supportato) in &preset.base {
        if *supportato {
            s.insert(k.clone(), true);
        }
    }
    s
}

/// Chiavi da richiedere alla centralina e, fra queste, le prioritarie.
///
/// I parametri calcolati dall'app non vengono mai richiesti: scegliere la
/// distanza parziale fa leggere la velocità da cui deriva. Le prioritarie sono
/// le sonde del preset, ma solo se selezionate.
pub(crate) fn chiavi_da_selezione(
    selezione: &BTreeMap<String, bool>,
    preset: &PresetP0420,
) -> (Vec<String>, Vec<String>) {
    let attivo = |k: &str| selezione.get(k).copied().unwrap_or(false);
    let mut chiavi: Vec<String> = Vec::new();
    for k in selezione.keys().filter(|k| attivo(k)) {
        let da_leggere = match pids::per_key(k) {
            Some(p) if p.derivato => match pids::sorgente_derivato(k) {
                Some(sorgente) => sorgente,
                None => continue,
            },
            _ => k.as_str(),
        };
        if !chiavi.iter().any(|c| c == da_leggere) {
            chiavi.push(da_leggere.to_string());
        }
    }
    let prioritarie = preset.sonde().into_iter().filter(|k| attivo(k)).collect();
    (chiavi, prioritarie)
}

impl App {
    /// Parametri da mostrare nell'elenco, per gruppo e poi per PID. Collegati:
    /// quelli che la centralina dichiara (i calcolati se è disponibile la
    /// sorgente; in riproduzione, quelli presenti nella sessione, liberi
    /// compresi). Scollegati: quelli presenti nei dati già letti.
    pub(crate) fn parametri_disponibili(&self) -> Vec<&'static Param> {
        let disponibile = |p: &Param| match &self.info {
            Some(info) if p.libero => info.liberi.contains(&p.key),
            Some(info) if p.derivato => pids::sorgente_derivato(&p.key)
                .and_then(pids::per_key)
                .is_some_and(|s| info.unione.contains(&s.pid)),
            Some(info) => info.unione.contains(&p.pid),
            None => self.live.serie.contains_key(&p.key),
        };
        let mut v: Vec<&Param> = pids::catalogo()
            .iter()
            .chain(pids::liberi())
            .filter(|p| disponibile(p))
            .collect();
        v.sort_by_key(|p| (p.gruppo, p.derivato, p.libero, p.pid));
        v
    }

    /// Imposta la selezione rapida «Diagnosi P0420».
    fn seleziona_p0420(&mut self) {
        let preset = if self.collegato() {
            self.preset.clone()
        } else {
            PresetP0420::calcola(&self.live.pid_presenti())
        };
        self.selezione = selezione_p0420(&preset);
    }

    /// Se la lettura è attiva, riparte con le chiavi della nuova selezione.
    fn selezione_cambiata(&self) {
        if self.polling {
            self.avvia_polling();
        }
    }

    pub(crate) fn pagina_sensori(&mut self, ui: &mut egui::Ui) {
        if self.disposizione.stretta() {
            componenti::segmentato(
                ui,
                &mut self.pannello_sensori,
                &[
                    (PannelloSensori::Elenco, "Elenco"),
                    (PannelloSensori::Grafici, "Grafici"),
                ],
            );
            ui.add_space(tema::SP_2);
            self.comandi_sensori(ui, true);
            ui.add_space(tema::SP_2);
            match self.pannello_sensori {
                PannelloSensori::Elenco => self.elenco_sensori(ui, true),
                PannelloSensori::Grafici => self.sensori_grafici(ui, true),
            }
            return;
        }
        egui::Panel::left("sensori_sinistra")
            .resizable(false)
            .exact_size(LARGHEZZA_ELENCO)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 0,
                right: tema::SP_4 as i8,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| {
                componenti::cornice_scheda().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.comandi_sensori(ui, false);
                });
                ui.add_space(tema::SP_3);
                self.elenco_sensori(ui, false);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: tema::SP_2 as i8,
                right: 0,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| self.sensori_grafici(ui, false));
    }

    // ------------------------------------------------------------------
    // Comandi: lettura, registrazione, marcatori
    // ------------------------------------------------------------------

    /// Tre pulsanti uguali (lettura, registrazione, marcatore) e lo stato
    /// della registrazione.
    fn comandi_sensori(&mut self, ui: &mut egui::Ui, stretto: bool) {
        if self.in_riproduzione() {
            ui.add(
                egui::Label::new(
                    RichText::new(if stretto {
                        "Riproduzione: le spunte scelgono che cosa mostrare. Lettura, \
                         registrazione e marcatori non servono."
                    } else {
                        "Riproduzione: scorrono tutti i dati della sessione. Non si avvia la \
                         lettura, non si registra e non si aggiungono marcatori (quelli \
                         registrati sono sulla barra di trasporto). Le spunte scelgono che \
                         cosa mostrare."
                    })
                    .color(tema::DEMO),
                )
                .wrap(),
            );
            return;
        }
        let ha_chiavi = !self.chiavi_polling().0.is_empty();
        let alto = ui.spacing().interact_size.y;
        let spazio = tema::SP_2;
        let largo = ((ui.available_width() - 2.0 * spazio) / 3.0).floor();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spazio;
            // Lettura.
            if self.polling {
                if ui
                    .add_enabled_ui(self.registrazione.is_none(), |ui| {
                        ui.add_sized([largo, alto], egui::Button::new("Ferma lettura"))
                    })
                    .inner
                    .on_disabled_hover_text("Prima ferma la registrazione")
                    .clicked()
                {
                    self.invia(Comando::FermaPolling);
                }
            } else if ui
                .add_enabled_ui(self.pronto() && ha_chiavi, |ui| {
                    ui.add_sized([largo, alto], primario("Avvia lettura"))
                })
                .inner
                .on_disabled_hover_text("Seleziona almeno un sensore e collega l'adattatore")
                .clicked()
            {
                self.avvia_polling();
            }
            // Registrazione.
            let suggerimento = "La registrazione prosegue anche con la finestra ridotta a \
                                icona. Durante la registrazione le sonde vengono lette più \
                                spesso degli altri parametri.";
            if self.registrazione.is_some() {
                if ui
                    .add_sized(
                        [largo, alto],
                        componenti::pericolo_tenue(if stretto {
                            "Ferma REC"
                        } else {
                            "Ferma reg."
                        }),
                    )
                    .on_hover_text("Ferma la registrazione e salva la sessione")
                    .clicked()
                {
                    self.invia(Comando::FermaRegistrazione);
                }
            } else if ui
                .add_enabled_ui(self.pronto() && (self.polling || ha_chiavi), |ui| {
                    ui.add_sized([largo, alto], egui::Button::new("Registra"))
                })
                .inner
                .on_hover_text(suggerimento)
                .on_disabled_hover_text("Collega l'adattatore e seleziona almeno un sensore")
                .clicked()
            {
                if !self.polling {
                    self.avvia_polling();
                }
                self.invia(Comando::AvviaRegistrazione);
            }
            // Marcatori.
            let attivi = self.collegato() && self.polling;
            let r = ui
                .add_enabled_ui(attivi, |ui| {
                    ui.add_sized([largo, alto], egui::Button::new("Marcatore…"))
                })
                .inner
                .on_disabled_hover_text("I marcatori si aggiungono con la lettura attiva");
            egui::Popup::menu(&r)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(260.0)
                .show(|ui| {
                    ui.label(RichText::new("Aggiungi un marcatore").strong());
                    nota(
                        ui,
                        "Segna un momento della lettura (e della registrazione).",
                    );
                    for m in MARCATORI_RAPIDI {
                        if ui.button(m).clicked() {
                            self.invia(Comando::Marcatore(m.to_string()));
                            ui.close();
                        }
                    }
                    ui.add_space(tema::SP_1);
                    componenti::campo_testo(
                        ui,
                        egui::TextEdit::singleline(&mut self.testo_marcatore)
                            .hint_text("Altro marcatore…")
                            .desired_width(f32::INFINITY),
                    );
                    let testo = self.testo_marcatore.trim().to_string();
                    if ui
                        .add_enabled(!testo.is_empty(), primario("Aggiungi"))
                        .clicked()
                    {
                        self.invia(Comando::Marcatore(testo));
                        self.testo_marcatore.clear();
                        ui.close();
                    }
                });
        });
        if let Some(r) = &self.registrazione {
            let s = (self.live.t_max - r.t_inizio).max(0.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Registrazione in corso da {}",
                        super::navigazione::durata_breve(s)
                    ))
                    .color(tema::PERICOLO),
                );
                if !stretto {
                    ui.label(
                        RichText::new(nome_file(&r.percorso))
                            .small()
                            .color(tema::TESTO_2),
                    );
                }
            });
        } else if let Some(u) = &self.ultima {
            let testo = format!(
                "Ultima sessione salvata: {} campioni{}",
                num_it(u.campioni as f64, 0),
                if u.csv.is_some() {
                    ", con CSV"
                } else {
                    " — CSV non scritto"
                }
            );
            let mut apri = false;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(testo).small().color(tema::TESTO_2));
                apri = ui.small_button("Apri la cartella").clicked();
            });
            if apri {
                apri_cartella(&self.cartella);
            }
        }
    }

    // ------------------------------------------------------------------
    // Elenco dei sensori
    // ------------------------------------------------------------------

    fn elenco_sensori(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let disponibili = self.parametri_disponibili();
        if disponibili.is_empty() && !self.collegato() {
            if componenti::stato_vuoto(
                ui,
                "Nessun sensore da mostrare",
                "Collega l'adattatore o avvia la demo: qui compaiono i parametri dichiarati \
                 dalla centralina, da scegliere per la lettura.",
                Some("Vai a Connessione"),
            ) {
                self.scheda = Scheda::Connessione;
            }
            return;
        }
        componenti::campo_testo(
            ui,
            egui::TextEdit::singleline(&mut self.ricerca)
                .hint_text("Cerca per nome…")
                .desired_width(ui.available_width()),
        );
        let filtro = self.ricerca.trim().to_lowercase();
        let visibili: Vec<&Param> = disponibili
            .iter()
            .filter(|p| filtro.is_empty() || p.nome.to_lowercase().contains(&filtro))
            .copied()
            .collect();

        let mut cambiata = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Diagnosi P0420")
                .on_hover_text(
                    "Sonde a monte e a valle, correzioni carburante, giri, velocità e temperatura",
                )
                .clicked()
            {
                self.seleziona_p0420();
                cambiata = true;
            }
            if ui
                .button("Tutti")
                .on_hover_text("Seleziona tutti i sensori dell'elenco (rispetta la ricerca)")
                .clicked()
            {
                for p in &visibili {
                    self.selezione.insert(p.key.clone(), true);
                }
                cambiata = true;
            }
            if ui
                .button("Nessuno")
                .on_hover_text(
                    "Deseleziona i sensori (con una ricerca attiva, solo quelli visibili)",
                )
                .clicked()
            {
                if filtro.is_empty() {
                    self.selezione.clear();
                } else {
                    for p in &visibili {
                        self.selezione.remove(&p.key);
                    }
                }
                cambiata = true;
            }
        });
        let n_attivi = self.selezione.values().filter(|v| **v).count();
        let mut riepilogo = format!(
            "{} selezionati su {} disponibili",
            n_attivi,
            disponibili.len()
        );
        if !self.collegato() {
            riepilogo.push_str(" · non collegato: parametri dei dati già letti");
        }
        nota(ui, riepilogo);
        ui.add_space(tema::SP_1);

        ScrollArea::vertical()
            .id_salt("sensori_elenco")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if disponibili.is_empty() {
                    nota(ui, "La centralina non dichiara parametri noti all'app.");
                } else if visibili.is_empty() {
                    nota(ui, "Nessun sensore corrisponde alla ricerca.");
                }
                for g in Gruppo::TUTTI {
                    let voci: Vec<&&Param> = visibili.iter().filter(|p| p.gruppo == g).collect();
                    if voci.is_empty() {
                        continue;
                    }
                    ui.add_space(tema::SP_2);
                    ui.label(
                        RichText::new(g.nome())
                            .small()
                            .strong()
                            .color(tema::TESTO_2),
                    );
                    for p in voci {
                        if self.riga_sensore(ui, p) {
                            cambiata = true;
                        }
                    }
                }
                ui.add_space(tema::SP_2);
                nota(
                    ui,
                    "Hz = frequenza effettiva di aggiornamento negli ultimi 5 secondi.",
                );
                if stretto && n_attivi > 0 {
                    ui.add_space(tema::SP_2);
                    if componenti::pulsante(ui, true, true, egui::Button::new("Vedi i grafici"))
                        .clicked()
                    {
                        self.pannello_sensori = PannelloSensori::Grafici;
                    }
                }
                ui.add_space(tema::SP_4);
            });
        if cambiata {
            self.selezione_cambiata();
        }
    }

    /// Una riga dell'elenco, tutta toccabile: casella, nome e, se attivo,
    /// valore e Hz a destra. Restituisce `true` se la spunta è cambiata.
    fn riga_sensore(&mut self, ui: &mut egui::Ui, p: &Param) -> bool {
        let mut attivo = self.selezione.get(&p.key).copied().unwrap_or(false);
        let alto = ui.spacing().interact_size.y;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), alto), Sense::click());
        let resp = if p.derivato {
            resp.on_hover_text("Calcolata dall'app integrando la velocità")
        } else {
            resp
        };
        let cambiata = resp.clicked();
        if cambiata {
            attivo = !attivo;
            self.selezione.insert(p.key.clone(), attivo);
        }
        if !ui.is_rect_visible(rect) {
            return cambiata;
        }
        let pt = ui.painter();
        let raggio = egui::CornerRadius::same(tema::RAGGIO_CONTROLLO);
        if resp.hovered() {
            pt.rect_filled(rect, raggio, tema::SUPERFICIE_ALTA);
        }
        // Casella di spunta disegnata (18 px, come quelle di egui).
        let lato = 18.0;
        let casella = egui::Rect::from_center_size(
            egui::pos2(rect.left() + tema::SP_1 + lato / 2.0, rect.center().y),
            vec2(lato, lato),
        );
        if attivo {
            pt.rect_filled(casella, egui::CornerRadius::same(4), tema::ACCENTO);
            let c = casella.center();
            pt.line(
                vec![
                    c + vec2(-4.5, 0.0),
                    c + vec2(-1.5, 3.5),
                    c + vec2(4.5, -3.5),
                ],
                egui::Stroke::new(2.0, tema::TESTO_SU_COLORE),
            );
        } else {
            pt.rect_stroke(
                casella,
                egui::CornerRadius::same(4),
                egui::Stroke::new(tema::BORDO_SOTTILE, tema::TESTO_3),
                egui::StrokeKind::Inside,
            );
        }
        // Valore e Hz a destra.
        let mut destra = rect.right() - tema::SP_2;
        if attivo {
            if let Some(hz) = self.live.frequenza(&p.key) {
                let g = pt.layout_no_wrap(
                    format!("{} Hz", num_it(hz, 1)),
                    tema::font_testo(tema::CARATTERE_NOTA),
                    tema::TESTO_3,
                );
                destra -= g.size().x;
                pt.galley(
                    egui::pos2(destra, rect.center().y - g.size().y / 2.0),
                    g,
                    tema::TESTO_3,
                );
                destra -= tema::SP_2;
            }
            let (testo, colore) = match self.live.ultimo(&p.key) {
                Some(x) => match x.v {
                    Some(v) => (valore_con_unita(&p.key, v, p.unita), tema::TESTO),
                    None => (x.stato.etichetta().to_string(), colore_stato(x.stato)),
                },
                None => ("in attesa".to_string(), tema::TESTO_3),
            };
            let g = pt.layout_no_wrap(testo, tema::font_testo(tema::CARATTERE_CORPO), colore);
            destra -= g.size().x;
            pt.galley(
                egui::pos2(destra, rect.center().y - g.size().y / 2.0),
                g,
                colore,
            );
            destra -= tema::SP_3;
        }
        let x_nome = casella.right() + tema::SP_2;
        let nome = testo_troncato(
            pt,
            &p.nome,
            tema::font_testo(tema::CARATTERE_CORPO),
            if attivo { tema::TESTO } else { tema::TESTO_2 },
            destra - x_nome,
        );
        pt.galley(
            egui::pos2(x_nome, rect.center().y - nome.size().y / 2.0),
            nome,
            tema::TESTO,
        );
        cambiata
    }

    // ------------------------------------------------------------------
    // Grafici
    // ------------------------------------------------------------------

    fn sensori_grafici(&mut self, ui: &mut egui::Ui, stretto: bool) {
        if stretto {
            let spazio = tema::SP_2;
            let combo = ((ui.available_width() - spazio) / 2.0).floor();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spazio;
                egui::ComboBox::from_id_salt("sensori_vista")
                    .width(combo)
                    .selected_text(self.vista.nome_breve())
                    .show_ui(ui, |ui| {
                        for v in [VistaSensori::Confronto, VistaSensori::PerSensore] {
                            ui.selectable_value(&mut self.vista, v, v.nome());
                        }
                    });
                egui::ComboBox::from_id_salt("sensori_finestra")
                    .width(combo)
                    .selected_text(
                        FINESTRE
                            .iter()
                            .find(|f| f.0 == self.finestra)
                            .map_or("Tutto", |f| f.1),
                    )
                    .show_ui(ui, |ui| {
                        for (valore, nome) in FINESTRE {
                            ui.selectable_value(&mut self.finestra, valore, nome);
                        }
                    });
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                for v in [VistaSensori::Confronto, VistaSensori::PerSensore] {
                    ui.selectable_value(&mut self.vista, v, v.nome());
                }
                ui.add_space(tema::SP_4);
                ui.label(RichText::new("Finestra").color(tema::TESTO_2));
                for (valore, nome) in FINESTRE {
                    ui.selectable_value(&mut self.finestra, valore, nome);
                }
            });
        }
        ui.horizontal(|ui| match self.vista {
            VistaSensori::Confronto => {
                selettore_extra(
                    ui,
                    "live_extra",
                    &mut self.extra,
                    &self.live,
                    (!stretto).then_some(210.0),
                );
            }
            VistaSensori::PerSensore => {
                if ui
                    .button("Azzera min/max")
                    .on_hover_text("Riparte da zero con minimo, medio e massimo di ogni sensore")
                    .clicked()
                {
                    self.live.azzera_statistiche();
                }
            }
        });
        nota(
            ui,
            "I grafici servono a confrontare i segnali: l'app non deduce se catalizzatore o \
             sonde siano guasti. Le fasce rosse indicano intervalli senza dato valido.",
        );
        ui.add_space(tema::SP_1);
        match self.vista {
            VistaSensori::Confronto => self.sensori_confronto(ui, stretto),
            VistaSensori::PerSensore => self.sensori_griglia(ui),
        }
    }

    /// Stato vuoto dei grafici, con l'azione che serve.
    fn grafici_vuoti(&mut self, ui: &mut egui::Ui) {
        if self.collegato() {
            let pronto = self.pronto() && !self.chiavi_polling().0.is_empty();
            if componenti::stato_vuoto(
                ui,
                "Nessun segnale da mostrare",
                if self.in_riproduzione() {
                    "La sessione non contiene questi segnali: scegli un grafico aggiuntivo o la \
                     vista «Un grafico per sensore»."
                } else {
                    "Avvia la lettura per vedere i segnali dei sensori selezionati."
                },
                (pronto && !self.polling && !self.in_riproduzione()).then_some("Avvia lettura"),
            ) {
                self.avvia_polling();
            }
        } else if componenti::stato_vuoto(
            ui,
            "Nessun dato",
            "Collega l'adattatore o avvia la demo, poi avvia la lettura.",
            Some("Vai a Connessione"),
        ) {
            self.scheda = Scheda::Connessione;
        }
    }

    /// Confronto delle sonde e delle correzioni carburante (grafici impilati).
    fn sensori_confronto(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let opzioni = Opzioni {
            id: "live",
            b1s1: self.preset.b1s1.as_deref(),
            b1s2: self.preset.b1s2.as_deref(),
            extra: self.extra.as_deref(),
            finestra: self.finestra,
            sintetico: self.live_demo,
            stretto,
        };
        let disegnato = if stretto {
            ScrollArea::vertical()
                .id_salt("sensori_confronto")
                .auto_shrink([false, false])
                .show(ui, |ui| grafici::disegna(ui, &self.live, &opzioni))
                .inner
        } else {
            grafici::disegna(ui, &self.live, &opzioni)
        };
        if !disegnato {
            self.grafici_vuoti(ui);
        }
    }

    /// Griglia di schede, una per sensore selezionato.
    fn sensori_griglia(&mut self, ui: &mut egui::Ui) {
        let sensori: Vec<&'static Param> = self
            .parametri_disponibili()
            .into_iter()
            .filter(|p| self.selezione.get(&p.key).copied().unwrap_or(false))
            .collect();
        if sensori.is_empty() {
            let stretto = self.disposizione.stretta();
            if componenti::stato_vuoto(
                ui,
                "Nessun sensore selezionato",
                if stretto {
                    "Scegli i sensori nell'«Elenco»: qui compare una scheda con il grafico per \
                     ognuno."
                } else {
                    "Spunta i sensori nell'elenco a sinistra: qui compare una scheda con il \
                     grafico per ognuno."
                },
                stretto.then_some("Vai all'elenco"),
            ) {
                self.pannello_sensori = PannelloSensori::Elenco;
            }
            return;
        }
        let spazio = ui.spacing().item_spacing.x;
        // Un margine per la barra di scorrimento verticale.
        let disponibile = ui.available_width() - tema::SP_3;
        let colonne = colonne_schede(disponibile, spazio);
        let larghezza = ((disponibile - spazio * (colonne - 1) as f32) / colonne as f32).floor();

        let mut area = ScrollArea::vertical()
            .id_salt("sensori_griglia")
            .auto_shrink([false, false]);
        if let Some(y) = self.scorri_sensori.take() {
            area = area.vertical_scroll_offset(y);
        }
        area.show(ui, |ui| {
            for riga in sensori.chunks(colonne) {
                ui.horizontal_top(|ui| {
                    for p in riga {
                        self.scheda_sensore(ui, p, larghezza);
                    }
                });
            }
            ui.add_space(tema::SP_4);
        });
    }

    /// Scheda di un sensore: nome, valore grande, statistiche, Hz e grafico.
    fn scheda_sensore(&self, ui: &mut egui::Ui, p: &Param, larghezza: f32) {
        componenti::cornice_scheda()
            .inner_margin(egui::Margin::same(tema::SP_3 as i8))
            // La scheda è nel gruppo orizzontale di una riga: il suo contenuto
            // va impilato in verticale.
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width((larghezza - INGOMBRO_SCHEDA).max(120.0));
                    ui.set_min_height(ALTEZZA_SCHEDA);
                    let nome = if self.live_demo {
                        format!("{} (sintetico)", p.nome)
                    } else {
                        p.nome.clone()
                    };
                    let hz = match self.live.frequenza(&p.key) {
                        Some(hz) => format!("{} Hz", num_it(hz, 1)),
                        None => "— Hz".to_string(),
                    };
                    intestazione_scheda(ui, &nome, &hz);
                    ui.horizontal(|ui| match self.live.ultimo(&p.key) {
                        Some(pt) => match pt.v {
                            Some(v) => {
                                ui.label(
                                    RichText::new(numero_param(&p.key, v))
                                        .font(tema::font_numeri(tema::VALORE_MIN))
                                        .color(tema::TESTO),
                                );
                                if !p.unita.is_empty() {
                                    ui.label(RichText::new(p.unita).color(tema::TESTO_2));
                                }
                            }
                            None => {
                                ui.label(
                                    RichText::new(pt.stato.etichetta())
                                        .color(colore_stato(pt.stato)),
                                );
                            }
                        },
                        None => {
                            ui.label(RichText::new("in attesa").color(tema::TESTO_3));
                        }
                    });
                    let st = self
                        .live
                        .serie
                        .get(&p.key)
                        .map(|s| s.stat)
                        .unwrap_or_default();
                    let num = |v: Option<f64>| match v {
                        Some(v) => numero_param(&p.key, v),
                        None => "—".to_string(),
                    };
                    ui.label(
                        RichText::new(format!(
                            "min {} · medio {} · max {}",
                            num(st.min),
                            num(st.media()),
                            num(st.max)
                        ))
                        .small()
                        .color(tema::TESTO_2),
                    );
                    grafici::disegna_sensore(
                        ui,
                        &self.live,
                        &p.key,
                        ALTEZZA_GRAFICO,
                        self.finestra,
                        self.live_demo,
                    );
                });
            });
    }
}

/// Riga del titolo di una scheda: nome a sinistra (troncato), nota a destra.
fn intestazione_scheda(ui: &mut egui::Ui, nome: &str, nota: &str) {
    let alto = tema::SP_6;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), alto), Sense::hover());
    let p = ui.painter();
    let n = p.layout_no_wrap(
        nota.to_string(),
        tema::font_testo(tema::CARATTERE_NOTA),
        tema::TESTO_3,
    );
    let x_nota = rect.right() - n.size().x;
    p.galley(
        egui::pos2(x_nota, rect.center().y - n.size().y / 2.0),
        n,
        tema::TESTO_3,
    );
    let g = testo_troncato(
        p,
        nome,
        tema::font_testo(tema::CARATTERE_CORPO),
        tema::TESTO_2,
        x_nota - rect.left() - tema::SP_2,
    );
    p.galley(
        egui::pos2(rect.left(), rect.center().y - g.size().y / 2.0),
        g,
        tema::TESTO_2,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn preset_demo() -> PresetP0420 {
        let s: BTreeSet<u8> = [0x04, 0x05, 0x06, 0x07, 0x0B, 0x0C, 0x0D, 0x15, 0x24]
            .into_iter()
            .collect();
        PresetP0420::calcola(&s)
    }

    #[test]
    fn una_scheda_per_riga_sul_telefono() {
        // 390 px di schermo, 12 px di margine per lato, 12 per lo scorrimento.
        assert_eq!(colonne_schede(390.0 - 24.0 - 12.0, 8.0), 1);
        assert_eq!(colonne_schede(300.0, 8.0), 1);
        // Sul largo (1280 px meno barra laterale, margini ed elenco) due colonne.
        assert_eq!(
            colonne_schede(1280.0 - 184.0 - 32.0 - 372.0 - 8.0 - 12.0, 8.0),
            2
        );
        assert_eq!(colonne_schede(1100.0, 8.0), 3);
    }

    #[test]
    fn selezione_p0420_e_esattamente_sonde_e_base() {
        let s = selezione_p0420(&preset_demo());
        let chiavi: BTreeSet<&str> = s.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            chiavi,
            BTreeSet::from([
                "o2s1_lambda",
                "o2s2_tensione",
                "stft_b1",
                "ltft_b1",
                "giri",
                "velocita",
                "temp_refrigerante"
            ])
        );
        assert!(s.values().all(|v| *v));
        // Senza sonde né base supportati la selezione è vuota.
        assert!(selezione_p0420(&PresetP0420::calcola(&BTreeSet::new())).is_empty());
    }

    #[test]
    fn chiavi_con_priorita_alle_sonde_selezionate() {
        let preset = preset_demo();
        let mut sel = selezione_p0420(&preset);
        sel.insert("carico".into(), true);
        sel.insert("map".into(), false);
        let (chiavi, prioritarie) = chiavi_da_selezione(&sel, &preset);
        assert_eq!(
            prioritarie,
            vec!["o2s1_lambda".to_string(), "o2s2_tensione".to_string()]
        );
        assert!(chiavi.contains(&"carico".to_string()));
        assert!(!chiavi.contains(&"map".to_string()));
        assert!(chiavi.contains(&"o2s1_lambda".to_string()));

        // Una sonda deselezionata non è più prioritaria né letta.
        sel.insert("o2s2_tensione".into(), false);
        let (chiavi, prioritarie) = chiavi_da_selezione(&sel, &preset);
        assert_eq!(prioritarie, vec!["o2s1_lambda".to_string()]);
        assert!(!chiavi.contains(&"o2s2_tensione".to_string()));
    }

    #[test]
    fn distanza_parziale_non_e_mai_richiesta_ma_fa_leggere_la_velocita() {
        let preset = PresetP0420::default();
        let sel = BTreeMap::from([(pids::CHIAVE_DISTANZA_PARZIALE.to_string(), true)]);
        let (chiavi, prioritarie) = chiavi_da_selezione(&sel, &preset);
        assert_eq!(chiavi, vec!["velocita".to_string()]);
        assert!(prioritarie.is_empty());

        // Se la velocità è già selezionata non compare due volte.
        let sel = BTreeMap::from([
            (pids::CHIAVE_DISTANZA_PARZIALE.to_string(), true),
            ("velocita".to_string(), true),
        ]);
        let (chiavi, _) = chiavi_da_selezione(&sel, &preset);
        assert_eq!(chiavi, vec!["velocita".to_string()]);
        assert!(!chiavi.iter().any(|k| k == pids::CHIAVE_DISTANZA_PARZIALE));

        // Nessuna spunta, nessuna richiesta.
        let vuota = BTreeMap::from([("giri".to_string(), false)]);
        assert!(chiavi_da_selezione(&vuota, &preset).0.is_empty());
    }
}
