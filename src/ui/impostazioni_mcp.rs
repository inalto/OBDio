//! Scheda «Server MCP per assistenti AI» della pagina Impostazioni e
//! indicatore nella barra di stato.
//!
//! A differenza del modulo MQTT qui ogni scelta vale subito: l'interruttore,
//! il livello, la porta e «Rigenera» salvano `mcp.json` e mandano la nuova
//! configurazione al thread di lavoro.

use eframe::egui::{self, Color32, RichText, Sense, vec2};

use super::componenti::{self, nota, paragrafo};
use super::icone;
use super::impostazioni::riga_modulo;
use super::{App, GRIGIO, ROSSO, VERDE, tema};
use crate::config;
use crate::mcp::{self, Livello, McpConfig, StatoMcp, StatoMcpInfo};
use crate::worker::Comando;

/// Stato della scheda.
pub(crate) struct StatoMcpUi {
    /// Configurazione in vigore (salvata e inviata al thread di lavoro).
    pub config: McpConfig,
    /// Porta nel campo, finché non viene applicata.
    pub porta: u16,
    /// Ultimo stato comunicato dal thread di lavoro.
    pub info: StatoMcpInfo,
    /// Token mostrato in chiaro.
    pub mostra_token: bool,
    /// Esito dell'ultima azione; il secondo valore è vero se è un errore.
    pub messaggio: Option<(String, bool)>,
    /// Scorrimento da applicare una volta alla pagina (collaudo).
    pub scorri: Option<f32>,
    /// Altezza della scheda nell'ultimo fotogramma (collaudo: per scorrere
    /// fino alle schede che seguono).
    pub altezza: f32,
    /// Tiene la pagina scorsa appena oltre questa scheda (collaudo: per le
    /// schermate delle schede che seguono, su qualunque larghezza).
    pub oltre_la_scheda: bool,
}

impl StatoMcpUi {
    /// Legge `mcp.json`. Se il file è illeggibile si parte dai valori
    /// predefiniti (server spento) e lo si dice.
    pub fn carica() -> Self {
        let (config, messaggio) = match McpConfig::carica(&config::cartella_configurazione()) {
            Ok(c) => (c, None),
            Err(e) => (
                McpConfig::default(),
                Some((format!("Impostazioni del server MCP non lette: {e}"), true)),
            ),
        };
        StatoMcpUi {
            porta: config.porta,
            config,
            info: StatoMcpInfo::disattivato(),
            mostra_token: false,
            messaggio,
            scorri: None,
            altezza: 0.0,
            oltre_la_scheda: false,
        }
    }

    /// Indicatore compatto per la barra superiore; `None` se il server è spento.
    pub fn testo_barra(&self) -> Option<(String, Color32)> {
        match &self.info.stato {
            StatoMcp::Disattivato => None,
            StatoMcp::InAscolto(_) => Some(("MCP: in ascolto".into(), VERDE)),
            StatoMcp::Errore(_) => Some(("MCP: non avviato".into(), ROSSO)),
        }
    }

    /// Porta da mostrare negli indirizzi: quella effettiva se il server è in
    /// ascolto, altrimenti quella configurata.
    fn porta_effettiva(&self) -> u16 {
        match self.info.stato {
            StatoMcp::InAscolto(p) => p,
            _ => self.config.porta,
        }
    }
}

/// Ciò che l'utente ha chiesto in questo fotogramma.
enum Gesto {
    Attiva(bool),
    Livello(Livello),
    ApplicaPorta,
    Rigenera,
    MostraToken(bool),
    Copia(String, &'static str),
}

/// Interruttore a levetta con l'etichetta accanto. Restituisce la risposta
/// dell'intero controllo (levetta ed etichetta sono un solo bersaglio).
pub(super) fn interruttore(ui: &mut egui::Ui, acceso: bool, etichetta: &str) -> egui::Response {
    let alto = ui.spacing().interact_size.y;
    let pista = vec2(48.0, 26.0f32.min(alto));
    let colore_testo = if acceso { tema::TESTO } else { tema::TESTO_2 };
    let galley = ui.painter().layout_no_wrap(
        etichetta.to_string(),
        tema::font_testo(tema::CARATTERE_CORPO),
        colore_testo,
    );
    let misura = vec2(
        pista.x + tema::SP_3 + galley.size().x,
        alto.max(galley.size().y),
    );
    let (rect, resp) = ui.allocate_exact_size(misura, Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, acceso, etichetta)
    });
    if ui.is_rect_visible(rect) {
        let quanto = ui.ctx().animate_bool(resp.id, acceso);
        let r_pista = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.center().y - pista.y / 2.0),
            pista,
        );
        let p = ui.painter();
        let raggio = pista.y / 2.0;
        let fondo = if acceso {
            tema::ACCENTO
        } else if resp.hovered() {
            tema::BORDO_FORTE
        } else {
            tema::SUPERFICIE_ALTA
        };
        p.rect(
            r_pista,
            egui::CornerRadius::same(raggio as u8),
            fondo,
            egui::Stroke::new(
                tema::BORDO_SOTTILE,
                if acceso {
                    tema::ACCENTO
                } else {
                    tema::BORDO_FORTE
                },
            ),
            egui::StrokeKind::Inside,
        );
        let x = egui::lerp(
            (r_pista.left() + raggio)..=(r_pista.right() - raggio),
            quanto,
        );
        p.circle_filled(
            egui::pos2(x, r_pista.center().y),
            raggio - 4.0,
            if acceso {
                tema::TESTO_SU_COLORE
            } else {
                tema::TESTO_2
            },
        );
        if resp.has_focus() {
            p.rect_stroke(
                r_pista.expand(2.0),
                egui::CornerRadius::same(raggio as u8 + 2),
                egui::Stroke::new(tema::BORDO_SELEZIONE, tema::TESTO),
                egui::StrokeKind::Outside,
            );
        }
        p.galley(
            egui::pos2(
                r_pista.right() + tema::SP_3,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            colore_testo,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Testo a larghezza fissa in un riquadro (indirizzi, frammenti da copiare).
fn blocco_codice(ui: &mut egui::Ui, testo: &str) {
    egui::Frame::new()
        .fill(tema::SFONDO)
        .stroke(tema::tratto_bordo())
        .corner_radius(egui::CornerRadius::same(tema::RAGGIO_CONTROLLO))
        .inner_margin(egui::Margin::same(tema::SP_2 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new(testo)
                        .monospace()
                        .size(tema::CARATTERE_NOTA)
                        .color(tema::TESTO),
                )
                .wrap(),
            );
        });
}

/// Voce dell'elenco «che cosa vede e che cosa può fare».
fn voce(ui: &mut egui::Ui, titolo: &str, testo: &str) {
    componenti::testo_composto(
        ui,
        vec![
            RichText::new(titolo).color(tema::TESTO).strong(),
            RichText::new(testo).color(tema::TESTO_2),
        ],
    );
    ui.add_space(tema::SP_1);
}

impl App {
    /// La scheda intera, da mettere nella pagina Impostazioni.
    pub(super) fn scheda_mcp(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let mut gesto: Option<Gesto> = None;
        let scheda = componenti::scheda_titolata(ui, "Server MCP per assistenti AI", |ui| {
            self.contenuto_mcp(ui, stretto, &mut gesto);
        });
        self.mcp.altezza = scheda.response.rect.height();
        if let Some(g) = gesto {
            self.gesto_mcp(ui.ctx(), g);
        }
    }

    fn contenuto_mcp(&mut self, ui: &mut egui::Ui, stretto: bool, gesto: &mut Option<Gesto>) {
        let m = &mut self.mcp;
        let acceso = m.config.attivo;
        paragrafo(
            ui,
            "Un assistente AI che usa il protocollo MCP (per esempio Claude) può collegarsi a \
             OBDio su questo computer e leggere ciò che l'app sta mostrando: così gli puoi \
             chiedere di aiutarti a interpretare codici e sensori. Funziona solo mentre OBDio \
             è aperta.",
        );
        ui.add_space(tema::SP_3);

        // L'interruttore: il controllo principale della scheda.
        let etichetta = if acceso {
            "Server MCP attivo"
        } else {
            "Server MCP spento"
        };
        if interruttore(ui, acceso, etichetta)
            .on_hover_text(if acceso {
                "Spegni il server: nessun assistente potrà più collegarsi"
            } else {
                "Accendi il server su questo computer"
            })
            .clicked()
        {
            *gesto = Some(Gesto::Attiva(!acceso));
        }
        let (colore, testo_stato) = match &m.info.stato {
            StatoMcp::Disattivato => (GRIGIO, "Disattivato: nessuna porta aperta".to_string()),
            StatoMcp::InAscolto(_) => (VERDE, m.info.stato.testo()),
            StatoMcp::Errore(_) => (ROSSO, m.info.stato.testo()),
        };
        componenti::riga_testo(ui, |ui| {
            icone::pallino(ui, colore);
            ui.add(egui::Label::new(RichText::new(testo_stato).color(colore).strong()).wrap());
        });
        if let Some((testo, errore)) = &m.messaggio {
            let colore = if *errore { ROSSO } else { VERDE };
            ui.add(egui::Label::new(RichText::new(testo).color(colore)).wrap());
        }
        ui.add_space(tema::SP_3);

        riga_modulo(ui, stretto, "Che cosa può fare", |ui| {
            let mut livello = m.config.livello;
            if componenti::segmentato(
                ui,
                &mut livello,
                &[
                    (Livello::SolaLettura, Livello::SolaLettura.etichetta()),
                    (Livello::LetturaEAzioni, Livello::LetturaEAzioni.etichetta()),
                ],
            ) {
                *gesto = Some(Gesto::Livello(livello));
            }
            nota(
                ui,
                match m.config.livello {
                    Livello::SolaLettura => {
                        "L'assistente legge soltanto: stato, sensori, diagnosi, sessioni."
                    }
                    Livello::LetturaEAzioni => {
                        "L'assistente può anche avviare e fermare lettura e registrazione, \
                         aggiungere marcatori e rileggere la diagnosi."
                    }
                },
            );
        });

        riga_modulo(ui, stretto, "Porta", |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut m.porta).range(1024..=65535));
                if m.porta != m.config.porta && ui.button("Applica").clicked() {
                    *gesto = Some(Gesto::ApplicaPorta);
                }
            });
            nota(
                ui,
                "Se la porta è occupata il server non parte e lo dice: non ne sceglie un'altra.",
            );
        });

        let indirizzo = McpConfig::indirizzo_con(m.porta_effettiva());
        riga_modulo(ui, stretto, "Indirizzo", |ui| {
            blocco_codice(ui, &indirizzo);
            if componenti::pulsante(ui, stretto, true, egui::Button::new("Copia l'indirizzo"))
                .clicked()
            {
                *gesto = Some(Gesto::Copia(indirizzo.clone(), "Indirizzo copiato."));
            }
        });

        let token = m.config.token.clone();
        riga_modulo(ui, stretto, "Token di accesso", |ui| {
            let visibile = if m.mostra_token && !token.is_empty() {
                token.clone()
            } else {
                mcp::token_mascherato(&token)
            };
            blocco_codice(ui, &visibile);
            let presente = !token.is_empty();
            let mut pulsanti = |ui: &mut egui::Ui| {
                let mostra = if m.mostra_token { "Nascondi" } else { "Mostra" };
                if componenti::pulsante(ui, stretto, presente, egui::Button::new(mostra)).clicked()
                {
                    *gesto = Some(Gesto::MostraToken(!m.mostra_token));
                }
                if componenti::pulsante(ui, stretto, presente, egui::Button::new("Copia")).clicked()
                {
                    *gesto = Some(Gesto::Copia(token.clone(), "Token copiato."));
                }
                if componenti::pulsante(ui, stretto, true, egui::Button::new("Rigenera"))
                    .on_hover_text(
                        "Crea un token nuovo: i client già configurati smettono di funzionare \
                         finché non li aggiorni",
                    )
                    .clicked()
                {
                    *gesto = Some(Gesto::Rigenera);
                }
            };
            if stretto {
                pulsanti(ui);
            } else {
                ui.horizontal(|ui| pulsanti(ui));
            }
            nota(
                ui,
                "Ogni richiesta deve portare questo token. È salvato in chiaro nel file \
                 mcp.json: trattalo come una password.",
            );
        });

        riga_modulo(ui, stretto, "Attività", |ui| {
            ui.add(
                egui::Label::new(format!(
                    "Richieste servite: {} — respinte: {}",
                    m.info.richieste, m.info.rifiutate
                ))
                .wrap(),
            );
            ui.add(
                egui::Label::new(format!(
                    "Ultimo client: {}",
                    m.info.ultimo_client.as_deref().unwrap_or("nessuno finora")
                ))
                .wrap(),
            );
            if let Some(e) = &m.info.ultimo_errore
                && !matches!(m.info.stato, StatoMcp::Errore(_))
            {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Ultimo avviso: {e}")).color(tema::ATTENZIONE),
                    )
                    .wrap(),
                );
            }
        });

        ui.add_space(tema::SP_2);
        ui.label(RichText::new("Configurare il client").strong());
        nota(
            ui,
            "Per Claude Code, da incollare nel file .mcp.json del progetto (i frammenti copiati \
             contengono il token in chiaro):",
        );
        let per_vista = if m.mostra_token && !token.is_empty() {
            token.clone()
        } else {
            mcp::token_mascherato(&token)
        };
        blocco_codice(ui, &mcp::frammento_json(&indirizzo, &per_vista));
        nota(ui, "Oppure, dal terminale:");
        blocco_codice(ui, &mcp::comando_claude(&indirizzo, &per_vista));
        let presente = !token.is_empty();
        let mut pulsanti = |ui: &mut egui::Ui| {
            if componenti::pulsante(ui, stretto, presente, egui::Button::new("Copia il JSON"))
                .clicked()
            {
                *gesto = Some(Gesto::Copia(
                    mcp::frammento_json(&indirizzo, &token),
                    "Configurazione JSON copiata (contiene il token).",
                ));
            }
            if componenti::pulsante(ui, stretto, presente, egui::Button::new("Copia il comando"))
                .clicked()
            {
                *gesto = Some(Gesto::Copia(
                    mcp::comando_claude(&indirizzo, &token),
                    "Comando copiato (contiene il token).",
                ));
            }
        };
        if stretto {
            pulsanti(ui);
        } else {
            ui.horizontal(|ui| pulsanti(ui));
        }
        if !presente {
            nota(
                ui,
                "Il token viene creato la prima volta che accendi il server.",
            );
        }

        ui.add_space(tema::SP_3);
        ui.label(RichText::new("Che cosa vede e che cosa può fare").strong());
        ui.add_space(tema::SP_1);
        voce(
            ui,
            "Vede: ",
            "stato del collegamento, sensori e loro storia recente, ultima diagnosi, sessioni \
             registrate. Dice sempre se i dati vengono dal veicolo, dalla demo o da una \
             riproduzione. Il VIN non esce mai.",
        );
        voce(
            ui,
            "Con «Lettura e azioni» può anche: ",
            "avviare e fermare la lettura dei sensori e la registrazione, aggiungere marcatori, \
             chiedere alla centralina una nuova diagnosi.",
        );
        voce(
            ui,
            "Non può mai: ",
            "cancellare i codici guasto, né collegare o scollegare l'adattatore. La \
             cancellazione richiede una persona davanti allo schermo.",
        );
        voce(
            ui,
            "Solo su questo computer: ",
            "il server ascolta su 127.0.0.1 e non è raggiungibile dalla rete. Un altro \
             programma di questo PC che conosce il token può però leggere gli stessi dati.",
        );
        nota(ui, "Dettagli, esempi e problemi comuni: docs/MCP.md.");
    }

    /// Esegue ciò che l'utente ha chiesto nella scheda.
    fn gesto_mcp(&mut self, ctx: &egui::Context, g: Gesto) {
        let m = &mut self.mcp;
        match g {
            Gesto::Copia(testo, conferma) => {
                ctx.copy_text(testo);
                m.messaggio = Some((conferma.to_string(), false));
            }
            Gesto::MostraToken(mostra) => m.mostra_token = mostra,
            Gesto::Attiva(attivo) => self.attiva_mcp(attivo),
            Gesto::Livello(l) => {
                m.config.livello = l;
                self.applica_mcp();
            }
            Gesto::ApplicaPorta => {
                m.config.porta = m.porta;
                self.applica_mcp();
            }
            Gesto::Rigenera => match mcp::nuovo_token() {
                Ok(t) => {
                    m.config.token = t;
                    self.applica_mcp();
                    self.mcp.messaggio = Some((
                        "Token rigenerato: aggiorna la configurazione dei client.".into(),
                        false,
                    ));
                }
                Err(e) => m.messaggio = Some((format!("Token non rigenerato: {e}"), true)),
            },
        }
    }

    /// Accende o spegne il server. Alla prima accensione crea il token.
    pub(crate) fn attiva_mcp(&mut self, attivo: bool) {
        let m = &mut self.mcp;
        if attivo {
            m.config.porta = m.porta;
            if let Some(p) = m.config.problema_porta() {
                m.messaggio = Some((p, true));
                return;
            }
            if let Err(e) = m.config.assicura_token() {
                m.messaggio = Some((format!("Server non acceso: {e}"), true));
                return;
            }
        }
        m.config.attivo = attivo;
        self.applica_mcp();
    }

    /// Salva `mcp.json` e manda la configurazione al thread di lavoro.
    fn applica_mcp(&mut self) {
        let m = &mut self.mcp;
        m.porta = m.config.porta;
        m.messaggio = match m.config.salva(&config::cartella_configurazione()) {
            Ok(()) => None,
            Err(e) => Some((
                format!("Impostazione applicata ma non salvata su disco: {e}"),
                true,
            )),
        };
        let config = m.config.clone();
        self.invia(Comando::ConfiguraMcp(config));
    }
}
