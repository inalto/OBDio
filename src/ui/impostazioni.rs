//! Pagina Impostazioni: pubblicazione MQTT, cartelle, informazioni.

use eframe::egui::{self, Color32, RichText, ScrollArea};

use super::componenti::{self, nota, paragrafo, riga_valore, scheda_titolata};
use super::icone;
use super::{App, GIALLO, GRIGIO, ROSSO, VERDE, apri_cartella, tema};
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

/// Riga del modulo: sul largo etichetta a sinistra e campo a destra; sullo
/// stretto etichetta sopra e campo a tutta larghezza.
fn riga_modulo(
    ui: &mut egui::Ui,
    stretto: bool,
    etichetta: &str,
    contenuto: impl FnOnce(&mut egui::Ui),
) {
    if stretto {
        if !etichetta.is_empty() {
            ui.label(RichText::new(etichetta).color(tema::TESTO_2));
        }
        contenuto(ui);
        ui.add_space(tema::SP_2);
    } else {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(LARGHEZZA_ETICHETTA, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(LARGHEZZA_ETICHETTA);
                    ui.label(RichText::new(etichetta).color(tema::TESTO_2));
                },
            );
            ui.vertical(|ui| {
                ui.set_max_width(LARGHEZZA_CAMPO);
                contenuto(ui)
            });
        });
        ui.add_space(tema::SP_1);
    }
}

/// Colonna delle etichette del modulo sul layout largo.
const LARGHEZZA_ETICHETTA: f32 = 200.0;
/// Larghezza massima dei campi di testo sul layout largo.
const LARGHEZZA_CAMPO: f32 = 340.0;

impl App {
    pub(super) fn pagina_impostazioni(&mut self, ui: &mut egui::Ui) {
        let stretto = self.disposizione.stretta();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if !stretto {
                ui.set_max_width(800.0);
            }
            scheda_titolata(ui, "Pubblicazione MQTT", |ui| {
                paragrafo(
                    ui,
                    "OBDio può pubblicare i dati letti su un broker MQTT, per esempio per Home \
                     Assistant, mentre continua a funzionare come sempre. Se il broker non è \
                     raggiungibile la lettura dei dati non rallenta: i messaggi in eccesso \
                     vengono scartati e contati.",
                );
                ui.add_space(tema::SP_3);
                self.modulo_mqtt(ui, stretto);
            });
            scheda_titolata(ui, "Stato della pubblicazione", |ui| self.stato_mqtt(ui));
            scheda_titolata(ui, "Argomenti pubblicati", |ui| {
                self.anteprima_argomenti(ui, stretto)
            });
            scheda_titolata(ui, "Sicurezza e riservatezza", |ui| {
                ui.add(
                    egui::Label::new(
                        "Il VIN non viene mai pubblicato, nemmeno se lo includi nel riepilogo \
                         della diagnosi.",
                    )
                    .wrap(),
                );
                ui.add_space(tema::SP_1);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "La connessione al broker non è cifrata: TLS non è supportato in \
                             questa versione. Utente, password e dati viaggiano in chiaro \
                             sulla rete. Usa un broker nella tua rete locale e una password \
                             che non usi altrove.",
                        )
                        .color(GIALLO),
                    )
                    .wrap(),
                );
            });
            scheda_titolata(ui, "Cartelle", |ui| {
                let cartelle = [
                    ("Sessioni registrate e importate", self.cartella.clone()),
                    (
                        "Configurazione (cruscotti, MQTT)",
                        config::cartella_configurazione(),
                    ),
                ];
                for (i, (nome, percorso)) in cartelle.into_iter().enumerate() {
                    if i > 0 {
                        ui.add_space(tema::SP_2);
                    }
                    ui.label(nome);
                    nota(ui, percorso.display().to_string());
                    if ui.button("Apri la cartella").clicked() {
                        apri_cartella(&percorso);
                    }
                }
            });
            scheda_titolata(ui, "Informazioni", |ui| {
                riga_valore(
                    ui,
                    stretto,
                    "Versione",
                    RichText::new(env!("CARGO_PKG_VERSION")),
                );
                riga_valore(
                    ui,
                    stretto,
                    "Cambiare pagina",
                    RichText::new("Ctrl+1 … Ctrl+6 (nell'ordine della barra)"),
                );
                riga_valore(ui, stretto, "Chiudere una finestra", RichText::new("Esc"));
                riga_valore(
                    ui,
                    stretto,
                    "Unità di misura",
                    RichText::new("Quelle dei parametri OBD (sistema metrico)"),
                );
                ui.add_space(tema::SP_1);
                nota(
                    ui,
                    "Sotto i 900 px di larghezza l'app passa al layout da telefono: barra di \
                     navigazione in basso e una colonna sola.",
                );
            });
            ui.add_space(tema::SP_4);
        });
    }

    fn modulo_mqtt(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let imp = &mut self.impostazioni;
        let problemi = if imp.mostra_problemi {
            imp.form.problemi()
        } else {
            Vec::new()
        };
        let f = &mut imp.form;
        riga_modulo(ui, stretto, "", |ui| {
            ui.checkbox(&mut f.attivo, "Pubblica i dati su MQTT");
        });

        riga_modulo(ui, stretto, "Indirizzo del broker", |ui| {
            con_errore(ui, &problemi, Campo::Host, |ui| {
                componenti::campo_testo(
                    ui,
                    egui::TextEdit::singleline(&mut f.host)
                        .hint_text("192.168.1.10 oppure broker.local")
                        .desired_width(f32::INFINITY),
                );
            });
        });

        riga_modulo(ui, stretto, "Porta", |ui| {
            con_errore(ui, &problemi, Campo::Porta, |ui| {
                ui.add(egui::DragValue::new(&mut f.porta).range(0..=65535));
            });
        });

        riga_modulo(ui, stretto, "Utente", |ui| {
            componenti::campo_testo(
                ui,
                egui::TextEdit::singleline(&mut f.utente)
                    .hint_text("facoltativo")
                    .desired_width(f32::INFINITY),
            );
        });

        riga_modulo(ui, stretto, "Password", |ui| {
            componenti::campo_testo(
                ui,
                egui::TextEdit::singleline(&mut f.password)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );
            ui.checkbox(
                &mut f.ricorda_password,
                "Ricorda la password (salvata in chiaro)",
            );
            if f.ricorda_password {
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "La password verrà scritta senza protezione nel file mqtt.json, \
                             leggibile da chiunque abbia accesso a questo PC.",
                        )
                        .small()
                        .color(GIALLO),
                    )
                    .wrap(),
                );
            } else {
                nota(
                    ui,
                    "Non viene salvata su disco: dopo il riavvio va inserita di nuovo.",
                );
            }
        });

        riga_modulo(ui, stretto, "Identificativo del client", |ui| {
            con_errore(ui, &problemi, Campo::IdClient, |ui| {
                componenti::campo_testo(
                    ui,
                    egui::TextEdit::singleline(&mut f.id_client)
                        .hint_text("vuoto: generato a ogni avvio")
                        .desired_width(f32::INFINITY),
                );
            });
        });

        riga_modulo(ui, stretto, "Prefisso degli argomenti", |ui| {
            con_errore(ui, &problemi, Campo::Prefisso, |ui| {
                componenti::campo_testo(
                    ui,
                    egui::TextEdit::singleline(&mut f.prefisso).desired_width(f32::INFINITY),
                );
            });
        });

        riga_modulo(ui, stretto, "Nome del veicolo", |ui| {
            componenti::campo_testo(
                ui,
                egui::TextEdit::singleline(&mut f.veicolo).desired_width(f32::INFINITY),
            );
            let pulito = f.veicolo_pulito();
            if pulito != f.veicolo {
                nota(
                    ui,
                    format!(
                        "Negli argomenti si usa «{pulito}»: sono ammessi solo a-z, 0-9, «_» e \
                         «-»."
                    ),
                );
            }
        });

        riga_modulo(ui, stretto, "Qualità del servizio", |ui| {
            con_errore(ui, &problemi, Campo::Qos, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.radio_value(&mut f.qos, 0, "0 (al più una volta)");
                    ui.radio_value(&mut f.qos, 1, "1 (almeno una volta)");
                });
            });
        });

        riga_modulo(ui, stretto, "Intervallo per sensore", |ui| {
            con_errore(ui, &problemi, Campo::IntervalloSensore, |ui| {
                ui.add(
                    egui::DragValue::new(&mut f.intervallo_sensore_ms)
                        .range(0..=3_600_000)
                        .speed(10.0)
                        .suffix(" ms"),
                );
                nota(ui, "Tempo minimo fra due messaggi dello stesso sensore.");
            });
        });

        riga_modulo(ui, stretto, "Intervallo dell'istantanea", |ui| {
            con_errore(ui, &problemi, Campo::IntervalloIstantanea, |ui| {
                ui.add(
                    egui::DragValue::new(&mut f.intervallo_istantanea_ms)
                        .range(0..=3_600_000)
                        .speed(10.0)
                        .suffix(" ms"),
                );
                nota(ui, "Tempo minimo fra due messaggi con tutti i valori.");
            });
        });

        riga_modulo(ui, stretto, "Home Assistant", |ui| {
            ui.checkbox(
                &mut f.discovery_home_assistant,
                "Annuncia i sensori (discovery)",
            );
        });

        riga_modulo(ui, stretto, "Prefisso di discovery", |ui| {
            con_errore(ui, &problemi, Campo::PrefissoDiscovery, |ui| {
                ui.add_enabled_ui(f.discovery_home_assistant, |ui| {
                    componenti::campo_testo(
                        ui,
                        egui::TextEdit::singleline(&mut f.prefisso_discovery)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
        });

        ui.add_space(tema::SP_3);
        let mut pulsanti = |ui: &mut egui::Ui| {
            if componenti::pulsante(ui, stretto, true, componenti::primario("Salva e applica"))
                .clicked()
            {
                self.applica_mqtt(false);
            }
            if componenti::pulsante(ui, stretto, true, egui::Button::new("Disattiva")).clicked() {
                self.applica_mqtt(true);
            }
        };
        if stretto {
            pulsanti(ui);
        } else {
            ui.horizontal(|ui| {
                ui.add_space(LARGHEZZA_ETICHETTA + ui.spacing().item_spacing.x);
                pulsanti(ui);
            });
        }
        if let Some((testo, errore)) = &self.impostazioni.messaggio {
            let colore = if *errore { ROSSO } else { VERDE };
            ui.add(egui::Label::new(RichText::new(testo).color(colore)).wrap());
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
        let colore = match &info.stato {
            StatoMqtt::Disattivato => GRIGIO,
            StatoMqtt::Connessione => GIALLO,
            StatoMqtt::Connesso => VERDE,
            StatoMqtt::Errore(_) => ROSSO,
        };
        ui.horizontal(|ui| {
            icone::pallino(ui, colore);
            ui.label(RichText::new(info.stato.testo()).color(colore).strong());
        });
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
            ui.add(
                egui::Label::new(RichText::new(format!("Ultimo errore: {e}")).color(ROSSO)).wrap(),
            );
        }
    }

    fn anteprima_argomenti(&self, ui: &mut egui::Ui, stretto: bool) {
        let f = &self.impostazioni.form;
        let a = Argomenti::new(f);
        nota(ui, "Con le impostazioni del modulo qui sopra:");
        ui.add_space(tema::SP_1);
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
        for (argomento, descrizione) in righe {
            if stretto {
                ui.add(egui::Label::new(RichText::new(argomento).monospace()).wrap());
                nota(ui, descrizione);
                ui.add_space(tema::SP_1);
            } else {
                ui.horizontal_top(|ui| {
                    let largo = (ui.available_width() * 0.5).min(380.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(largo, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(largo);
                            ui.add(egui::Label::new(RichText::new(argomento).monospace()).wrap());
                        },
                    );
                    ui.add(
                        egui::Label::new(RichText::new(descrizione).color(tema::TESTO_2)).wrap(),
                    );
                });
            }
        }
    }
}
