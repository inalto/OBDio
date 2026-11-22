//! Pagina Sensori: tutti i parametri dichiarati dalla centralina, con ricerca,
//! spunta per attivare la lettura, valori correnti, statistiche e grafici
//! (confronto delle sonde oppure un grafico per sensore).

use std::collections::BTreeMap;

use eframe::egui::{self, RichText, ScrollArea};

use super::grafici::{self, Opzioni};
use super::pagine::{nome_file, selettore_extra};
use super::{
    ARANCIO_DEMO, App, GIALLO, GRIGIO, ROSSO, apri_cartella, colore_stato, etichetta_stato,
};
use crate::model::num_it;
use crate::obd::pids::{self, Gruppo, Param, PresetP0420};
use crate::summary::{numero_param, valore_con_unita};
use crate::worker::Comando;

const MARCATORI_RAPIDI: [&str; 3] = ["Minimo", "2.500 giri", "Velocità costante"];

/// Larghezza minima di una scheda nella vista «Un grafico per sensore».
const LARGHEZZA_MIN_SCHEDA: f32 = 360.0;
const ALTEZZA_GRAFICO: f32 = 110.0;
const ALTEZZA_SCHEDA: f32 = 212.0;
/// Margine interno della scheda (per lato) più il bordo: 2 × (10 + 1).
const INGOMBRO_SCHEDA: f32 = 22.0;

/// Come sono mostrati i grafici a destra dell'elenco.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VistaSensori {
    /// Grafici impilati delle sonde, con asse del tempo condiviso.
    Confronto,
    /// Una scheda con il proprio grafico per ogni sensore selezionato.
    PerSensore,
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
        egui::Panel::left("sensori_sinistra")
            .resizable(false)
            .exact_size(360.0)
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.sensori_sinistra(ui));
            });
        egui::CentralPanel::default().show(ui, |ui| self.sensori_destra(ui));
    }

    // ------------------------------------------------------------------
    // Pannello sinistro: elenco, lettura, registrazione
    // ------------------------------------------------------------------

    fn sensori_sinistra(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Sensori").heading());
        if !self.collegato() {
            if self.live.serie.is_empty() {
                ui.label(
                    RichText::new(
                        "Non collegato: collega l'adattatore o avvia la demo dalla scheda \
                         «Connessione».",
                    )
                    .color(GIALLO),
                );
            } else {
                ui.label(
                    RichText::new(
                        "Non collegato: l'elenco mostra i parametri presenti nei dati già letti.",
                    )
                    .color(GRIGIO),
                );
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.ricerca)
                .hint_text("Cerca per nome…")
                .desired_width(ui.available_width()),
        );

        let disponibili = self.parametri_disponibili();
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
        ui.label(
            RichText::new(format!(
                "{} selezionati su {} disponibili",
                n_attivi,
                disponibili.len()
            ))
            .small()
            .color(GRIGIO),
        );

        let altezza_elenco = (ui.ctx().content_rect().height() * 0.50).clamp(240.0, 560.0);
        ScrollArea::vertical()
            .id_salt("sensori_elenco")
            .max_height(altezza_elenco)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if disponibili.is_empty() && self.collegato() {
                    ui.label(
                        RichText::new("La centralina non dichiara parametri noti all'app.")
                            .color(GRIGIO),
                    );
                } else if visibili.is_empty() && !disponibili.is_empty() {
                    ui.label(
                        RichText::new("Nessun sensore corrisponde alla ricerca.").color(GRIGIO),
                    );
                }
                for g in Gruppo::TUTTI {
                    let voci: Vec<&&Param> = visibili.iter().filter(|p| p.gruppo == g).collect();
                    if voci.is_empty() {
                        continue;
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new(g.nome()).strong().color(GRIGIO));
                    for p in voci {
                        if self.riga_sensore(ui, p) {
                            cambiata = true;
                        }
                    }
                }
            });
        ui.label(
            RichText::new("Hz = frequenza effettiva di aggiornamento negli ultimi 5 secondi.")
                .small()
                .color(GRIGIO),
        );
        if cambiata {
            self.selezione_cambiata();
        }

        self.sensori_lettura_e_registrazione(ui);
    }

    /// Una riga dell'elenco: spunta con il nome e, se attivo, valore e Hz.
    /// Restituisce `true` se la spunta è cambiata.
    fn riga_sensore(&mut self, ui: &mut egui::Ui, p: &Param) -> bool {
        let mut attivo = self.selezione.get(&p.key).copied().unwrap_or(false);
        let cambiata = ui.checkbox(&mut attivo, p.nome.as_str()).changed();
        if cambiata {
            self.selezione.insert(p.key.clone(), attivo);
        }
        if attivo {
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                match self.live.ultimo(&p.key) {
                    Some(pt) => match pt.v {
                        Some(v) => {
                            ui.label(RichText::new(valore_con_unita(&p.key, v, p.unita)).strong());
                        }
                        None => etichetta_stato(ui, pt.stato),
                    },
                    None => {
                        ui.label(RichText::new("in attesa").color(GRIGIO));
                    }
                }
                if let Some(hz) = self.live.frequenza(&p.key) {
                    ui.label(
                        RichText::new(format!("{} Hz", num_it(hz, 1)))
                            .small()
                            .color(GRIGIO),
                    );
                }
                if p.derivato {
                    ui.label(
                        RichText::new("calcolata dalla velocità")
                            .small()
                            .color(GRIGIO),
                    );
                }
            });
        }
        cambiata
    }

    fn sensori_lettura_e_registrazione(&mut self, ui: &mut egui::Ui) {
        if self.in_riproduzione() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Riproduzione: scorrono tutti i dati della sessione, la lettura non si avvia \
                     né si ferma. Le spunte scelgono che cosa mostrare.",
                )
                .color(ARANCIO_DEMO),
            );
            ui.add_space(8.0);
            ui.label(RichText::new("Registrazione").heading());
            ui.add_enabled(false, egui::Button::new("Avvia registrazione"))
                .on_disabled_hover_text(
                    "Durante la riproduzione non si registra: la sessione è già registrata.",
                );
            ui.label(RichText::new("Marcatori").strong());
            ui.label(
                RichText::new(
                    "Durante la riproduzione non si aggiungono marcatori: quelli registrati \
                     compaiono sulla barra di trasporto e nei grafici.",
                )
                .small()
                .color(GRIGIO),
            );
            return;
        }
        let ha_chiavi = !self.chiavi_polling().0.is_empty();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if self.polling {
                if ui
                    .add_enabled(
                        self.registrazione.is_none(),
                        egui::Button::new("Ferma lettura"),
                    )
                    .clicked()
                {
                    self.invia(Comando::FermaPolling);
                }
            } else if ui
                .add_enabled(
                    self.pronto() && ha_chiavi,
                    egui::Button::new("Avvia lettura"),
                )
                .on_disabled_hover_text("Seleziona almeno un sensore e collega l'adattatore")
                .clicked()
            {
                self.avvia_polling();
            }
        });

        ui.add_space(8.0);
        ui.label(RichText::new("Registrazione").heading());
        if let Some(r) = &self.registrazione {
            let s = (self.live.t_max - r.t_inizio).max(0.0) as u64;
            let nome = nome_file(&r.percorso);
            if ui
                .button(RichText::new("Ferma registrazione").color(ROSSO))
                .clicked()
            {
                self.invia(Comando::FermaRegistrazione);
            }
            ui.label(format!("In corso da {:02}:{:02}", s / 60, s % 60));
            ui.label(RichText::new(nome).small().color(GRIGIO));
        } else if ui
            .add_enabled(
                self.pronto() && (self.polling || ha_chiavi),
                egui::Button::new("Avvia registrazione"),
            )
            .clicked()
        {
            if !self.polling {
                self.avvia_polling();
            }
            self.invia(Comando::AvviaRegistrazione);
        }
        ui.label(
            RichText::new(
                "La registrazione prosegue anche con la finestra ridotta a icona. Durante la \
                 registrazione le sonde vengono lette più spesso degli altri parametri.",
            )
            .small()
            .color(GRIGIO),
        );

        ui.add_space(6.0);
        ui.label(RichText::new("Marcatori").strong());
        let attivi = self.collegato() && self.polling;
        ui.horizontal_wrapped(|ui| {
            for m in MARCATORI_RAPIDI {
                if ui.add_enabled(attivi, egui::Button::new(m)).clicked() {
                    self.invia(Comando::Marcatore(m.to_string()));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.testo_marcatore)
                    .hint_text("Altro marcatore…")
                    .desired_width(190.0),
            );
            let testo = self.testo_marcatore.trim().to_string();
            if ui
                .add_enabled(attivi && !testo.is_empty(), egui::Button::new("Aggiungi"))
                .clicked()
            {
                self.invia(Comando::Marcatore(testo));
                self.testo_marcatore.clear();
            }
        });

        if let Some(u) = &self.ultima {
            ui.add_space(8.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Ultima sessione salvata").strong());
                ui.label(format!("{} campioni", num_it(u.campioni as f64, 0)));
                ui.label(RichText::new(nome_file(&u.percorso)).small());
                match &u.csv {
                    Some(c) => ui.label(RichText::new(format!("CSV: {}", nome_file(c))).small()),
                    None => ui.label(RichText::new("CSV non scritto").small().color(ROSSO)),
                };
                if ui.button("Apri la cartella").clicked() {
                    apri_cartella(&self.cartella);
                }
            });
        }
    }

    // ------------------------------------------------------------------
    // Lato destro: vista e grafici
    // ------------------------------------------------------------------

    fn sensori_destra(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.vista, VistaSensori::Confronto, "Confronto");
            ui.selectable_value(
                &mut self.vista,
                VistaSensori::PerSensore,
                "Un grafico per sensore",
            );
            ui.separator();
            ui.label("Finestra:");
            for (valore, nome) in [
                (Some(30.0), "30 s"),
                (Some(60.0), "60 s"),
                (Some(120.0), "2 min"),
                (Some(300.0), "5 min"),
                (None, "Tutto"),
            ] {
                ui.selectable_value(&mut self.finestra, valore, nome);
            }
        });
        ui.horizontal_wrapped(|ui| match self.vista {
            VistaSensori::Confronto => {
                selettore_extra(ui, "live_extra", &mut self.extra, &self.live);
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
        ui.label(
            RichText::new(
                "I grafici servono a confrontare i segnali. L'app non deduce se catalizzatore o \
                 sonde siano guasti. Le fasce rosse indicano intervalli senza dato valido.",
            )
            .small()
            .color(GRIGIO),
        );
        match self.vista {
            VistaSensori::Confronto => self.sensori_confronto(ui),
            VistaSensori::PerSensore => self.sensori_griglia(ui),
        }
    }

    /// Confronto delle sonde e delle correzioni carburante (grafici impilati).
    fn sensori_confronto(&mut self, ui: &mut egui::Ui) {
        let disegnato = grafici::disegna(
            ui,
            &self.live,
            &Opzioni {
                id: "live",
                b1s1: self.preset.b1s1.as_deref(),
                b1s2: self.preset.b1s2.as_deref(),
                extra: self.extra.as_deref(),
                finestra: self.finestra,
                sintetico: self.live_demo,
            },
        );
        if !disegnato {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(if self.collegato() {
                    "Premi «Avvia lettura» per vedere i segnali."
                } else {
                    "Nessun dato: collega l'adattatore o avvia la demo."
                });
            });
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
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label("Seleziona almeno un sensore nell'elenco a sinistra.");
            });
            return;
        }
        let spazio = ui.spacing().item_spacing.x;
        // Un margine per la barra di scorrimento verticale.
        let disponibile = ui.available_width() - 16.0;
        let colonne =
            (((disponibile + spazio) / (LARGHEZZA_MIN_SCHEDA + spazio)).floor() as usize).max(1);
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
        });
    }

    /// Scheda di un sensore: nome, valore grande, statistiche, Hz e grafico.
    fn scheda_sensore(&self, ui: &mut egui::Ui, p: &Param, larghezza: f32) {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(10))
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
                    ui.add(egui::Label::new(RichText::new(nome).strong()).truncate());
                    ui.horizontal(|ui| {
                        match self.live.ultimo(&p.key) {
                            Some(pt) => match pt.v {
                                Some(v) => {
                                    ui.label(
                                        RichText::new(numero_param(&p.key, v)).size(26.0).strong(),
                                    );
                                    if !p.unita.is_empty() {
                                        ui.label(RichText::new(p.unita).color(GRIGIO));
                                    }
                                }
                                None => {
                                    ui.label(
                                        RichText::new(pt.stato.etichetta())
                                            .size(18.0)
                                            .color(colore_stato(pt.stato)),
                                    );
                                }
                            },
                            None => {
                                ui.label(RichText::new("in attesa").size(18.0).color(GRIGIO));
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let hz = match self.live.frequenza(&p.key) {
                                Some(hz) => format!("{} Hz", num_it(hz, 1)),
                                None => "— Hz".to_string(),
                            };
                            ui.label(RichText::new(hz).small().color(GRIGIO));
                        });
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
                        .color(GRIGIO),
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
