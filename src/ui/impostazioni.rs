//! Scheda Impostazioni: per ora la sola pubblicazione MQTT.

use eframe::egui::{self, Color32, RichText, ScrollArea};

use super::{App, GIALLO, GRIGIO, ROSSO, VERDE};
use crate::config;
use crate::mqtt::{Argomenti, Campo, MqttConfig, Problema, StatoMqtt, StatoMqttInfo};
use crate::worker::Comando;

/// Stato della scheda: il modulo in modifica e l'ultimo stato comunicato dal
/// thread di lavoro.
pub(crate) struct Impostazioni {
    pub form: MqttConfig,
    pub info: StatoMqttInfo,
    /// I problemi si mostrano solo dopo il primo tentativo di salvataggio.
    mostra_problemi: bool,
    /// Esito dell'ultima azione; il secondo valore è vero se è un errore.
    messaggio: Option<(String, bool)>,
}

impl Impostazioni {
    /// Legge `mqtt.json`. Se il file è illeggibile si parte dai valori
    /// predefiniti e lo si dice.
    pub fn carica() -> Self {
        let (form, messaggio) = match MqttConfig::carica(&config::cartella_configurazione()) {
            Ok(c) => (c, None),
            Err(e) => (
                MqttConfig::default(),
                Some((format!("Impostazioni MQTT non lette: {e}"), true)),
            ),
        };
        Impostazioni {
            form,
            info: StatoMqttInfo::disattivato(),
            mostra_problemi: false,
            messaggio,
        }
    }

    /// Indicatore compatto per la barra superiore; `None` se MQTT è spento.
    pub fn testo_barra(&self) -> Option<(String, Color32)> {
        match &self.info.stato {
            StatoMqtt::Disattivato => None,
            StatoMqtt::Connessione => Some(("MQTT: connessione in corso".into(), GIALLO)),
            StatoMqtt::Connesso => Some(("MQTT: connesso".into(), VERDE)),
            StatoMqtt::Errore(_) => Some(("MQTT: non raggiungibile".into(), ROSSO)),
        }
    }
}

fn sezione(ui: &mut egui::Ui, titolo: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(titolo).heading());
    ui.separator();
}

/// Campo con il messaggio di errore sotto, se c'è.
fn con_errore(
    ui: &mut egui::Ui,
    problemi: &[Problema],
    campo: Campo,
    contenuto: impl FnOnce(&mut egui::Ui),
) {
    ui.vertical(|ui| {
        contenuto(ui);
        for p in problemi.iter().filter(|p| p.campo == campo) {
            ui.label(RichText::new(&p.messaggio).color(ROSSO).small());
        }
    });
}

fn nota(ui: &mut egui::Ui, testo: &str) {
    ui.label(RichText::new(testo).small().color(GRIGIO));
}

impl App {
    pub(super) fn pagina_impostazioni(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(760.0);
            ui.label(RichText::new("Impostazioni").heading());
            ui.add_space(4.0);
            ui.label(
                "OBDio può pubblicare i dati letti su un broker MQTT, per esempio per Home \
                 Assistant, mentre continua a funzionare come sempre. Se il broker non è \
                 raggiungibile la lettura dei dati non rallenta: i messaggi in eccesso vengono \
                 scartati e contati.",
            );

            sezione(ui, "Pubblicazione MQTT");
            self.modulo_mqtt(ui);
            self.stato_mqtt(ui);
            self.anteprima_argomenti(ui);

            sezione(ui, "Sicurezza e riservatezza");
            ui.label(
                "Il VIN non viene mai pubblicato, nemmeno se lo includi nel riepilogo della \
                 diagnosi.",
            );
            ui.label(
                RichText::new(
                    "La connessione al broker non è cifrata: TLS non è supportato in questa \
                     versione. Utente, password e dati viaggiano in chiaro sulla rete. Usa un \
                     broker nella tua rete locale e una password che non usi altrove.",
                )
                .color(GIALLO),
            );
            ui.add_space(12.0);
        });
    }

    fn modulo_mqtt(&mut self, ui: &mut egui::Ui) {
        let imp = &mut self.impostazioni;
        let problemi = if imp.mostra_problemi {
            imp.form.problemi()
        } else {
            Vec::new()
        };
        let f = &mut imp.form;
        egui::Grid::new("modulo_mqtt")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("");
                ui.checkbox(&mut f.attivo, "Pubblica i dati su MQTT");
                ui.end_row();

                ui.label("Indirizzo del broker");
                con_errore(ui, &problemi, Campo::Host, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut f.host)
                            .hint_text("192.168.1.10 oppure broker.local")
                            .desired_width(280.0),
                    );
                });
                ui.end_row();

                ui.label("Porta");
                con_errore(ui, &problemi, Campo::Porta, |ui| {
                    ui.add(egui::DragValue::new(&mut f.porta).range(0..=65535));
                });
                ui.end_row();

                ui.label("Utente");
                ui.add(
                    egui::TextEdit::singleline(&mut f.utente)
                        .hint_text("facoltativo")
                        .desired_width(280.0),
                );
                ui.end_row();

                ui.label("Password");
                ui.vertical(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut f.password)
                            .password(true)
                            .desired_width(280.0),
                    );
                    ui.checkbox(
                        &mut f.ricorda_password,
                        "Ricorda la password (salvata in chiaro)",
                    );
                    if f.ricorda_password {
                        ui.label(
                            RichText::new(
                                "La password verrà scritta senza protezione nel file mqtt.json, \
                                 leggibile da chiunque abbia accesso a questo PC.",
                            )
                            .small()
                            .color(GIALLO),
                        );
                    } else {
                        nota(
                            ui,
                            "Non viene salvata su disco: dopo il riavvio va inserita di nuovo.",
                        );
                    }
                });
                ui.end_row();

                ui.label("Identificativo del client");
                con_errore(ui, &problemi, Campo::IdClient, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut f.id_client)
                            .hint_text("vuoto: generato a ogni avvio")
                            .desired_width(280.0),
                    );
                });
                ui.end_row();

                ui.label("Prefisso degli argomenti");
                con_errore(ui, &problemi, Campo::Prefisso, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.prefisso).desired_width(280.0));
                });
                ui.end_row();

                ui.label("Nome del veicolo");
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.veicolo).desired_width(280.0));
                    let pulito = f.veicolo_pulito();
                    if pulito != f.veicolo {
                        nota(
                            ui,
                            &format!(
                                "Negli argomenti si usa «{pulito}»: sono ammessi solo a-z, 0-9, \
                                 «_» e «-»."
                            ),
                        );
                    }
                });
                ui.end_row();

                ui.label("Qualità del servizio");
                con_errore(ui, &problemi, Campo::Qos, |ui| {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut f.qos, 0, "0 (al più una volta)");
                        ui.radio_value(&mut f.qos, 1, "1 (almeno una volta)");
                    });
                });
                ui.end_row();

                ui.label("Intervallo per sensore");
                con_errore(ui, &problemi, Campo::IntervalloSensore, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut f.intervallo_sensore_ms)
                            .range(0..=3_600_000)
                            .speed(10.0)
                            .suffix(" ms"),
                    );
                    nota(ui, "Tempo minimo fra due messaggi dello stesso sensore.");
                });
                ui.end_row();

                ui.label("Intervallo dell'istantanea");
                con_errore(ui, &problemi, Campo::IntervalloIstantanea, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut f.intervallo_istantanea_ms)
                            .range(0..=3_600_000)
                            .speed(10.0)
                            .suffix(" ms"),
                    );
                    nota(ui, "Tempo minimo fra due messaggi con tutti i valori.");
                });
                ui.end_row();

                ui.label("Home Assistant");
                ui.checkbox(
                    &mut f.discovery_home_assistant,
                    "Annuncia i sensori a Home Assistant (discovery)",
                );
                ui.end_row();

                ui.label("Prefisso di discovery");
                con_errore(ui, &problemi, Campo::PrefissoDiscovery, |ui| {
                    ui.add_enabled(
                        f.discovery_home_assistant,
                        egui::TextEdit::singleline(&mut f.prefisso_discovery).desired_width(280.0),
                    );
                });
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new("Salva e applica").min_size(egui::vec2(0.0, 30.0)))
                .clicked()
            {
                self.applica_mqtt(false);
            }
            if ui
                .add(egui::Button::new("Disattiva").min_size(egui::vec2(0.0, 30.0)))
                .clicked()
            {
                self.applica_mqtt(true);
            }
        });
        if let Some((testo, errore)) = &self.impostazioni.messaggio {
            let colore = if *errore { ROSSO } else { VERDE };
            ui.label(RichText::new(testo).color(colore));
        }
    }

    /// Salva il modulo e lo manda al thread di lavoro.
    fn applica_mqtt(&mut self, disattiva: bool) {
        let imp = &mut self.impostazioni;
        imp.mostra_problemi = true;
        if disattiva {
            imp.form.attivo = false;
        }
        imp.form.host = imp.form.host.trim().to_string();
        let problemi = imp.form.problemi();
        if imp.form.attivo && !problemi.is_empty() {
            imp.messaggio = Some((
                "Controlla i valori segnalati in rosso: nulla è stato salvato né applicato.".into(),
                true,
            ));
            return;
        }
        let esito = imp.form.salva(&config::cartella_configurazione());
        imp.messaggio = Some(match esito {
            Ok(()) if imp.form.attivo => ("Impostazioni salvate e applicate.".into(), false),
            Ok(()) => (
                "Impostazioni salvate. La pubblicazione MQTT è disattivata.".into(),
                false,
            ),
            Err(e) => (
                format!("Impostazioni applicate ma non salvate su disco: {e}"),
                true,
            ),
        });
        let config = imp.form.clone();
        self.invia(Comando::ConfiguraMqtt(config));
    }

    fn stato_mqtt(&self, ui: &mut egui::Ui) {
        let info = &self.impostazioni.info;
        sezione(ui, "Stato");
        let colore = match &info.stato {
            StatoMqtt::Disattivato => GRIGIO,
            StatoMqtt::Connessione => GIALLO,
            StatoMqtt::Connesso => VERDE,
            StatoMqtt::Errore(_) => ROSSO,
        };
        ui.label(RichText::new(info.stato.testo()).color(colore).strong());
        ui.label(format!(
            "Messaggi accodati per l'invio: {} — scartati: {}",
            info.pubblicati, info.scartati
        ));
        nota(
            ui,
            "Gli scartati sono messaggi dei sensori che non hanno trovato posto nella coda o \
             sono arrivati mentre il broker non era raggiungibile.",
        );
        if let Some(e) = &info.ultimo_errore {
            ui.label(RichText::new(format!("Ultimo errore: {e}")).color(ROSSO));
        }
    }

    fn anteprima_argomenti(&self, ui: &mut egui::Ui) {
        let f = &self.impostazioni.form;
        let a = Argomenti::new(f);
        sezione(ui, "Argomenti pubblicati");
        nota(ui, "Con le impostazioni del modulo qui sopra:");
        let mut righe = vec![
            (
                a.stato(),
                "stato dell'app (trattenuto, ultima volontà «offline»)",
            ),
            (
                a.modello_sensore(),
                "un numero per sensore, per esempio giri",
            ),
            (
                a.istantanea(),
                "JSON con tutti i valori, assenti compresi (null)",
            ),
            (a.diagnosi(), "JSON della diagnosi, senza VIN (trattenuto)"),
            (a.evento(), "marcatori e cancellazione dei codici guasto"),
        ];
        if f.discovery_home_assistant {
            righe.push((
                a.modello_discovery(),
                "configurazione per Home Assistant (trattenuta)",
            ));
        }
        egui::Grid::new("anteprima_mqtt")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (argomento, descrizione) in righe {
                    ui.label(RichText::new(argomento).monospace());
                    ui.label(RichText::new(descrizione).color(GRIGIO));
                    ui.end_row();
                }
            });
    }
}
