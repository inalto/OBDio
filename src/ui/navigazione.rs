//! Navigazione e cornice dell'app (docs/DESIGN.md, sezione 2): sei
//! destinazioni in una barra laterale (largo) o in basso (stretto), barra di
//! stato in alto, fasce della demo e della riproduzione, scorciatoie da
//! tastiera e conferma della chiusura con il cruscotto non salvato.

use eframe::egui::{self, Color32, RichText, Sense, vec2};

use super::componenti::{self, nota};
use super::icone::{self, Icona};
use super::{App, Scheda, tema};
use crate::riproduzione::data_leggibile;
use crate::worker::StatoConn;

impl Scheda {
    /// Le sei destinazioni, nell'ordine della navigazione e di Ctrl+1…6.
    pub(crate) const TUTTE: [Scheda; 6] = [
        Scheda::Connessione,
        Scheda::Diagnosi,
        Scheda::Cruscotto,
        Scheda::Sensori,
        Scheda::Sessioni,
        Scheda::Impostazioni,
    ];

    /// Nome nella barra laterale e titolo della pagina.
    pub(crate) fn nome(self) -> &'static str {
        match self {
            Scheda::Connessione => "Connessione",
            Scheda::Diagnosi => "Diagnosi",
            Scheda::Cruscotto => "Cruscotto",
            Scheda::Sensori => "Sensori",
            Scheda::Sessioni => "Sessioni",
            Scheda::Impostazioni => "Impostazioni",
        }
    }

    /// Nome breve per la barra in basso (al più 9 caratteri: sta in 60 px).
    pub(crate) fn nome_breve(self) -> &'static str {
        match self {
            Scheda::Connessione => "Collega",
            Scheda::Diagnosi => "Diagnosi",
            Scheda::Cruscotto => "Cruscotto",
            Scheda::Sensori => "Sensori",
            Scheda::Sessioni => "Sessioni",
            Scheda::Impostazioni => "Opzioni",
        }
    }

    pub(crate) fn icona(self) -> Icona {
        match self {
            Scheda::Connessione => Icona::Connessione,
            Scheda::Diagnosi => Icona::Diagnosi,
            Scheda::Cruscotto => Icona::Cruscotto,
            Scheda::Sensori => Icona::Sensori,
            Scheda::Sessioni => Icona::Sessioni,
            Scheda::Impostazioni => Icona::Impostazioni,
        }
    }

    /// Destinazione del tasto Ctrl+`n` (1…6).
    pub(crate) fn da_numero(n: usize) -> Option<Scheda> {
        n.checked_sub(1).and_then(|i| Scheda::TUTTE.get(i).copied())
    }
}

/// Testo breve della durata di una registrazione: `mm:ss`.
pub(crate) fn durata_breve(secondi: f64) -> String {
    let s = secondi.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

const TASTI: [egui::Key; 6] = [
    egui::Key::Num1,
    egui::Key::Num2,
    egui::Key::Num3,
    egui::Key::Num4,
    egui::Key::Num5,
    egui::Key::Num6,
];

impl App {
    /// Stato del collegamento: testo esteso, testo breve e colore.
    pub(crate) fn stato_collegamento(&self) -> (&'static str, &'static str, Color32) {
        match &self.stato {
            StatoConn::Scollegato => ("Non collegato", "Non collegato", tema::TESTO_2),
            StatoConn::Scansione => ("Ricerca Bluetooth…", "Ricerca…", tema::ATTENZIONE),
            StatoConn::Connessione(_) => ("Collegamento in corso…", "Collego…", tema::ATTENZIONE),
            StatoConn::Connesso if self.in_riproduzione() => match &self.riproduzione {
                Some(r) if r.finita => ("Riproduzione finita", "Finita", tema::DEMO),
                Some(r) if r.in_pausa => ("Riproduzione in pausa", "In pausa", tema::DEMO),
                _ => ("Riproduzione in corso", "Riproduzione", tema::DEMO),
            },
            StatoConn::Connesso => ("Collegato", "Collegato", tema::OK),
            StatoConn::Diagnosi => ("Lettura della diagnosi…", "Diagnosi…", tema::ATTENZIONE),
            StatoConn::Cancellazione => (
                "Cancellazione dei codici…",
                "Cancellazione…",
                tema::PERICOLO,
            ),
            StatoConn::Disconnessione => ("Disconnessione…", "Scollego…", tema::ATTENZIONE),
        }
    }

    /// Ctrl+1…6 cambiano destinazione.
    pub(crate) fn scorciatoie(&mut self, ctx: &egui::Context) {
        let scelta = ctx.input_mut(|i| {
            TASTI
                .iter()
                .position(|k| i.consume_key(egui::Modifiers::COMMAND, *k))
        });
        if let Some(s) = scelta.and_then(|i| Scheda::da_numero(i + 1)) {
            self.scheda = s;
        }
    }

    // ------------------------------------------------------------------
    // Barre di navigazione
    // ------------------------------------------------------------------

    /// Barra laterale (layout largo): nome dell'app, poi una voce per
    /// destinazione con icona e nome.
    pub(crate) fn barra_laterale(&mut self, ui: &mut egui::Ui) {
        ui.add_space(tema::riserva_superiore());
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), tema::ALTEZZA_BARRA_STATO),
            Sense::hover(),
        );
        ui.painter().text(
            rect.left_center() + vec2(tema::SP_3, 0.0),
            egui::Align2::LEFT_CENTER,
            "OBDio",
            egui::FontId::proportional(tema::CARATTERE_TITOLO),
            tema::TESTO,
        );
        ui.add_space(tema::SP_2);
        for (i, s) in Scheda::TUTTE.into_iter().enumerate() {
            let scelta = self.scheda == s;
            let (rect, resp) = ui.allocate_exact_size(
                vec2(ui.available_width(), tema::BERSAGLIO_STRETTO),
                Sense::click(),
            );
            let p = ui.painter();
            let raggio = egui::CornerRadius::same(tema::RAGGIO_CONTROLLO);
            if scelta {
                p.rect_filled(rect, raggio, tema::velatura(tema::ACCENTO, 0.14));
                let barra = egui::Rect::from_min_size(
                    rect.left_top() + vec2(0.0, tema::SP_2 + 2.0),
                    vec2(3.0, rect.height() - 2.0 * (tema::SP_2 + 2.0)),
                );
                p.rect_filled(barra, egui::CornerRadius::same(2), tema::ACCENTO);
            } else if resp.hovered() {
                p.rect_filled(rect, raggio, tema::SUPERFICIE_ALTA);
            }
            let (colore_icona, colore_testo) = if scelta {
                (tema::ACCENTO, tema::ACCENTO)
            } else {
                (tema::TESTO_2, tema::TESTO)
            };
            let x_icona = rect.left() + tema::SP_3 + tema::LATO_ICONA / 2.0;
            icone::disegna(
                p,
                egui::pos2(x_icona, rect.center().y),
                tema::LATO_ICONA,
                colore_icona,
                s.icona(),
            );
            p.text(
                egui::pos2(
                    x_icona + tema::LATO_ICONA / 2.0 + tema::SP_3,
                    rect.center().y,
                ),
                egui::Align2::LEFT_CENTER,
                s.nome(),
                egui::FontId::proportional(tema::CARATTERE_CORPO),
                colore_testo,
            );
            let resp = resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("Ctrl+{}", i + 1));
            if resp.clicked() {
                self.scheda = s;
            }
            ui.add_space(tema::SP_1);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(tema::SP_3);
            ui.horizontal(|ui| {
                ui.add_space(tema::SP_3);
                ui.label(
                    RichText::new(concat!("v", env!("CARGO_PKG_VERSION"), " · Ctrl+1…6"))
                        .small()
                        .color(tema::TESTO_3),
                );
            });
        });
    }

    /// Barra in basso (layout stretto): sei voci uguali con icona e nome breve.
    pub(crate) fn barra_inferiore(&mut self, ui: &mut egui::Ui) {
        let tutto = ui.available_rect_before_wrap();
        let largo = tutto.width() / Scheda::TUTTE.len() as f32;
        for (i, s) in Scheda::TUTTE.into_iter().enumerate() {
            let rect = egui::Rect::from_min_size(
                tutto.left_top() + vec2(i as f32 * largo, 0.0),
                vec2(largo, tema::ALTEZZA_BARRA_NAV),
            );
            let resp = ui.interact(rect, ui.id().with(("nav", i)), Sense::click());
            let scelta = self.scheda == s;
            let p = ui.painter();
            let centro_icona = egui::pos2(rect.center().x, rect.top() + 22.0);
            if scelta {
                let pillola = egui::Rect::from_center_size(
                    centro_icona,
                    vec2((largo - tema::SP_2).min(56.0), 30.0),
                );
                p.rect_filled(
                    pillola,
                    egui::CornerRadius::same(15),
                    tema::velatura(tema::ACCENTO, 0.18),
                );
            } else if resp.hovered() {
                p.rect_filled(
                    rect.shrink(tema::SP_1),
                    egui::CornerRadius::same(tema::RAGGIO_CONTROLLO),
                    tema::SUPERFICIE_ALTA,
                );
            }
            let colore = if scelta { tema::ACCENTO } else { tema::TESTO_2 };
            icone::disegna(p, centro_icona, tema::LATO_ICONA, colore, s.icona());
            p.text(
                egui::pos2(rect.center().x, rect.top() + 48.0),
                egui::Align2::CENTER_CENTER,
                s.nome_breve(),
                egui::FontId::proportional(tema::CARATTERE_NOTA),
                if scelta { tema::ACCENTO } else { tema::TESTO_2 },
            );
            if resp.clicked() {
                self.scheda = s;
            }
        }
        ui.allocate_rect(tutto, Sense::hover());
    }

    // ------------------------------------------------------------------
    // Barra di stato e fasce
    // ------------------------------------------------------------------

    /// Barra in alto: titolo della pagina, stato del collegamento,
    /// registrazione, MQTT, server MCP; sotto, le fasce di demo, riproduzione ed errore.
    pub(crate) fn barra_superiore(&mut self, ui: &mut egui::Ui) {
        let stretto = self.disposizione.stretta();
        ui.add_space(tema::riserva_superiore());
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), tema::ALTEZZA_BARRA_STATO),
            Sense::hover(),
        );
        let mut riga = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        riga.label(
            RichText::new(self.scheda.nome())
                .heading()
                .color(tema::TESTO),
        );
        let mut apri_impostazioni = false;
        #[cfg(not(target_os = "android"))]
        let mut apri_aggiornamenti = false;
        riga.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = tema::SP_1;
            let (lungo, breve, colore) = self.stato_collegamento();
            let testo = if stretto { breve } else { lungo };
            ui.label(RichText::new(testo).color(colore));
            icone::pallino(ui, colore);
            if let Some(r) = &self.registrazione {
                ui.add_space(tema::SP_3);
                let durata = durata_breve(self.live.t_max - r.t_inizio);
                let testo = if stretto {
                    format!("REC {durata}")
                } else {
                    format!("Registrazione {durata}")
                };
                ui.label(RichText::new(testo).color(tema::PERICOLO).strong())
                    .on_hover_text("Registrazione in corso");
                icone::pallino(ui, tema::PERICOLO);
            } else if self.polling && !stretto {
                ui.add_space(tema::SP_3);
                ui.label(RichText::new("Lettura attiva").color(tema::TESTO_2));
                icone::pallino(ui, tema::OK);
            }
            // MQTT solo se resta posto per il titolo.
            if let Some((testo, colore)) = self.impostazioni.testo_barra()
                && (!stretto || ui.available_width() > 220.0)
            {
                ui.add_space(tema::SP_3);
                let testo = if stretto { "MQTT".to_string() } else { testo };
                ui.label(RichText::new(testo).color(tema::TESTO_2));
                icone::pallino(ui, colore);
            }
            // Server MCP: come MQTT, solo se acceso e se resta posto. Un clic
            // porta alle Impostazioni, dove sta l'interruttore.
            if let Some((testo, colore)) = self.mcp.testo_barra()
                && (!stretto || ui.available_width() > 220.0)
            {
                ui.add_space(tema::SP_3);
                let testo = if stretto { "MCP".to_string() } else { testo };
                let etichetta = egui::Label::new(RichText::new(testo).color(tema::TESTO_2))
                    .sense(Sense::click());
                if ui
                    .add(etichetta)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Server MCP per assistenti AI: apri le Impostazioni")
                    .clicked()
                {
                    apri_impostazioni = true;
                }
                icone::pallino(ui, colore);
            }
            // Versione nuova: un avviso discreto, mai una finestra. Un clic
            // porta alla scheda Aggiornamenti.
            #[cfg(not(target_os = "android"))]
            if let Some((largo, breve, colore)) = self.aggiornamenti.testo_barra()
                && (!stretto || ui.available_width() > 100.0)
            {
                ui.add_space(tema::SP_3);
                let testo = if stretto { breve } else { largo };
                let etichetta = egui::Label::new(RichText::new(testo).color(tema::TESTO_2))
                    .sense(Sense::click());
                if ui
                    .add(etichetta)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Aggiornamento di OBDio: apri la scheda nelle Impostazioni")
                    .clicked()
                {
                    apri_aggiornamenti = true;
                }
                icone::pallino(ui, colore);
            }
        });
        if apri_impostazioni {
            self.scheda = Scheda::Impostazioni;
        }
        #[cfg(not(target_os = "android"))]
        if apri_aggiornamenti {
            self.scheda = Scheda::Impostazioni;
            self.aggiornamenti.scorri_alla_scheda = true;
        }
        self.fasce(ui);
    }

    /// Vero se nella pagina corrente si vedono dati della demo.
    fn demo_in_vista(&self) -> bool {
        match self.scheda {
            Scheda::Sessioni => self
                .aperta
                .as_ref()
                .is_some_and(|s| s.dati.intestazione.demo),
            Scheda::Sensori | Scheda::Cruscotto => {
                self.info.as_ref().is_some_and(|i| i.demo)
                    || (self.live_demo && !self.live.serie.is_empty())
            }
            Scheda::Diagnosi => {
                self.info.as_ref().is_some_and(|i| i.demo)
                    || self.diagnosi.as_ref().is_some_and(|d| d.demo)
            }
            Scheda::Connessione => self.info.as_ref().is_some_and(|i| i.demo),
            Scheda::Impostazioni => false,
        }
    }

    /// Fascia arancione (demo o riproduzione), barra di trasporto, errore.
    fn fasce(&mut self, ui: &mut egui::Ui) {
        let stretto = self.disposizione.stretta();
        // Un'unica etichetta: titolo in grassetto e spiegazione, a capo dove serve.
        let fascia = |ui: &mut egui::Ui, titolo: String, testo: String| {
            egui::Frame::new()
                .fill(tema::DEMO)
                .corner_radius(egui::CornerRadius::same(tema::RAGGIO_CONTROLLO))
                .inner_margin(egui::Margin::symmetric(tema::SP_3 as i8, tema::SP_2 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    componenti::testo_composto(
                        ui,
                        vec![
                            RichText::new(titolo).color(tema::TESTO_SU_COLORE).strong(),
                            RichText::new(testo).color(tema::TESTO_SU_COLORE),
                        ],
                    );
                });
            ui.add_space(tema::SP_2);
        };
        if let Some(r) = self.info.as_ref().and_then(|i| i.riproduzione.clone()) {
            // Per tutta la durata della riproduzione, su ogni pagina.
            let origine = r
                .importata
                .clone()
                .map(|i| format!("importata: {i}"))
                .unwrap_or_else(|| r.file.clone());
            fascia(
                ui,
                format!(
                    "RIPRODUZIONE — dati registrati il {}{}",
                    data_leggibile(&r.registrata_il),
                    if r.sessione_demo {
                        ", sessione demo"
                    } else {
                        ""
                    }
                ),
                if stretto {
                    " · non sono letture attuali".to_string()
                } else {
                    format!(" · non sono letture attuali · {origine}")
                },
            );
            self.barra_trasporto(ui);
        } else if self.demo_in_vista() {
            fascia(
                ui,
                "MODALITÀ DEMO".into(),
                if stretto {
                    " — dati simulati, non da un veicolo".into()
                } else if self.info.as_ref().is_some_and(|i| i.td5) {
                    " — tutti i dati sono simulati: adattatore OBDLink e centralina Td5 \
                     sintetici, non un veicolo"
                        .into()
                } else {
                    " — tutti i dati sono simulati; i segnali delle sonde lambda sono sintetici \
                     e non provengono da un veicolo"
                        .into()
                },
            );
        }
        if let Some(e) = self.errore.clone() {
            let mut chiudi = false;
            egui::Frame::new()
                .fill(tema::FONDO_ERRORE)
                .stroke(egui::Stroke::new(tema::BORDO_SOTTILE, tema::PERICOLO))
                .corner_radius(egui::CornerRadius::same(tema::RAGGIO_CONTROLLO))
                .inner_margin(egui::Margin::symmetric(tema::SP_3 as i8, tema::SP_2 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        let lato = ui.spacing().interact_size.y;
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width() - lato - tema::SP_2, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.label(RichText::new(e.titolo()).strong().color(tema::TESTO));
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(e.messaggio()).color(tema::TESTO),
                                    )
                                    .wrap(),
                                );
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            if icone::pulsante(
                                ui,
                                Icona::Chiudi,
                                "Chiudi il messaggio",
                                false,
                                true,
                            )
                            .clicked()
                            {
                                chiudi = true;
                            }
                        });
                    });
                });
            ui.add_space(tema::SP_2);
            if chiudi {
                self.errore = None;
            }
        }
        self.avviso_chiusura_per_errore(ui, stretto);
    }

    /// Avviso, senza bloccare nulla, che l'ultima esecuzione è finita con un
    /// errore registrato (vedi [`crate::registri::errori_dall_ultima_chiusura`]).
    fn avviso_chiusura_per_errore(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let Some(avviso) = self.avviso_errore.clone() else {
            return;
        };
        let mut chiudi = false;
        egui::Frame::new()
            .fill(tema::velatura(tema::ATTENZIONE, 0.12))
            .stroke(egui::Stroke::new(tema::BORDO_SOTTILE, tema::ATTENZIONE))
            .corner_radius(egui::CornerRadius::same(tema::RAGGIO_CONTROLLO))
            .inner_margin(egui::Margin::symmetric(tema::SP_3 as i8, tema::SP_2 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("L'ultima volta OBDio si è chiuso per un errore")
                        .strong()
                        .color(tema::TESTO),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "I dettagli sono nel registro degli errori. Se il problema si \
                             ripete, invia il registro (e quello dei collegamenti, nella stessa \
                             cartella) a chi ti assiste.",
                        )
                        .color(tema::TESTO),
                    )
                    .wrap(),
                );
                if !stretto {
                    componenti::nota(ui, avviso.file.display().to_string());
                }
                ui.add_space(tema::SP_1);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Apri il registro").clicked() {
                        crate::piattaforma::apri_file(&avviso.file);
                    }
                    if ui.button("Apri la cartella").clicked()
                        && let Some(d) = avviso.file.parent()
                    {
                        crate::piattaforma::apri_cartella(d);
                    }
                    if ui.button("Chiudi").clicked() {
                        chiudi = true;
                    }
                });
            });
        ui.add_space(tema::SP_2);
        if chiudi {
            crate::registri::segna_errori_visti();
            self.avviso_errore = None;
        }
    }

    // ------------------------------------------------------------------
    // Chiusura con il cruscotto non salvato
    // ------------------------------------------------------------------

    /// Intercetta la chiusura della finestra se il cruscotto ha modifiche non
    /// salvate e chiede che cosa fare.
    pub(crate) fn governa_chiusura(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && self.cruscotto.non_salvato()
            && !self.chiusura_confermata
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.chiedi_chiusura = true;
        }
        if !self.chiedi_chiusura {
            return;
        }
        let stretto = self.disposizione.stretta();
        let largo = larghezza_finestra(ctx, 460.0);
        let mut scelta: Option<u8> = None;
        egui::Modal::new(egui::Id::new("conferma_chiusura"))
            .frame(componenti::cornice_finestra(ctx))
            .backdrop_color(egui::Color32::from_black_alpha(170))
            .show(ctx, |ui| {
                ui.set_width(largo);
                ui.label(
                    RichText::new("Salvare le modifiche al cruscotto?")
                        .heading()
                        .color(tema::TESTO),
                );
                ui.add_space(tema::SP_2);
                componenti::paragrafo(
                    ui,
                    "Hai cambiato il cruscotto senza salvare. Se chiudi senza salvare, le \
                 modifiche vanno perse.",
                );
                if let Some((m, colore)) = &self.cruscotto.messaggio
                    && *colore == tema::PERICOLO
                {
                    ui.label(RichText::new(m).color(tema::PERICOLO));
                }
                ui.add_space(tema::SP_4);
                let mut pulsanti = |ui: &mut egui::Ui| {
                    if componenti::pulsante(ui, stretto, true, componenti::primario("Salva"))
                        .clicked()
                    {
                        scelta = Some(0);
                    }
                    if componenti::pulsante(
                        ui,
                        stretto,
                        true,
                        componenti::pericolo_tenue("Non salvare"),
                    )
                    .clicked()
                    {
                        scelta = Some(1);
                    }
                    if componenti::pulsante(ui, stretto, true, egui::Button::new("Annulla"))
                        .clicked()
                    {
                        scelta = Some(2);
                    }
                };
                if stretto {
                    pulsanti(ui);
                } else {
                    ui.horizontal(|ui| pulsanti(ui));
                }
                nota(ui, "Esc annulla la chiusura.");
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            scelta = Some(2);
        }
        match scelta {
            Some(0) => {
                self.cruscotto.salva();
                if !self.cruscotto.non_salvato() {
                    self.chiusura_confermata = true;
                    self.chiedi_chiusura = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Some(1) => {
                self.chiusura_confermata = true;
                self.chiedi_chiusura = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(_) => {
                self.chiedi_chiusura = false;
                // Chiusura annullata: niente riavvio dopo un aggiornamento.
                #[cfg(not(target_os = "android"))]
                {
                    self.aggiornamenti.riavvio_richiesto = false;
                }
            }
            None => {}
        }
    }
}

/// Larghezza del contenuto di una finestra modale: al più `massima`, e mai
/// oltre lo schermo (margini e cornice della finestra compresi).
pub(crate) fn larghezza_finestra(ctx: &egui::Context, massima: f32) -> f32 {
    larghezza_modale(ctx.content_rect().width(), massima)
}

/// Versione pura di [`larghezza_finestra`].
pub(crate) fn larghezza_modale(schermo: f32, massima: f32) -> f32 {
    // Margine esterno (12 per lato) e cornice interna della finestra (16 per lato).
    (schermo - 2.0 * tema::SP_3 - 2.0 * tema::SP_4).clamp(200.0, massima)
}

/// Altezza massima del contenuto scorrevole di una finestra modale, lasciati
/// `riservato` px a titolo e pulsanti che non scorrono.
pub(crate) fn altezza_modale(ctx: &egui::Context, riservato: f32) -> f32 {
    altezza_scorrevole(ctx.content_rect().height(), riservato)
}

/// Versione pura di [`altezza_modale`].
pub(crate) fn altezza_scorrevole(schermo: f32, riservato: f32) -> f32 {
    (schermo - 2.0 * tema::SP_3 - 2.0 * tema::SP_4 - riservato).max(120.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sei_destinazioni_in_ordine() {
        assert_eq!(Scheda::TUTTE.len(), 6);
        let nomi: Vec<&str> = Scheda::TUTTE.iter().map(|s| s.nome()).collect();
        assert_eq!(
            nomi,
            [
                "Connessione",
                "Diagnosi",
                "Cruscotto",
                "Sensori",
                "Sessioni",
                "Impostazioni"
            ]
        );
        // Ctrl+1…6 seguono lo stesso ordine; 0 e 7 non portano da nessuna parte.
        for (i, s) in Scheda::TUTTE.iter().enumerate() {
            assert_eq!(Scheda::da_numero(i + 1), Some(*s));
        }
        assert_eq!(Scheda::da_numero(0), None);
        assert_eq!(Scheda::da_numero(7), None);
    }

    #[test]
    fn nomi_brevi_e_icone_distinti() {
        let mut brevi: Vec<&str> = Scheda::TUTTE.iter().map(|s| s.nome_breve()).collect();
        assert!(brevi.iter().all(|n| n.chars().count() <= 9), "{brevi:?}");
        brevi.sort();
        brevi.dedup();
        assert_eq!(brevi.len(), 6);
        let mut icone: Vec<String> = Scheda::TUTTE
            .iter()
            .map(|s| format!("{:?}", s.icona()))
            .collect();
        icone.sort();
        icone.dedup();
        assert_eq!(icone.len(), 6);
    }

    #[test]
    fn finestre_modali_entrano_nello_schermo() {
        // Su un telefono da 360 px la finestra (più 2 × 12 di margine e
        // 2 × 16 di cornice) non supera lo schermo.
        let l = larghezza_modale(360.0, 580.0);
        assert!(l + 2.0 * 12.0 + 2.0 * 16.0 <= 360.0, "{l}");
        assert_eq!(larghezza_modale(1280.0, 580.0), 580.0);
        // Sul telefono la spiegazione della cancellazione scorre, ma resta
        // visibile per almeno metà schermo.
        assert!(altezza_scorrevole(844.0, 330.0) >= 400.0);
        assert_eq!(altezza_scorrevole(500.0, 600.0), 120.0);
        assert_eq!(durata_breve(65.4), "01:05");
        assert_eq!(durata_breve(-3.0), "00:00");
    }
}
