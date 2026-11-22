//! Le pagine Connessione e Diagnosi (Sensori, Sessioni e Impostazioni hanno
//! un modulo ciascuno).

use eframe::egui::{self, RichText, ScrollArea, Sense, vec2};

use super::componenti::{
    self, nota, paragrafo, pastiglia, primario, riga_valore, scheda, scheda_titolata,
};
use super::navigazione::{altezza_modale, larghezza_finestra};
use super::sessioni::AzioneSessioni;
use super::{App, Scheda, apri_cartella, colore_stato, etichetta_stato, tema};
use crate::diagnosis::{DiagnosisReport, Lettura, RigaDtc, StatoDtc};
use crate::elm::{ConfermaCancellazione, EsitoCancellazione, RispostaCancellazione};
use crate::model::{Stato, num_it};
use crate::obd::pids;
use crate::obd::readiness::Monitor;
use crate::riproduzione::data_leggibile;
use crate::summary::{self, valore_con_unita};
use crate::worker::{Comando, Sorgente, StatoConn};

const EXTRA: [&str; 5] = ["giri", "velocita", "temp_refrigerante", "carico", "map"];

pub(super) fn nome_parametro(key: &str) -> String {
    pids::per_key(key)
        .map(|p| p.nome.clone())
        .unwrap_or_else(|| key.to_string())
}

pub(super) fn nome_file(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Stati di un codice guasto in una riga: «confermato · in sospeso».
pub(super) fn testo_stati(stati: &[StatoDtc]) -> String {
    stati
        .iter()
        .map(|s| s.etichetta())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Le tre righe della scheda di un codice guasto sul layout stretto:
/// (codice, stati), descrizione, centralina.
pub(super) fn righe_scheda_dtc(x: &RigaDtc) -> ((String, String), String, String) {
    let descrizione = if x.descrizione.trim().is_empty() {
        "Descrizione non disponibile".to_string()
    } else {
        x.descrizione.clone()
    };
    (
        (x.codice.clone(), testo_stati(&x.stati)),
        descrizione,
        format!("ECU {}", x.ecu),
    )
}

/// Stato di un monitor di prontezza: testo e colore. Sul layout stretto il
/// testo è breve.
pub(super) fn stato_monitor(m: &Monitor, stretto: bool) -> (&'static str, egui::Color32) {
    if !m.disponibile {
        (
            if stretto {
                "Non disponibile"
            } else {
                "Non disponibile su questo veicolo"
            },
            tema::TESTO_3,
        )
    } else if m.completato {
        ("Completato", tema::OK)
    } else {
        ("Non completato", tema::ATTENZIONE)
    }
}

fn riga_lettura(ui: &mut egui::Ui, titolo: &str, l: &Lettura) {
    if l.stato == Stato::Ok && l.dettaglio.is_empty() {
        return;
    }
    componenti::riga_testo(ui, |ui| {
        ui.label(RichText::new(format!("{titolo}:")).color(tema::TESTO_2));
        etichetta_stato(ui, l.stato);
        if !l.dettaglio.is_empty() {
            ui.label(RichText::new(&l.dettaglio).color(tema::TESTO_2));
        }
    });
}

/// Righe etichetta/valore in una o due colonne (due solo sul largo, se le
/// righe sono abbastanza da valerne la pena).
fn righe_in_colonne(ui: &mut egui::Ui, stretto: bool, righe: &[(String, RichText)]) {
    if stretto || righe.len() < 6 || ui.available_width() < 640.0 {
        for (e, v) in righe {
            riga_valore(ui, stretto, e, v.clone());
        }
        return;
    }
    let meta = righe.len().div_ceil(2);
    ui.columns(2, |c| {
        for (e, v) in &righe[..meta] {
            riga_valore(&mut c[0], false, e, v.clone());
        }
        for (e, v) in &righe[meta..] {
            riga_valore(&mut c[1], false, e, v.clone());
        }
    });
}

/// Mostra un referto di diagnosi (dal vivo o da una sessione salvata).
pub(super) fn disegna_diagnosi(ui: &mut egui::Ui, d: &DiagnosisReport, stretto: bool) {
    // Codici guasto, con la spia MIL in testa.
    scheda_titolata(ui, "Codici guasto", |ui| {
        for p in &d.prontezza {
            componenti::riga_testo(ui, |ui| {
                if p.mil_accesa {
                    pastiglia(ui, "MIL ACCESA", tema::ATTENZIONE);
                } else {
                    pastiglia(ui, "MIL spenta", tema::OK);
                }
                ui.label(
                    RichText::new(format!(
                        "ECU {} · codici memorizzati dichiarati: {}",
                        p.ecu, p.numero_dtc
                    ))
                    .color(tema::TESTO_2),
                );
            });
        }
        riga_lettura(ui, "Spia MIL", &d.lettura_prontezza);
        ui.add_space(tema::SP_2);
        if d.dtc.is_empty() {
            ui.label("Nessun codice riportato dalle letture riuscite.");
        } else if stretto {
            for (i, x) in d.dtc.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                let ((codice, stati), descrizione, ecu) = righe_scheda_dtc(x);
                componenti::riga_testo(ui, |ui| {
                    ui.label(
                        RichText::new(codice)
                            .monospace()
                            .strong()
                            .color(tema::TESTO),
                    );
                    ui.label(RichText::new(stati).small().color(tema::TESTO_2));
                });
                ui.add(egui::Label::new(descrizione).wrap());
                nota(ui, ecu);
            }
        } else {
            // Tabella a colonne fisse, righe allineate in alto: la descrizione
            // prende lo spazio che resta e va a capo lì.
            let spazio = tema::SP_6;
            let (codice, ecu, stati) = (72.0, 48.0, 270.0);
            let descrizione =
                (ui.available_width() - codice - ecu - stati - 3.0 * spazio).max(200.0);
            let colonne = [codice, descrizione, ecu, stati];
            let riga = |ui: &mut egui::Ui, celle: [RichText; 4]| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = spazio;
                    for (w, t) in colonne.iter().zip(celle) {
                        ui.allocate_ui_with_layout(
                            egui::vec2(*w, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(*w);
                                ui.add(egui::Label::new(t).wrap());
                            },
                        );
                    }
                });
            };
            riga(
                ui,
                ["Codice", "Descrizione", "ECU", "Stati rilevati"]
                    .map(|t| RichText::new(t).color(tema::TESTO_2)),
            );
            for x in &d.dtc {
                riga(
                    ui,
                    [
                        RichText::new(&x.codice).strong().monospace(),
                        RichText::new(&x.descrizione),
                        RichText::new(&x.ecu),
                        RichText::new(testo_stati(&x.stati)),
                    ],
                );
            }
        }
        ui.add_space(tema::SP_1);
        riga_lettura(ui, "Codici confermati", &d.lettura_confermati);
        riga_lettura(ui, "Codici in sospeso", &d.lettura_in_sospeso);
        riga_lettura(ui, "Codici permanenti", &d.lettura_permanenti);
    });

    // Fermo immagine.
    scheda(ui, |ui| {
        ui.label(
            RichText::new("Fermo immagine")
                .heading()
                .color(tema::ACCENTO),
        );
        nota(
            ui,
            "Valori memorizzati dalla centralina nel momento in cui ha registrato il guasto. \
             Non sono dati in tempo reale.",
        );
        ui.add_space(tema::SP_2);
        let f = &d.fermo;
        if !f.presente {
            componenti::riga_testo(ui, |ui| {
                ui.label("Nessun fermo immagine disponibile.");
                if f.lettura.stato != Stato::Ok {
                    etichetta_stato(ui, f.lettura.stato);
                }
                ui.label(RichText::new(&f.lettura.dettaglio).color(tema::TESTO_2));
            });
            return;
        }
        componenti::riga_testo(ui, |ui| {
            ui.label(RichText::new("Generato da").color(tema::TESTO_2));
            ui.label(
                RichText::new(f.dtc.as_deref().unwrap_or("?"))
                    .strong()
                    .monospace(),
            );
            ui.label(RichText::new(format!("· ECU {}", f.ecu)).color(tema::TESTO_2));
        });
        ui.add_space(tema::SP_2);
        let righe: Vec<(String, RichText)> = f
            .righe
            .iter()
            .map(|r| {
                let valore = match (r.stato, r.valore, &r.testo) {
                    (Stato::Ok, Some(v), _) => {
                        RichText::new(valore_con_unita(&r.key, v, &r.unita)).strong()
                    }
                    (Stato::Ok, None, Some(t)) => RichText::new(t).strong(),
                    (Stato::Ok, None, None) => RichText::new("—").color(tema::TESTO_3),
                    (s, _, _) => {
                        RichText::new(format!("— {}", s.etichetta())).color(colore_stato(s))
                    }
                };
                (r.nome.clone(), valore)
            })
            .collect();
        righe_in_colonne(ui, stretto, &righe);
    });

    // Monitor di prontezza.
    scheda_titolata(ui, "Monitor di prontezza", |ui| {
        if d.prontezza.is_empty() {
            etichetta_stato(ui, d.lettura_prontezza.stato);
        }
        for p in &d.prontezza {
            nota(
                ui,
                format!(
                    "ECU {} · motore {}",
                    p.ecu,
                    if p.diesel {
                        "ad accensione spontanea"
                    } else {
                        "ad accensione comandata"
                    }
                ),
            );
            ui.add_space(tema::SP_1);
            let righe: Vec<(String, RichText)> = p
                .monitor
                .iter()
                .map(|m| {
                    let (t, c) = stato_monitor(m, stretto);
                    (m.nome.clone(), RichText::new(t).color(c))
                })
                .collect();
            righe_in_colonne(ui, stretto, &righe);
        }
    });

    // Mode 06.
    scheda_titolata(ui, "Monitor del catalizzatore (Mode 06)", |ui| {
        riga_lettura(ui, "Lettura", &d.mode06.lettura);
        if d.mode06.test.is_empty() {
            if d.mode06.lettura.stato == Stato::Ok {
                ui.label("Nessun test riportato per il MID 21.");
            }
            return;
        }
        let scalati = |t: &crate::obd::mode06::TestMonitor| -> [String; 3] {
            match &t.scalato {
                Some(s) => [
                    format!("{} {}", num_it(s.valore, 4), s.unita),
                    format!("{} {}", num_it(s.minimo, 4), s.unita),
                    format!("{} {}", num_it(s.massimo, 4), s.unita),
                ],
                None => ["solo grezzo".into(), String::new(), String::new()],
            }
        };
        let grezzo = |t: &crate::obd::mode06::TestMonitor| {
            format!(
                "{:04X} / {:04X} / {:04X}, scala {:02X}",
                t.grezzo_valore, t.grezzo_minimo, t.grezzo_massimo, t.uas_id
            )
        };
        if stretto {
            for (i, t) in d.mode06.test.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                ui.label(RichText::new(format!("ECU {} · test {:02X}", t.ecu, t.tid)).strong());
                let [v, mi, ma] = scalati(t);
                for (e, x) in [("Valore", v), ("Minimo", mi), ("Massimo", ma)] {
                    if !x.is_empty() {
                        riga_valore(ui, true, e, RichText::new(x));
                    }
                }
                nota(
                    ui,
                    format!("Dato grezzo (valore / min / max): {}", grezzo(t)),
                );
            }
        } else {
            egui::Grid::new("diag_m06")
                .num_columns(6)
                .spacing([tema::SP_6, tema::SP_2])
                .show(ui, |ui| {
                    for t in [
                        "ECU",
                        "Test (TID)",
                        "Valore",
                        "Minimo",
                        "Massimo",
                        "Dato grezzo (valore / min / max, scala)",
                    ] {
                        ui.label(RichText::new(t).color(tema::TESTO_2));
                    }
                    ui.end_row();
                    for t in &d.mode06.test {
                        ui.label(&t.ecu);
                        ui.label(format!("{:02X}", t.tid));
                        for x in scalati(t) {
                            ui.label(x);
                        }
                        ui.label(RichText::new(grezzo(t)).monospace());
                        ui.end_row();
                    }
                });
        }
        ui.add_space(tema::SP_2);
        nota(
            ui,
            "MID 21, catalizzatore bancata 1. Le unità derivano dalla tabella «Unit and \
             Scaling ID» ripresa dal progetto python-OBD, non confrontata con il testo SAE \
             J1979: fa fede il dato grezzo. Il significato dei TID da 80 in su è definito dal \
             costruttore. L'app non giudica l'esito del test.",
        );
    });

    // Dettagli della lettura.
    scheda_titolata(ui, "Dettagli della lettura", |ui| {
        let mut righe: Vec<(String, RichText)> = vec![
            (
                "Lettura del".into(),
                RichText::new(data_leggibile(&d.letto_il)),
            ),
            (
                "Adattatore".into(),
                RichText::new(format!("{} — {}", d.adattatore, d.collegamento)),
            ),
            ("Protocollo".into(), RichText::new(&d.protocollo)),
            ("Centraline".into(), RichText::new(d.ecu.join(", "))),
        ];
        if let Some(t) = &d.tensione {
            righe.push((
                "Tensione al connettore".into(),
                RichText::new(format!("{t} (misurata dall'adattatore)")),
            ));
        }
        if let Some(c) = &d.tipo_carburante {
            righe.push((
                "Tipo di carburante".into(),
                RichText::new(format!("{c} (dichiarato dalla centralina)")),
            ));
        }
        if let Some(n) = &d.norma_obd {
            righe.push((
                "Norma OBD".into(),
                RichText::new(format!("{n} (dichiarata dalla centralina)")),
            ));
        }
        for (e, v) in righe {
            riga_valore(ui, stretto, &e, v);
        }
        ui.add_space(tema::SP_1);
        ui.collapsing("Parametri disponibili (PID del servizio 01)", |ui| {
            for (ecu, lista) in &d.pid_supportati {
                let testo: Vec<String> = lista.iter().map(|p| format!("{p:02X}")).collect();
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("ECU {ecu}: {}", testo.join(" "))).monospace(),
                    )
                    .wrap(),
                );
            }
        });
    });
}

impl App {
    // ------------------------------------------------------------------
    // Connessione
    // ------------------------------------------------------------------

    pub(crate) fn pagina_connessione(&mut self, ui: &mut egui::Ui) {
        let stretto = self.disposizione.stretta();
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.collegato() {
                    self.connessione_attiva(ui, stretto);
                } else {
                    self.connessione_scelta(ui, stretto);
                }
                if let Some(g) = self.gatt_sconosciuto.clone() {
                    scheda_titolata(ui, "Profilo BLE non riconosciuto", |ui| {
                        paragrafo(
                            ui,
                            "Nessuna scrittura è stata eseguita. Copia questo elenco per \
                             verificare quale profilo usa l'adattatore.",
                        );
                        let testo = g.testo();
                        if ui.button("Copia elenco UUID").clicked() {
                            ui.ctx().copy_text(testo.clone());
                        }
                        ui.add(egui::Label::new(RichText::new(testo).monospace()).wrap());
                    });
                }
                scheda(ui, |ui| {
                    egui::CollapsingHeader::new(
                        RichText::new("Registro degli scambi con l'adattatore").color(tema::TESTO),
                    )
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Copia registro").clicked() {
                                ui.ctx().copy_text(self.registro.join("\n"));
                            }
                            if ui.button("Svuota").clicked() {
                                self.registro.clear();
                            }
                        });
                        nota(ui, "La risposta al VIN non viene mai registrata.");
                        ScrollArea::vertical()
                            .id_salt("registro")
                            .max_height(260.0)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for r in &self.registro {
                                    ui.add(
                                        egui::Label::new(RichText::new(r).monospace().small())
                                            .wrap(),
                                    );
                                }
                                if self.registro.is_empty() {
                                    nota(ui, "Ancora nessuno scambio: collega l'adattatore.");
                                }
                            });
                    });
                });
            });
    }

    fn connessione_attiva(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let Some(info) = self.info.clone() else {
            return;
        };
        let riproduzione = info.riproduzione.clone();
        let titolo = if riproduzione.is_some() {
            "Riproduzione in corso"
        } else {
            "Collegamento attivo"
        };
        scheda_titolata(ui, titolo, |ui| {
            let mut righe: Vec<(&str, String)> = vec![("Collegamento", info.collegamento.clone())];
            if let Some(r) = &riproduzione {
                righe.push(("Dati registrati il", data_leggibile(&r.registrata_il)));
            }
            righe.push(("Adattatore", info.adattatore.clone()));
            righe.push(("Protocollo", info.protocollo.clone()));
            righe.push(("Centraline che rispondono", info.ecu.join(", ")));
            if riproduzione.is_some() {
                righe.push((
                    "Parametri nella sessione",
                    format!(
                        "{} PID del servizio 01{}",
                        info.unione.len(),
                        if info.liberi.is_empty() {
                            String::new()
                        } else {
                            format!(", {} parametri liberi", info.liberi.len())
                        }
                    ),
                ));
            } else {
                righe.push((
                    "Parametri dichiarati",
                    format!("{} PID del servizio 01", info.unione.len()),
                ));
            }
            if let Some(g) = &info.gatt {
                righe.push((
                    "Profilo BLE",
                    format!(
                        "{} — fonte: {}",
                        g.profilo.as_deref().unwrap_or("?"),
                        g.fonte_profilo.as_deref().unwrap_or("?")
                    ),
                ));
            }
            for (e, v) in righe {
                riga_valore(ui, stretto, e, RichText::new(v));
            }
            for a in &info.avvisi {
                ui.add(egui::Label::new(RichText::new(a).color(tema::ATTENZIONE)).wrap());
            }
            ui.add_space(tema::SP_3);
            let mut azioni = |ui: &mut egui::Ui| {
                if componenti::pulsante(ui, stretto, true, primario("Vai al cruscotto")).clicked() {
                    self.scheda = Scheda::Cruscotto;
                }
                if riproduzione.is_none()
                    && componenti::pulsante(
                        ui,
                        stretto,
                        true,
                        egui::Button::new("Vai alla diagnosi"),
                    )
                    .clicked()
                {
                    self.scheda = Scheda::Diagnosi;
                }
                if componenti::pulsante(ui, stretto, true, egui::Button::new("Vai ai sensori"))
                    .clicked()
                {
                    self.scheda = Scheda::Sensori;
                }
                let scollega = if riproduzione.is_some() {
                    "Chiudi riproduzione"
                } else {
                    "Scollega"
                };
                if componenti::pulsante(ui, stretto, true, egui::Button::new(scollega)).clicked() {
                    self.invia(Comando::Scollega);
                }
            };
            if stretto {
                azioni(ui);
            } else {
                ui.horizontal_wrapped(|ui| azioni(ui));
            }
            if let Some(g) = &info.gatt {
                ui.add_space(tema::SP_2);
                ui.collapsing("Servizi GATT del dispositivo", |ui| {
                    ui.add(egui::Label::new(RichText::new(g.testo()).monospace().small()).wrap());
                });
            }
        });
    }

    fn connessione_scelta(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let libero = matches!(self.stato, StatoConn::Scollegato | StatoConn::Scansione);
        if let StatoConn::Connessione(fase) = &self.stato {
            let fase = fase.clone();
            scheda(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(fase).color(tema::ATTENZIONE));
                });
            });
        }
        paragrafo(
            ui,
            "L'app legge dati diagnostici standard. L'unica eccezione è «Cancella codici \
             guasto», che chiede sempre una conferma esplicita. Non scrive sulle centraline, \
             non esegue codifiche né test attuatori e non permette di inviare comandi liberi.",
        );
        ui.add_space(tema::SP_3);
        if stretto || ui.available_width() < 760.0 {
            self.scheda_ble(ui, true, libero);
            self.scheda_seriale(ui, true, libero);
            self.scheda_demo(ui, true, libero);
            self.scheda_riproduzione(ui, true, libero);
        } else {
            ui.columns(2, |c| {
                self.scheda_ble(&mut c[0], false, libero);
                self.scheda_seriale(&mut c[0], false, libero);
                self.scheda_demo(&mut c[1], false, libero);
                self.scheda_riproduzione(&mut c[1], false, libero);
            });
        }
    }

    /// Bluetooth Low Energy: ricerca, elenco toccabile, collegamento.
    fn scheda_ble(&mut self, ui: &mut egui::Ui, pieno: bool, libero: bool) {
        scheda_titolata(ui, "Bluetooth Low Energy", |ui| {
            paragrafo(
                ui,
                "Inserisci l'adattatore nella presa OBD con il quadro acceso e chiudi sul \
                 telefono le app collegate all'adattatore. Non serve associarlo in Windows.",
            );
            ui.add_space(tema::SP_2);
            let scelto = self.ble_scelto.clone();
            if self.stato == StatoConn::Scansione {
                ui.horizontal(|ui| {
                    if ui.button("Ferma ricerca").clicked() {
                        self.invia(Comando::FermaScansione);
                    }
                    ui.spinner();
                    ui.label(RichText::new("Ricerca in corso…").color(tema::TESTO_2));
                });
            } else {
                let testo = "Cerca dispositivi";
                let b = if scelto.is_none() {
                    primario(testo)
                } else {
                    egui::Button::new(testo)
                };
                if componenti::pulsante(ui, pieno, libero, b).clicked() {
                    self.dispositivi.clear();
                    self.ble_scelto = None;
                    self.invia(Comando::ScansionaBle);
                }
            }
            ui.checkbox(
                &mut self.mostra_senza_nome,
                "Mostra anche i dispositivi senza nome",
            );
            let visibili: Vec<_> = self
                .dispositivi
                .iter()
                .filter(|d| self.mostra_senza_nome || !d.nome.is_empty())
                .cloned()
                .collect();
            if !visibili.is_empty() {
                ui.add_space(tema::SP_2);
                ScrollArea::vertical()
                    .id_salt("ble")
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for d in &visibili {
                            if self.riga_dispositivo(ui, d) {
                                self.ble_scelto = Some(d.indirizzo.clone());
                            }
                        }
                    });
            } else if self.stato == StatoConn::Scansione {
                nota(ui, "Nessun dispositivo trovato finora.");
            }
            ui.add_space(tema::SP_2);
            let b = egui::Button::new("Collega il dispositivo selezionato");
            let b = if scelto.is_some() {
                primario("Collega il dispositivo selezionato")
            } else {
                b
            };
            if componenti::pulsante(ui, pieno, libero && scelto.is_some(), b).clicked()
                && let Some(indirizzo) = scelto
            {
                self.errore = None;
                self.gatt_sconosciuto = None;
                self.invia(Comando::Collega(Sorgente::Ble(indirizzo)));
            }
        });
    }

    /// Riga toccabile di un dispositivo BLE: nome e «OBD» sopra, indirizzo e
    /// segnale sotto. Restituisce `true` se è stata scelta.
    fn riga_dispositivo(
        &self,
        ui: &mut egui::Ui,
        d: &crate::transport::ble::BleDispositivo,
    ) -> bool {
        let scelto = self.ble_scelto.as_deref() == Some(&d.indirizzo);
        let alto = tema::BERSAGLIO_STRETTO + tema::SP_1;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), alto), Sense::click());
        let p = ui.painter();
        let raggio = egui::CornerRadius::same(tema::RAGGIO_CONTROLLO);
        if scelto {
            p.rect(
                rect,
                raggio,
                tema::velatura(tema::ACCENTO, 0.14),
                egui::Stroke::new(tema::BORDO_SOTTILE, tema::ACCENTO),
                egui::StrokeKind::Inside,
            );
        } else if resp.hovered() {
            p.rect_filled(rect, raggio, tema::SUPERFICIE_ALTA);
        }
        let nome = if d.nome.is_empty() {
            "(senza nome)"
        } else {
            d.nome.as_str()
        };
        let x = rect.left() + tema::SP_3;
        let galley = p.layout_no_wrap(
            nome.to_string(),
            tema::font_testo(tema::CARATTERE_CORPO),
            if scelto { tema::ACCENTO } else { tema::TESTO },
        );
        let larghezza_nome = galley.size().x;
        p.galley(
            egui::pos2(x, rect.top() + tema::SP_1 + 2.0),
            galley,
            tema::TESTO,
        );
        if d.probabile_obd {
            let r = egui::Rect::from_min_size(
                egui::pos2(
                    x + larghezza_nome + tema::SP_2,
                    rect.top() + tema::SP_1 + 3.0,
                ),
                vec2(34.0, 18.0),
            );
            p.rect_filled(r, raggio, tema::velatura(tema::OK, 0.16));
            p.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "OBD",
                tema::font_testo(tema::CARATTERE_NOTA),
                tema::OK,
            );
        }
        let segnale = d
            .rssi
            .map(|r| format!("{r} dBm"))
            .unwrap_or_else(|| "segnale n.d.".into());
        p.text(
            egui::pos2(x, rect.bottom() - tema::SP_1 - 2.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{} · {segnale}", d.indirizzo),
            tema::font_testo(tema::CARATTERE_NOTA),
            tema::TESTO_2,
        );
        ui.add_space(tema::SP_1);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    }

    /// Porta seriale (Bluetooth Classic/SPP o USB).
    fn scheda_seriale(&mut self, ui: &mut egui::Ui, pieno: bool, libero: bool) {
        if !crate::piattaforma::SERIALE_DISPONIBILE {
            scheda_titolata(ui, "Porta seriale", |ui| {
                paragrafo(ui, crate::piattaforma::NOTA_SERIALE_ASSENTE);
            });
            return;
        }
        scheda_titolata(ui, "Porta seriale", |ui| {
            paragrafo(
                ui,
                "Bluetooth Classic (SPP) o USB. Per gli adattatori associati in Windows come \
                 dispositivo Bluetooth classico: scegli la porta COM «in uscita» creata \
                 dall'associazione.",
            );
            ui.add_space(tema::SP_2);
            let largo_combo = if pieno { ui.available_width() } else { 220.0 };
            let mut campi = |ui: &mut egui::Ui| {
                egui::ComboBox::from_id_salt("porta")
                    .width(largo_combo)
                    .selected_text(if self.porta_scelta.is_empty() {
                        "Nessuna porta COM".to_string()
                    } else {
                        self.porta_scelta.clone()
                    })
                    .show_ui(ui, |ui| {
                        for p in self.porte.clone() {
                            ui.selectable_value(
                                &mut self.porta_scelta,
                                p.nome.clone(),
                                format!("{} — {}", p.nome, p.tipo),
                            );
                        }
                    });
                egui::ComboBox::from_id_salt("baud")
                    .width(if pieno { largo_combo } else { 110.0 })
                    .selected_text(format!("{} baud", self.baud))
                    .show_ui(ui, |ui| {
                        for b in [38400u32, 9600, 115200] {
                            ui.selectable_value(&mut self.baud, b, format!("{b} baud"));
                        }
                    });
            };
            if pieno {
                campi(ui);
            } else {
                ui.horizontal(|ui| campi(ui));
            }
            let mut pulsanti = |ui: &mut egui::Ui| {
                if componenti::pulsante(ui, pieno, true, egui::Button::new("Aggiorna elenco"))
                    .clicked()
                {
                    self.invia(Comando::ElencaSeriali);
                }
                if componenti::pulsante(
                    ui,
                    pieno,
                    libero && !self.porta_scelta.is_empty(),
                    egui::Button::new("Collega via seriale"),
                )
                .clicked()
                {
                    self.errore = None;
                    self.invia(Comando::Collega(Sorgente::Seriale {
                        porta: self.porta_scelta.clone(),
                        baud: self.baud,
                    }));
                }
            };
            if pieno {
                pulsanti(ui);
            } else {
                ui.horizontal(|ui| pulsanti(ui));
            }
        });
    }

    fn scheda_demo(&mut self, ui: &mut egui::Ui, pieno: bool, libero: bool) {
        scheda_titolata(ui, "Modalità demo", |ui| {
            paragrafo(
                ui,
                "Prova l'app senza veicolo: un simulatore risponde come un ELM327 collegato a \
                 un motore benzina con P0420. I dati sono sintetici.",
            );
            ui.add_space(tema::SP_2);
            if componenti::pulsante(ui, pieno, libero, egui::Button::new("Avvia la demo")).clicked()
            {
                self.errore = None;
                self.invia(Comando::Collega(Sorgente::Demo));
            }
        });
    }

    fn scheda_riproduzione(&mut self, ui: &mut egui::Ui, pieno: bool, libero: bool) {
        scheda_titolata(ui, "Riproduzione di una sessione", |ui| {
            paragrafo(
                ui,
                "Rivivi una sessione registrata o importata: i dati scorrono nel tempo su \
                 cruscotto, sensori, grafici e MQTT, con pausa, posizione e velocità. Non si \
                 registra e non si cancellano codici.",
            );
            ui.add_space(tema::SP_2);
            let mut pulsanti = |ui: &mut egui::Ui| {
                if componenti::pulsante(
                    ui,
                    pieno,
                    libero,
                    egui::Button::new("Riproduci una sessione…"),
                )
                .clicked()
                {
                    self.apri_scelta_sessioni(AzioneSessioni::Riproduci);
                }
                if componenti::pulsante(ui, pieno, true, egui::Button::new("Vai alle sessioni"))
                    .clicked()
                {
                    self.scheda = Scheda::Sessioni;
                }
            };
            if pieno {
                pulsanti(ui);
            } else {
                ui.horizontal_wrapped(|ui| pulsanti(ui));
            }
        });
    }

    // ------------------------------------------------------------------
    // Diagnosi
    // ------------------------------------------------------------------

    pub(crate) fn pagina_diagnosi(&mut self, ui: &mut egui::Ui) {
        let stretto = self.disposizione.stretta();
        let riproduzione = self.in_riproduzione();
        let pronto = self.pronto() && !riproduzione;
        let in_riproduzione = "Durante la riproduzione non c'è una centralina collegata: si vede \
                               la diagnosi registrata nella sessione.";
        let leggi = |app: &mut App, ui: &mut egui::Ui| {
            let r = componenti::pulsante(ui, stretto, pronto, primario("Leggi diagnosi"))
                .on_disabled_hover_text(if riproduzione {
                    in_riproduzione
                } else {
                    "Collega l'adattatore o avvia la demo"
                });
            if r.clicked() {
                app.invia(Comando::LeggiDiagnosi);
            }
        };
        let in_corso = |app: &App, ui: &mut egui::Ui| {
            if app.stato == StatoConn::Diagnosi {
                ui.spinner();
                ui.label(RichText::new("Lettura in corso…").color(tema::TESTO_2));
            }
            if app.stato == StatoConn::Cancellazione {
                ui.spinner();
                ui.label(RichText::new("Cancellazione in corso…").color(tema::PERICOLO));
            }
        };
        if stretto {
            leggi(self, ui);
            ui.horizontal_wrapped(|ui| {
                in_corso(self, ui);
                if let Some(d) = &self.diagnosi {
                    if ui.button("Copia riepilogo").clicked() {
                        ui.ctx().copy_text(summary::riepilogo(d, self.includi_vin));
                    }
                    ui.checkbox(&mut self.includi_vin, "Con il VIN");
                }
            });
        } else {
            ui.horizontal(|ui| {
                leggi(self, ui);
                in_corso(self, ui);
                if let Some(d) = &self.diagnosi {
                    if ui.button("Copia riepilogo").clicked() {
                        ui.ctx().copy_text(summary::riepilogo(d, self.includi_vin));
                    }
                    ui.checkbox(&mut self.includi_vin, "Includi il VIN nel riepilogo");
                }
                // Azione distruttiva: all'estremo opposto di «Leggi diagnosi».
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.pulsante_cancella(ui, false, pronto, riproduzione) {
                        self.conferma_cancellazione = Some(false);
                    }
                });
            });
        }
        ui.add_space(tema::SP_3);
        if riproduzione {
            ui.add(egui::Label::new(RichText::new(in_riproduzione).color(tema::DEMO)).wrap());
        }
        self.disegna_esito_cancellazione(ui, stretto);
        if !self.collegato() && self.diagnosi.is_some() {
            ui.add(
                egui::Label::new(
                    RichText::new("Adattatore non collegato: questa è l'ultima lettura eseguita.")
                        .color(tema::ATTENZIONE),
                )
                .wrap(),
            );
        }
        let mut area = ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(y) = self.scorri_diagnosi.take() {
            area = area.vertical_scroll_offset(y);
        }
        area.show(ui, |ui| {
            match &self.diagnosi {
                Some(d) => {
                    disegna_diagnosi(ui, d, stretto);
                    if let Some(v) = &d.vin {
                        scheda(ui, |ui| {
                            ui.collapsing("VIN (non viene mai salvato né esportato)", |ui| {
                                ui.label(RichText::new(v).monospace());
                            });
                        });
                    }
                }
                None => {
                    let collegato = self.collegato();
                    let vai = componenti::stato_vuoto(
                        ui,
                        "Nessuna diagnosi letta",
                        if collegato {
                            "Premi «Leggi diagnosi» per leggere codici guasto, spia MIL, \
                             prontezza e fermo immagine."
                        } else {
                            "Collega l'adattatore o avvia la demo, poi leggi la diagnosi."
                        },
                        (!collegato).then_some("Vai a Connessione"),
                    );
                    if vai {
                        self.scheda = Scheda::Connessione;
                    }
                }
            }
            // Sullo stretto l'azione distruttiva sta in fondo, lontano da
            // «Leggi diagnosi».
            if stretto {
                ui.add_space(tema::SP_4);
                let mut apri = false;
                componenti::cornice_scheda()
                    .stroke(egui::Stroke::new(
                        tema::BORDO_SOTTILE,
                        tema::velatura(tema::PERICOLO, 0.5),
                    ))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new("Cancellare i codici guasto")
                                .heading()
                                .color(tema::PERICOLO),
                        );
                        nota(
                            ui,
                            "Modifica lo stato della centralina e non si può annullare. Prima \
                             di cancellare l'app salva una copia della diagnosi.",
                        );
                        ui.add_space(tema::SP_2);
                        apri = self.pulsante_cancella(ui, true, pronto, riproduzione);
                    });
                if apri {
                    self.conferma_cancellazione = Some(false);
                }
                ui.add_space(tema::SP_6);
            }
        });
    }

    /// Pulsante «Cancella codici guasto…»; `true` se premuto.
    fn pulsante_cancella(
        &self,
        ui: &mut egui::Ui,
        pieno: bool,
        pronto: bool,
        riproduzione: bool,
    ) -> bool {
        componenti::pulsante(
            ui,
            pieno,
            pronto,
            componenti::pericolo("Cancella codici guasto…"),
        )
        .on_hover_text(
            "Cancella codici memorizzati e in sospeso, fermo immagine e prontezza. Chiede \
             conferma.",
        )
        .on_disabled_hover_text(if riproduzione {
            "Durante la riproduzione i codici non si cancellano: nessuna centralina è collegata."
        } else {
            "Collega l'adattatore o avvia la demo"
        })
        .clicked()
    }

    /// Esito dell'ultima cancellazione, in chiaro, sopra la diagnosi riletta.
    fn disegna_esito_cancellazione(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let Some(r) = self.esito_cancellazione.clone() else {
            return;
        };
        let (titolo, colore, spiegazione) = match &r.esito {
            EsitoCancellazione::Risposte(v) => {
                let accettate = v
                    .iter()
                    .filter(|e| e.risposta == RispostaCancellazione::Accettata)
                    .count();
                if accettate == v.len() {
                    (
                        "Codici guasto cancellati",
                        tema::OK,
                        "La diagnosi qui sotto è stata riletta dopo la cancellazione. I codici \
                         permanenti restano finché la centralina non li elimina da sola; i \
                         monitor di prontezza si completano nei prossimi cicli di guida."
                            .to_string(),
                    )
                } else if accettate > 0 {
                    (
                        "Cancellazione eseguita solo da alcune centraline",
                        tema::ATTENZIONE,
                        "La diagnosi qui sotto è stata riletta dopo la cancellazione.".to_string(),
                    )
                } else {
                    (
                        "Cancellazione rifiutata dalla centralina",
                        tema::PERICOLO,
                        "I codici non sono stati cancellati. La diagnosi qui sotto è stata \
                         riletta."
                            .to_string(),
                    )
                }
            }
            EsitoCancellazione::NessunaRisposta => (
                "La centralina non ha risposto alla cancellazione",
                tema::PERICOLO,
                "Probabilmente i codici non sono stati cancellati: controlla la diagnosi \
                 riletta qui sotto e verifica che il quadro sia acceso."
                    .to_string(),
            ),
            EsitoCancellazione::ErroreComunicazione(d) => (
                "Errore durante la cancellazione",
                tema::PERICOLO,
                format!(
                    "Non è possibile sapere se i codici sono stati cancellati: controlla la \
                     diagnosi riletta. Dettaglio: {d}"
                ),
            ),
        };
        let mut chiudi = false;
        componenti::cornice_scheda()
            .stroke(egui::Stroke::new(tema::BORDO_SOTTILE, colore))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                componenti::riga_testo(ui, |ui| {
                    ui.label(RichText::new(titolo).heading().color(colore));
                    if r.demo {
                        pastiglia(ui, "SIMULATA — modalità demo", tema::DEMO);
                    }
                });
                if let EsitoCancellazione::Risposte(v) = &r.esito {
                    for e in v {
                        let c = if e.risposta == RispostaCancellazione::Accettata {
                            tema::OK
                        } else {
                            tema::PERICOLO
                        };
                        ui.label(RichText::new(format!("ECU {}: {}", e.ecu, e.testo())).color(c));
                    }
                }
                ui.add(egui::Label::new(spiegazione).wrap());
                nota(
                    ui,
                    format!("Eseguita il {}", data_leggibile(&r.eseguita_il)),
                );
                nota(
                    ui,
                    format!("Copia della diagnosi precedente: {}", r.copia.display()),
                );
                ui.add_space(tema::SP_2);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Apri la cartella").clicked() {
                        apri_cartella(r.copia.parent().unwrap_or(&self.cartella));
                    }
                    if ui.button("Chiudi").clicked() {
                        chiudi = true;
                    }
                });
            });
        if chiudi {
            self.esito_cancellazione = None;
        }
        let _ = stretto;
        ui.add_space(tema::SP_3);
    }

    /// Finestra di conferma della cancellazione (docs/DESIGN.md, sezione 5).
    pub(crate) fn finestra_cancellazione(&mut self, ctx: &egui::Context) {
        let Some(mut spunta) = self.conferma_cancellazione else {
            return;
        };
        if !self.collegato() {
            self.conferma_cancellazione = None;
            return;
        }
        let stretto = self.disposizione.stretta();
        let demo = self.info.as_ref().is_some_and(|i| i.demo);
        let pronto = self.pronto();
        let mut annulla = false;
        let mut conferma = false;
        let punto = |ui: &mut egui::Ui, testo: &str| {
            ui.horizontal_top(|ui| {
                ui.add_space(tema::SP_1);
                let (r, _) = ui.allocate_exact_size(vec2(tema::SP_2, tema::SP_4), Sense::hover());
                ui.painter().circle_filled(r.center(), 2.5, tema::TESTO_2);
                ui.add(egui::Label::new(testo).wrap());
            });
        };
        let largo = larghezza_finestra(ctx, 580.0);
        // Titolo, avvertenza, spunta e pulsanti restano sempre visibili; scorre
        // solo la spiegazione.
        let alto = altezza_modale(ctx, if stretto { 330.0 } else { 260.0 });
        let risposta = egui::Modal::new(egui::Id::new("conferma_cancellazione"))
            .frame(componenti::cornice_finestra(ctx))
            .backdrop_color(egui::Color32::from_black_alpha(170))
            .show(ctx, |ui| {
                ui.set_width(largo);
                ui.label(
                    RichText::new("Cancellare i codici guasto?")
                        .heading()
                        .color(tema::PERICOLO),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Questa operazione modifica lo stato della centralina e non si può \
                             annullare.",
                        )
                        .strong(),
                    )
                    .wrap(),
                );
                if demo {
                    ui.add_space(tema::SP_2);
                    pastiglia(
                        ui,
                        "MODALITÀ DEMO — si cancella sul simulatore, non su un veicolo",
                        tema::DEMO,
                    );
                }
                ui.add_space(tema::SP_2);
                ScrollArea::vertical()
                    .id_salt("conferma_cancellazione_testo")
                    .max_height(alto)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Verranno cancellati").strong());
                        punto(ui, "i codici guasto memorizzati (confermati) e in sospeso;");
                        punto(ui, "il fermo immagine;");
                        punto(ui, "i risultati dei monitor (Mode 06);");
                        punto(
                            ui,
                            "lo stato di prontezza: i monitor tornano «non completato»; si \
                             azzerano anche distanza e tempo dalla cancellazione. La spia MIL \
                             si spegne.",
                        );
                        ui.add_space(tema::SP_2);
                        ui.label(RichText::new("Non verranno cancellati").strong());
                        punto(
                            ui,
                            "i codici permanenti: li elimina solo la centralina, dopo alcuni \
                             cicli di guida senza che il guasto si ripresenti.",
                        );
                        ui.add_space(tema::SP_2);
                        ui.label(RichText::new("Condizioni richieste").strong());
                        punto(
                            ui,
                            "quadro acceso e motore spento. Con il motore acceso molte \
                             centraline rifiutano la cancellazione.",
                        );
                        ui.add_space(tema::SP_2);
                        ui.label(RichText::new("Revisione").strong().color(tema::ATTENZIONE));
                        ui.add(
                            egui::Label::new(
                                "Finché i monitor di prontezza non tornano «completato» (servono \
                                 uno o più cicli di guida) il veicolo può non superare la \
                                 revisione. Cancellare i codici non ripara il guasto: se la \
                                 causa resta, il codice ricompare.",
                            )
                            .wrap(),
                        );
                        ui.add_space(tema::SP_2);
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "Prima di cancellare l'app salva una copia della diagnosi \
                                     attuale, fermo immagine compreso, nella cartella delle \
                                     sessioni.",
                                )
                                .color(tema::TESTO_2),
                            )
                            .wrap(),
                        );
                    });
                ui.add_space(tema::SP_3);
                ui.checkbox(&mut spunta, "Ho letto e voglio cancellare i codici");
                ui.add_space(tema::SP_3);
                let mut pulsanti = |ui: &mut egui::Ui| {
                    let r_annulla = if stretto {
                        componenti::pulsante(ui, true, true, egui::Button::new("Annulla"))
                    } else {
                        ui.add(egui::Button::new("Annulla").min_size(vec2(120.0, 0.0)))
                    };
                    // Pulsante predefinito: riceve il fuoco, Invio annulla.
                    if ui.memory(|m| m.focused().is_none()) {
                        r_annulla.request_focus();
                    }
                    if r_annulla.clicked() {
                        annulla = true;
                    }
                    ui.add_space(if stretto { tema::SP_2 } else { tema::SP_4 });
                    let pulsante = componenti::pericolo("Cancella codici guasto");
                    let r = if stretto {
                        componenti::pulsante(ui, true, spunta && pronto, pulsante)
                    } else {
                        ui.add_enabled(spunta && pronto, pulsante.min_size(vec2(180.0, 0.0)))
                    };
                    if r.clicked() {
                        conferma = true;
                    }
                };
                if stretto {
                    pulsanti(ui);
                } else {
                    ui.horizontal(|ui| pulsanti(ui));
                }
                if !pronto {
                    nota(ui, "Attendi la fine dell'operazione in corso.");
                }
            });
        if conferma && spunta && pronto {
            self.conferma_cancellazione = None;
            self.errore = None;
            self.esito_cancellazione = None;
            self.invia(Comando::CancellaDtc(
                ConfermaCancellazione::utente_ha_confermato(),
            ));
        } else if annulla || risposta.should_close() {
            self.conferma_cancellazione = None;
        } else {
            self.conferma_cancellazione = Some(spunta);
        }
    }
}

/// Scelta del parametro aggiuntivo da tracciare.
pub(super) fn selettore_extra(
    ui: &mut egui::Ui,
    id: &str,
    scelta: &mut Option<String>,
    archivio: &super::dati::Archivio,
    larghezza: Option<f32>,
) {
    ui.label(RichText::new("Grafico aggiuntivo").color(tema::TESTO_2));
    // Senza larghezza la casella occupa il resto della riga.
    let larghezza = larghezza.unwrap_or_else(|| ui.available_width());
    egui::ComboBox::from_id_salt(id)
        .width(larghezza)
        .selected_text(match scelta {
            Some(k) => nome_parametro(k),
            None => "Nessuno".to_string(),
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(scelta, None, "Nessuno");
            for k in EXTRA {
                if archivio.serie.contains_key(k) {
                    ui.selectable_value(scelta, Some(k.to_string()), nome_parametro(k));
                }
            }
            // Poi tutti gli altri parametri presenti, liberi compresi.
            for k in archivio.serie.keys() {
                if !EXTRA.contains(&k.as_str()) {
                    ui.selectable_value(scelta, Some(k.clone()), nome_parametro(k));
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheda_dtc_sul_layout_stretto() {
        let x = RigaDtc {
            codice: "P0420".into(),
            descrizione: "Efficienza del catalizzatore sotto la soglia, bancata 1".into(),
            ecu: "7E8".into(),
            stati: vec![
                StatoDtc::Confermato,
                StatoDtc::InSospeso,
                StatoDtc::Permanente,
            ],
        };
        let ((codice, stati), descrizione, ecu) = righe_scheda_dtc(&x);
        assert_eq!(codice, "P0420");
        assert_eq!(stati, "confermato · in sospeso · permanente");
        assert!(descrizione.starts_with("Efficienza"));
        assert_eq!(ecu, "ECU 7E8");

        // Senza descrizione lo si dice, senza lasciare una riga vuota.
        let y = RigaDtc {
            descrizione: "  ".into(),
            stati: vec![StatoDtc::Permanente],
            ..x
        };
        let ((_, stati), descrizione, _) = righe_scheda_dtc(&y);
        assert_eq!(stati, "permanente");
        assert_eq!(descrizione, "Descrizione non disponibile");
        assert_eq!(testo_stati(&[]), "");
    }

    #[test]
    fn stato_dei_monitor_breve_sullo_stretto() {
        let m = |disponibile, completato| Monitor {
            nome: "Catalizzatore".into(),
            disponibile,
            completato,
        };
        assert_eq!(stato_monitor(&m(false, false), true).0, "Non disponibile");
        assert_eq!(
            stato_monitor(&m(false, false), false).0,
            "Non disponibile su questo veicolo"
        );
        assert_eq!(
            stato_monitor(&m(true, true), true),
            ("Completato", tema::OK)
        );
        assert_eq!(
            stato_monitor(&m(true, false), false),
            ("Non completato", tema::ATTENZIONE)
        );
    }
}
