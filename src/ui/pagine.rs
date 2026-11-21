//! Le quattro schede dell'interfaccia.

use eframe::egui::{self, Color32, RichText, ScrollArea, Stroke};

use super::grafici::{self, Opzioni};
use super::{
    ARANCIO_DEMO, App, GIALLO, GRIGIO, ROSSO, Scheda, VERDE, apri_cartella, etichetta_stato,
};
use crate::diagnosis::{DiagnosisReport, Lettura};
use crate::elm::{ConfermaCancellazione, EsitoCancellazione, RispostaCancellazione};
use crate::model::{Stato, num_it};
use crate::obd::pids;
use crate::recording;
use crate::summary::{self, valore_con_unita};
use crate::worker::{Comando, Sorgente, StatoConn};

/// Fondo dei pulsanti delle azioni distruttive.
const ROSSO_SCURO: Color32 = Color32::from_rgb(140, 28, 40);

const EXTRA: [&str; 5] = ["giri", "velocita", "temp_refrigerante", "carico", "map"];
const MARCATORI_RAPIDI: [&str; 3] = ["Minimo", "2.500 giri", "Velocità costante"];

fn nome_parametro(key: &str) -> String {
    pids::per_key(key)
        .map(|p| p.nome.clone())
        .unwrap_or_else(|| key.to_string())
}

fn nome_file(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn riga_lettura(ui: &mut egui::Ui, titolo: &str, l: &Lettura) {
    if l.stato == Stato::Ok && l.dettaglio.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{titolo}:"));
        etichetta_stato(ui, l.stato);
        if !l.dettaglio.is_empty() {
            ui.label(RichText::new(&l.dettaglio).color(GRIGIO));
        }
    });
}

fn sezione(ui: &mut egui::Ui, titolo: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(titolo).heading());
    ui.separator();
}

/// Mostra un referto di diagnosi (dal vivo o da una sessione salvata).
fn disegna_diagnosi(ui: &mut egui::Ui, d: &DiagnosisReport) {
    egui::Grid::new("diag_info")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.label("Lettura del");
            ui.label(&d.letto_il);
            ui.end_row();
            ui.label("Adattatore");
            ui.label(format!("{} — {}", d.adattatore, d.collegamento));
            ui.end_row();
            ui.label("Protocollo");
            ui.label(&d.protocollo);
            ui.end_row();
            ui.label("Centraline");
            ui.label(d.ecu.join(", "));
            ui.end_row();
            if let Some(t) = &d.tensione {
                ui.label("Tensione al connettore");
                ui.label(format!("{t} (misurata dall'adattatore)"));
                ui.end_row();
            }
        });

    sezione(ui, "Spia MIL");
    riga_lettura(ui, "Lettura", &d.lettura_prontezza);
    for p in &d.prontezza {
        ui.horizontal(|ui| {
            ui.label(format!("ECU {}:", p.ecu));
            if p.mil_accesa {
                ui.label(RichText::new("MIL ACCESA").color(GIALLO).strong());
            } else {
                ui.label(RichText::new("MIL spenta").color(VERDE));
            }
            ui.label(format!("— codici memorizzati dichiarati: {}", p.numero_dtc));
        });
    }

    sezione(ui, "Codici guasto");
    if d.dtc.is_empty() {
        ui.label("Nessun codice riportato dalle letture riuscite.");
    } else {
        egui::Grid::new("diag_dtc")
            .num_columns(4)
            .striped(true)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                for t in ["Codice", "Descrizione", "ECU", "Stati rilevati"] {
                    ui.label(RichText::new(t).strong());
                }
                ui.end_row();
                for x in &d.dtc {
                    ui.label(RichText::new(&x.codice).strong().monospace());
                    ui.label(&x.descrizione);
                    ui.label(&x.ecu);
                    let stati: Vec<&str> = x.stati.iter().map(|s| s.etichetta()).collect();
                    ui.label(stati.join(", "));
                    ui.end_row();
                }
            });
    }
    riga_lettura(ui, "Codici confermati", &d.lettura_confermati);
    riga_lettura(ui, "Codici in sospeso", &d.lettura_in_sospeso);
    riga_lettura(ui, "Codici permanenti", &d.lettura_permanenti);

    sezione(ui, "Monitor di prontezza");
    if d.prontezza.is_empty() {
        etichetta_stato(ui, d.lettura_prontezza.stato);
    }
    for p in &d.prontezza {
        ui.label(format!(
            "ECU {} — motore {}",
            p.ecu,
            if p.diesel {
                "ad accensione spontanea"
            } else {
                "ad accensione comandata"
            }
        ));
        egui::Grid::new(format!("diag_pronto_{}", p.ecu))
            .num_columns(2)
            .striped(true)
            .spacing([18.0, 4.0])
            .show(ui, |ui| {
                for m in &p.monitor {
                    ui.label(&m.nome);
                    if !m.disponibile {
                        ui.label(RichText::new("Non disponibile su questo veicolo").color(GRIGIO));
                    } else if m.completato {
                        ui.label(RichText::new("Completato").color(VERDE));
                    } else {
                        ui.label(RichText::new("Non completato").color(GIALLO));
                    }
                    ui.end_row();
                }
            });
    }

    ui.add_space(12.0);
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(20, 38, 60))
        .stroke(Stroke::new(1.5, Color32::from_rgb(79, 195, 247)))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("FERMO IMMAGINE")
                    .heading()
                    .color(Color32::from_rgb(129, 212, 250)),
            );
            ui.label(
                RichText::new(
                    "Valori memorizzati dalla centralina nel momento in cui ha registrato il \
                     guasto. Non sono dati in tempo reale.",
                )
                .color(GRIGIO),
            );
            ui.add_space(4.0);
            let f = &d.fermo;
            if !f.presente {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Nessun fermo immagine disponibile.");
                    if f.lettura.stato != Stato::Ok {
                        etichetta_stato(ui, f.lettura.stato);
                    }
                    ui.label(RichText::new(&f.lettura.dettaglio).color(GRIGIO));
                });
                return;
            }
            ui.horizontal(|ui| {
                ui.label("DTC che ha generato il fotogramma:");
                ui.label(
                    RichText::new(f.dtc.as_deref().unwrap_or("?"))
                        .strong()
                        .monospace(),
                );
                ui.label(format!("(ECU {})", f.ecu));
            });
            ui.add_space(4.0);
            egui::Grid::new("diag_fermo")
                .num_columns(3)
                .striped(true)
                .spacing([24.0, 5.0])
                .show(ui, |ui| {
                    for t in ["Parametro", "Valore", "Stato"] {
                        ui.label(RichText::new(t).strong());
                    }
                    ui.end_row();
                    for r in &f.righe {
                        ui.label(&r.nome);
                        match (r.stato, r.valore, &r.testo) {
                            (Stato::Ok, Some(v), _) => {
                                ui.label(
                                    RichText::new(valore_con_unita(&r.key, v, &r.unita)).strong(),
                                );
                            }
                            (Stato::Ok, None, Some(t)) => {
                                ui.label(RichText::new(t).strong());
                            }
                            _ => {
                                ui.label(RichText::new("—").color(GRIGIO));
                            }
                        }
                        etichetta_stato(ui, r.stato);
                        ui.end_row();
                    }
                });
        });

    sezione(ui, "Monitor del catalizzatore (Mode 06, MID 21)");
    riga_lettura(ui, "Lettura", &d.mode06.lettura);
    if !d.mode06.test.is_empty() {
        egui::Grid::new("diag_m06")
            .num_columns(6)
            .striped(true)
            .spacing([18.0, 5.0])
            .show(ui, |ui| {
                for t in [
                    "ECU",
                    "Test (TID)",
                    "Valore",
                    "Minimo",
                    "Massimo",
                    "Dato grezzo (valore / min / max, scala)",
                ] {
                    ui.label(RichText::new(t).strong());
                }
                ui.end_row();
                for t in &d.mode06.test {
                    ui.label(&t.ecu);
                    ui.label(format!("{:02X}", t.tid));
                    match &t.scalato {
                        Some(s) => {
                            ui.label(format!("{} {}", num_it(s.valore, 4), s.unita));
                            ui.label(format!("{} {}", num_it(s.minimo, 4), s.unita));
                            ui.label(format!("{} {}", num_it(s.massimo, 4), s.unita));
                        }
                        None => {
                            ui.label(RichText::new("solo grezzo").color(GRIGIO));
                            ui.label("");
                            ui.label("");
                        }
                    }
                    ui.label(
                        RichText::new(format!(
                            "{:04X} / {:04X} / {:04X}, scala {:02X}",
                            t.grezzo_valore, t.grezzo_minimo, t.grezzo_massimo, t.uas_id
                        ))
                        .monospace(),
                    );
                    ui.end_row();
                }
            });
        ui.label(
            RichText::new(
                "Le unità derivano dalla tabella «Unit and Scaling ID» ripresa dal progetto \
                 python-OBD, non confrontata con il testo SAE J1979: fa fede il dato grezzo. \
                 Il significato dei TID da 80 in su è definito dal costruttore. L'app non \
                 giudica l'esito del test.",
            )
            .small()
            .color(GRIGIO),
        );
    }

    ui.add_space(10.0);
    ui.collapsing("Parametri disponibili (PID del servizio 01)", |ui| {
        for (ecu, lista) in &d.pid_supportati {
            let testo: Vec<String> = lista.iter().map(|p| format!("{p:02X}")).collect();
            ui.label(RichText::new(format!("ECU {ecu}: {}", testo.join(" "))).monospace());
        }
    });
}

impl App {
    // ------------------------------------------------------------------
    // Connessione
    // ------------------------------------------------------------------

    pub(crate) fn pagina_connessione(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.collegato() {
                    self.connessione_attiva(ui);
                } else {
                    self.connessione_scelta(ui);
                }
                if let Some(g) = self.gatt_sconosciuto.clone() {
                    sezione(
                        ui,
                        "Servizi GATT del dispositivo (profilo non riconosciuto)",
                    );
                    ui.label(
                        "Nessuna scrittura è stata eseguita. Copia questo elenco per verificare \
                         quale profilo usa l'adattatore.",
                    );
                    let testo = g.testo();
                    if ui.button("Copia elenco UUID").clicked() {
                        ui.ctx().copy_text(testo.clone());
                    }
                    ui.label(RichText::new(testo).monospace());
                }
                ui.add_space(10.0);
                ui.collapsing("Registro degli scambi con l'adattatore", |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Copia registro").clicked() {
                            ui.ctx().copy_text(self.registro.join("\n"));
                        }
                        if ui.button("Svuota").clicked() {
                            self.registro.clear();
                        }
                        ui.label(
                            RichText::new("La risposta al VIN non viene mai registrata.")
                                .small()
                                .color(GRIGIO),
                        );
                    });
                    ScrollArea::vertical()
                        .id_salt("registro")
                        .max_height(260.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for r in &self.registro {
                                ui.label(RichText::new(r).monospace().small());
                            }
                        });
                });
            });
    }

    fn connessione_attiva(&mut self, ui: &mut egui::Ui) {
        let Some(info) = self.info.clone() else {
            return;
        };
        sezione(ui, "Collegamento attivo");
        egui::Grid::new("conn_info")
            .num_columns(2)
            .spacing([16.0, 5.0])
            .show(ui, |ui| {
                ui.label("Collegamento");
                ui.label(&info.collegamento);
                ui.end_row();
                ui.label("Adattatore");
                ui.label(&info.adattatore);
                ui.end_row();
                ui.label("Protocollo");
                ui.label(&info.protocollo);
                ui.end_row();
                ui.label("Centraline che rispondono");
                ui.label(info.ecu.join(", "));
                ui.end_row();
                ui.label("Parametri dichiarati");
                ui.label(format!("{} PID del servizio 01", info.unione.len()));
                ui.end_row();
                if let Some(g) = &info.gatt {
                    ui.label("Profilo BLE");
                    ui.label(format!(
                        "{} — fonte: {}",
                        g.profilo.as_deref().unwrap_or("?"),
                        g.fonte_profilo.as_deref().unwrap_or("?")
                    ));
                    ui.end_row();
                }
            });
        for a in &info.avvisi {
            ui.label(RichText::new(a).color(GIALLO));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Scollega").clicked() {
                self.invia(Comando::Scollega);
            }
            if ui.button("Vai ai codici guasto").clicked() {
                self.scheda = Scheda::Diagnosi;
            }
            if ui.button("Vai ai dati in tempo reale").clicked() {
                self.scheda = Scheda::Live;
            }
        });
        if let Some(g) = &info.gatt {
            ui.add_space(6.0);
            ui.collapsing("Servizi GATT del dispositivo", |ui| {
                ui.label(RichText::new(g.testo()).monospace().small());
            });
        }
    }

    fn connessione_scelta(&mut self, ui: &mut egui::Ui) {
        let libero = matches!(self.stato, StatoConn::Scollegato | StatoConn::Scansione);
        if let StatoConn::Connessione(fase) = &self.stato {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(fase).color(GIALLO));
            });
        }
        ui.label(
            "L'app legge dati diagnostici standard. L'unica eccezione è «Cancella codici \
             guasto», che chiede sempre una conferma esplicita. Non scrive sulle centraline, \
             non esegue codifiche né test attuatori e non permette di inviare comandi liberi.",
        );

        sezione(ui, "1. Bluetooth Low Energy");
        ui.label(
            "Inserisci l'adattatore nella presa OBD con il quadro acceso e chiudi sul telefono \
             le app collegate all'adattatore. Non serve associarlo in Windows.",
        );
        ui.horizontal(|ui| {
            if self.stato == StatoConn::Scansione {
                if ui.button("Ferma ricerca").clicked() {
                    self.invia(Comando::FermaScansione);
                }
                ui.spinner();
                ui.label("Ricerca in corso…");
            } else if ui
                .add_enabled(libero, egui::Button::new("Cerca dispositivi"))
                .clicked()
            {
                self.dispositivi.clear();
                self.ble_scelto = None;
                self.invia(Comando::ScansionaBle);
            }
            ui.checkbox(
                &mut self.mostra_senza_nome,
                "Mostra anche i dispositivi senza nome",
            );
        });
        let visibili: Vec<_> = self
            .dispositivi
            .iter()
            .filter(|d| self.mostra_senza_nome || !d.nome.is_empty())
            .cloned()
            .collect();
        if !visibili.is_empty() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width().min(720.0));
                ScrollArea::vertical()
                    .id_salt("ble")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for d in &visibili {
                            let nome = if d.nome.is_empty() {
                                "(senza nome)"
                            } else {
                                d.nome.as_str()
                            };
                            let segnale = d
                                .rssi
                                .map(|r| format!("{r} dBm"))
                                .unwrap_or_else(|| "segnale n.d.".into());
                            let testo = format!(
                                "{}{nome}   [{}]   {segnale}",
                                if d.probabile_obd { "[OBD] " } else { "" },
                                d.indirizzo
                            );
                            let scelto = self.ble_scelto.as_deref() == Some(&d.indirizzo);
                            if ui.selectable_label(scelto, testo).clicked() {
                                self.ble_scelto = Some(d.indirizzo.clone());
                            }
                        }
                    });
            });
            ui.label(
                RichText::new("[OBD] = nome o servizi annunciati tipici di un adattatore OBD")
                    .small()
                    .color(GRIGIO),
            );
        } else if self.stato == StatoConn::Scansione {
            ui.label(RichText::new("Nessun dispositivo trovato finora.").color(GRIGIO));
        }
        let scelto = self.ble_scelto.clone();
        if ui
            .add_enabled(
                libero && scelto.is_some(),
                egui::Button::new("Collega il dispositivo selezionato"),
            )
            .clicked()
            && let Some(indirizzo) = scelto
        {
            self.errore = None;
            self.gatt_sconosciuto = None;
            self.invia(Comando::Collega(Sorgente::Ble(indirizzo)));
        }

        sezione(ui, "2. Porta seriale (Bluetooth Classic/SPP o USB)");
        ui.label(
            "Per gli adattatori associati in Windows come dispositivo Bluetooth classico: \
             scegli la porta COM «in uscita» creata dall'associazione.",
        );
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("porta")
                .width(260.0)
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
                .width(110.0)
                .selected_text(format!("{} baud", self.baud))
                .show_ui(ui, |ui| {
                    for b in [38400u32, 9600, 115200] {
                        ui.selectable_value(&mut self.baud, b, format!("{b} baud"));
                    }
                });
            if ui.button("Aggiorna elenco").clicked() {
                self.invia(Comando::ElencaSeriali);
            }
            if ui
                .add_enabled(
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
        });

        sezione(ui, "3. Modalità demo");
        ui.label(
            "Prova l'app senza veicolo: un simulatore risponde come un ELM327 collegato a un \
             motore benzina con P0420. I dati sono sintetici.",
        );
        if ui
            .add_enabled(libero, egui::Button::new("Avvia la demo"))
            .clicked()
        {
            self.errore = None;
            self.invia(Comando::Collega(Sorgente::Demo));
        }
    }

    // ------------------------------------------------------------------
    // Diagnosi
    // ------------------------------------------------------------------

    pub(crate) fn pagina_diagnosi(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.pronto(), egui::Button::new("Leggi diagnosi"))
                .clicked()
            {
                self.invia(Comando::LeggiDiagnosi);
            }
            if self.stato == StatoConn::Diagnosi {
                ui.spinner();
                ui.label("Lettura in corso…");
            }
            if self.stato == StatoConn::Cancellazione {
                ui.spinner();
                ui.label(RichText::new("Cancellazione in corso…").color(ROSSO));
            }
            if let Some(d) = &self.diagnosi {
                if ui.button("Copia riepilogo").clicked() {
                    ui.ctx().copy_text(summary::riepilogo(d, self.includi_vin));
                }
                ui.checkbox(&mut self.includi_vin, "Includi il VIN nel riepilogo");
            }
            // Azione distruttiva: all'estremo opposto di «Leggi diagnosi».
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let pulsante = egui::Button::new(
                    RichText::new("Cancella codici guasto…")
                        .color(Color32::WHITE)
                        .strong(),
                )
                .fill(ROSSO_SCURO)
                .stroke(Stroke::new(1.0, ROSSO));
                if ui
                    .add_enabled(self.pronto(), pulsante)
                    .on_hover_text(
                        "Cancella codici memorizzati e in sospeso, fermo immagine e prontezza. \
                         Chiede conferma.",
                    )
                    .clicked()
                {
                    self.conferma_cancellazione = Some(false);
                }
            });
        });
        ui.separator();
        self.disegna_esito_cancellazione(ui);
        if !self.collegato() && self.diagnosi.is_some() {
            ui.label(
                RichText::new("Adattatore non collegato: questa è l'ultima lettura eseguita.")
                    .color(GIALLO),
            );
        }
        let Some(d) = &self.diagnosi else {
            ui.add_space(20.0);
            ui.label(if self.collegato() {
                "Premi «Leggi diagnosi» per leggere codici guasto, spia MIL, prontezza e fermo \
                 immagine."
            } else {
                "Collega l'adattatore o avvia la demo dalla scheda «Connessione»."
            });
            return;
        };
        let mut area = ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(y) = self.scorri_diagnosi.take() {
            area = area.vertical_scroll_offset(y);
        }
        area.show(ui, |ui| {
            disegna_diagnosi(ui, d);
            if let Some(v) = &d.vin {
                ui.add_space(6.0);
                ui.collapsing("VIN (non viene mai salvato né esportato)", |ui| {
                    ui.label(RichText::new(v).monospace());
                });
            }
        });
    }

    /// Esito dell'ultima cancellazione, in chiaro, sopra la diagnosi riletta.
    fn disegna_esito_cancellazione(&mut self, ui: &mut egui::Ui) {
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
                        VERDE,
                        "La diagnosi qui sotto è stata riletta dopo la cancellazione. I codici \
                         permanenti restano finché la centralina non li elimina da sola; i \
                         monitor di prontezza si completano nei prossimi cicli di guida."
                            .to_string(),
                    )
                } else if accettate > 0 {
                    (
                        "Cancellazione eseguita solo da alcune centraline",
                        GIALLO,
                        "La diagnosi qui sotto è stata riletta dopo la cancellazione.".to_string(),
                    )
                } else {
                    (
                        "Cancellazione rifiutata dalla centralina",
                        ROSSO,
                        "I codici non sono stati cancellati. La diagnosi qui sotto è stata \
                         riletta."
                            .to_string(),
                    )
                }
            }
            EsitoCancellazione::NessunaRisposta => (
                "La centralina non ha risposto alla cancellazione",
                ROSSO,
                "Probabilmente i codici non sono stati cancellati: controlla la diagnosi \
                 riletta qui sotto e verifica che il quadro sia acceso."
                    .to_string(),
            ),
            EsitoCancellazione::ErroreComunicazione(d) => (
                "Errore durante la cancellazione",
                ROSSO,
                format!(
                    "Non è possibile sapere se i codici sono stati cancellati: controlla la \
                     diagnosi riletta. Dettaglio: {d}"
                ),
            ),
        };
        let mut chiudi = false;
        egui::Frame::group(ui.style())
            .stroke(Stroke::new(1.5, colore))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(titolo).heading().color(colore));
                    if r.demo {
                        ui.label(
                            RichText::new(" SIMULATA — modalità demo, nessun veicolo coinvolto ")
                                .background_color(ARANCIO_DEMO)
                                .color(Color32::BLACK)
                                .strong(),
                        );
                    }
                });
                if let EsitoCancellazione::Risposte(v) = &r.esito {
                    for e in v {
                        let c = if e.risposta == RispostaCancellazione::Accettata {
                            VERDE
                        } else {
                            ROSSO
                        };
                        ui.label(RichText::new(format!("ECU {}: {}", e.ecu, e.testo())).color(c));
                    }
                }
                ui.label(spiegazione);
                ui.label(
                    RichText::new(format!("Eseguita il {}", r.eseguita_il))
                        .small()
                        .color(GRIGIO),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label("Copia della diagnosi precedente:");
                    ui.label(RichText::new(r.copia.display().to_string()).monospace());
                });
                ui.horizontal(|ui| {
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
        ui.add_space(6.0);
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
        let demo = self.info.as_ref().is_some_and(|i| i.demo);
        let pronto = self.pronto();
        let mut annulla = false;
        let mut conferma = false;
        let punto = |ui: &mut egui::Ui, testo: &str| {
            ui.horizontal_wrapped(|ui| {
                ui.label("•");
                ui.label(testo);
            });
        };
        let risposta = egui::Modal::new(egui::Id::new("conferma_cancellazione"))
            .backdrop_color(Color32::from_black_alpha(170))
            .show(ctx, |ui| {
                ui.set_max_width(580.0);
                ui.label(
                    RichText::new("Cancellare i codici guasto?")
                        .heading()
                        .color(ROSSO),
                );
                ui.label(
                    RichText::new(
                        "Questa operazione modifica lo stato della centralina e non si può \
                         annullare.",
                    )
                    .strong(),
                );
                if demo {
                    ui.label(
                        RichText::new(
                            " MODALITÀ DEMO — la cancellazione avviene sul simulatore, non su un \
                             veicolo ",
                        )
                        .background_color(ARANCIO_DEMO)
                        .color(Color32::BLACK)
                        .strong(),
                    );
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Verranno cancellati").strong());
                punto(ui, "i codici guasto memorizzati (confermati) e in sospeso;");
                punto(ui, "il fermo immagine;");
                punto(ui, "i risultati dei monitor (Mode 06);");
                punto(
                    ui,
                    "lo stato di prontezza: i monitor tornano «non completato»; si azzerano \
                     anche distanza e tempo dalla cancellazione. La spia MIL si spegne.",
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Non verranno cancellati").strong());
                punto(
                    ui,
                    "i codici permanenti: li elimina solo la centralina, dopo alcuni cicli di \
                     guida senza che il guasto si ripresenti.",
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Condizioni richieste").strong());
                punto(
                    ui,
                    "quadro acceso e motore spento. Con il motore acceso molte centraline \
                     rifiutano la cancellazione.",
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Revisione").strong().color(GIALLO));
                ui.label(
                    "Finché i monitor di prontezza non tornano «completato» (servono uno o più \
                     cicli di guida) il veicolo può non superare la revisione. Cancellare i \
                     codici non ripara il guasto: se la causa resta, il codice ricompare.",
                );
                ui.add_space(4.0);
                ui.label(
                    "Prima di cancellare l'app salva una copia della diagnosi attuale, fermo \
                     immagine compreso, nella cartella delle sessioni.",
                );
                ui.add_space(10.0);
                ui.checkbox(&mut spunta, "Ho letto e voglio cancellare i codici");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let r_annulla =
                        ui.add(egui::Button::new("Annulla").min_size(egui::vec2(120.0, 32.0)));
                    // Pulsante predefinito: riceve il fuoco, Invio annulla.
                    if ui.memory(|m| m.focused().is_none()) {
                        r_annulla.request_focus();
                    }
                    if r_annulla.clicked() {
                        annulla = true;
                    }
                    ui.add_space(16.0);
                    let pulsante = egui::Button::new(
                        RichText::new("Cancella codici guasto")
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(ROSSO_SCURO)
                    .stroke(Stroke::new(1.0, ROSSO))
                    .min_size(egui::vec2(180.0, 32.0));
                    if ui.add_enabled(spunta && pronto, pulsante).clicked() {
                        conferma = true;
                    }
                });
                if !pronto {
                    ui.label(
                        RichText::new("Attendi la fine dell'operazione in corso.")
                            .small()
                            .color(GRIGIO),
                    );
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

    // ------------------------------------------------------------------
    // Dati in tempo reale
    // ------------------------------------------------------------------

    pub(crate) fn pagina_live(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("live_sinistra")
            .resizable(false)
            .exact_size(340.0)
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.live_controlli(ui));
            });
        egui::CentralPanel::default().show(ui, |ui| self.live_grafici(ui));
    }

    fn live_controlli(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Diagnosi P0420").heading());
        if !self.collegato() {
            ui.label(
                RichText::new("Non collegato: collega l'adattatore o avvia la demo.").color(GIALLO),
            );
        }
        let sintetico = if self.live_demo { " (sintetico)" } else { "" };
        for (titolo, scelta) in [
            ("B1S1, a monte", self.preset.b1s1.clone()),
            ("B1S2, a valle", self.preset.b1s2.clone()),
        ] {
            match scelta {
                Some(k) => {
                    let p = pids::per_key(&k);
                    ui.label(format!(
                        "{}{} [{}]",
                        nome_parametro(&k),
                        sintetico,
                        p.map(|p| p.unita).unwrap_or("")
                    ));
                }
                None if self.collegato() => {
                    ui.label(
                        RichText::new(format!(
                            "Sonda {titolo}: nessun segnale esposto dalla centralina"
                        ))
                        .color(GRIGIO),
                    );
                }
                None => {}
            }
        }
        let mut cambiata = false;
        let voci: Vec<(String, bool)> = self
            .preset
            .base
            .iter()
            .chain(self.preset.opzionali.iter())
            .cloned()
            .collect();
        let n_base = self.preset.base.len();
        for (i, (k, supportato)) in voci.iter().enumerate() {
            if i == n_base {
                ui.label(
                    RichText::new("Parametri selezionabili")
                        .small()
                        .color(GRIGIO),
                );
            }
            let mut attivo = self.selezione.get(k).copied().unwrap_or(false);
            let testo = if *supportato {
                nome_parametro(k)
            } else {
                format!("{} — non supportato", nome_parametro(k))
            };
            if ui
                .add_enabled(*supportato, egui::Checkbox::new(&mut attivo, testo))
                .changed()
            {
                self.selezione.insert(k.clone(), attivo);
                cambiata = true;
            }
        }
        if cambiata && self.polling {
            self.avvia_polling();
        }

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
                .add_enabled(self.pronto(), egui::Button::new("Avvia lettura"))
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
            .add_enabled(self.pronto(), egui::Button::new("Avvia registrazione"))
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

        ui.add_space(8.0);
        ui.label(RichText::new("Valori attuali").heading());
        let (chiavi, _) = self.chiavi_polling();
        egui::Grid::new("live_valori")
            .num_columns(3)
            .striped(true)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Parametro").strong());
                ui.label(RichText::new("Valore").strong());
                ui.label(RichText::new("Hz").strong());
                ui.end_row();
                for k in &chiavi {
                    let nome = pids::per_key(k)
                        .map(|p| {
                            p.nome
                                .replace("Sonda ", "")
                                .replace(" (a monte)", "")
                                .replace(" (a valle)", "")
                        })
                        .unwrap_or_else(|| k.clone());
                    ui.label(nome);
                    match self.live.ultimo(k) {
                        Some(p) => match p.v {
                            Some(v) => {
                                let unita = pids::per_key(k).map(|p| p.unita).unwrap_or("");
                                ui.label(RichText::new(valore_con_unita(k, v, unita)).strong());
                            }
                            None => etichetta_stato(ui, p.stato),
                        },
                        None => {
                            ui.label(RichText::new("in attesa").color(GRIGIO));
                        }
                    }
                    match self.live.frequenza(k) {
                        Some(hz) => ui.label(num_it(hz, 1)),
                        None => ui.label(RichText::new("—").color(GRIGIO)),
                    };
                    ui.end_row();
                }
            });
        ui.label(
            RichText::new("Hz = frequenza effettiva di aggiornamento negli ultimi 5 secondi.")
                .small()
                .color(GRIGIO),
        );
    }

    fn live_grafici(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
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
            ui.separator();
            selettore_extra(ui, "live_extra", &mut self.extra, &self.live);
        });
        ui.label(
            RichText::new(
                "I grafici servono a confrontare i segnali. L'app non deduce se catalizzatore o \
                 sonde siano guasti. Le fasce rosse indicano intervalli senza dato valido.",
            )
            .small()
            .color(GRIGIO),
        );
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

    // ------------------------------------------------------------------
    // Sessioni
    // ------------------------------------------------------------------

    pub(crate) fn pagina_sessioni(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("sessioni_sinistra")
            .resizable(false)
            .exact_size(340.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Aggiorna").clicked() {
                        self.elenco = recording::elenca(&self.cartella);
                    }
                    if ui.button("Apri la cartella").clicked() {
                        apri_cartella(&self.cartella);
                    }
                });
                ui.label(
                    RichText::new(self.cartella.display().to_string())
                        .small()
                        .color(GRIGIO),
                );
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
        egui::CentralPanel::default().show(ui, |ui| {
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
            let Some(s) = self.aperta.as_mut() else {
                ui.label(
                    "Scegli una sessione dall'elenco per rivedere i grafici senza collegare il \
                     veicolo.",
                );
                return;
            };
            let i = &s.dati.intestazione;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(nome_file(&s.dati.percorso)).strong());
                if i.demo {
                    ui.label(
                        RichText::new(" SESSIONE DEMO — dati sintetici ")
                            .background_color(ARANCIO_DEMO)
                            .color(Color32::BLACK)
                            .strong(),
                    );
                }
            });
            ui.label(format!(
                "Creata il {} — {} — {} — ECU {}",
                i.creata,
                i.adattatore,
                i.protocollo,
                i.ecu.join(", ")
            ));
            ui.label(format!(
                "{} campioni, {} marcatori, durata {} s{}",
                num_it(s.dati.campioni.len() as f64, 0),
                s.dati.marcatori.len(),
                num_it(s.archivio.t_max, 0),
                if s.dati.righe_scartate > 0 {
                    format!(
                        " — {} righe illeggibili ignorate (file interrotto)",
                        s.dati.righe_scartate
                    )
                } else {
                    String::new()
                }
            ));
            ui.horizontal_wrapped(|ui| {
                if ui.button("Esporta CSV").clicked() {
                    s.messaggio = Some(match recording::esporta_csv(&s.dati) {
                        Ok(p) => format!("CSV scritto: {}", p.display()),
                        Err(e) => e,
                    });
                }
                if let Some(d) = &s.dati.diagnosi
                    && ui.button("Copia riepilogo diagnosi").clicked()
                {
                    ui.ctx().copy_text(summary::riepilogo(d, false));
                    s.messaggio = Some("Riepilogo copiato negli appunti (senza VIN).".into());
                }
                ui.separator();
                selettore_extra(ui, "sessione_extra", &mut s.extra, &s.archivio);
            });
            if let Some(m) = &s.messaggio {
                ui.label(RichText::new(m).color(VERDE));
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
                    sintetico: i.demo,
                },
            );
            if !disegnato {
                ui.label("La sessione non contiene segnali da tracciare.");
            }
        });
    }
}

/// Scelta del parametro aggiuntivo da tracciare.
fn selettore_extra(
    ui: &mut egui::Ui,
    id: &str,
    scelta: &mut Option<String>,
    archivio: &super::dati::Archivio,
) {
    ui.label("Grafico aggiuntivo:");
    egui::ComboBox::from_id_salt(id)
        .width(210.0)
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
        });
}
