//! Barra di trasporto della riproduzione (docs/DESIGN.md, sezione 6):
//! riproduci/pausa, salto ai marcatori, posizione trascinabile con le tacche
//! dei marcatori, velocità, ripetizione, chiusura. Sta sotto la barra
//! superiore, su ogni pagina, finché la riproduzione dura.

use eframe::egui::{self, CornerRadius, RichText, Sense, Stroke, pos2, vec2};

use super::icone::{self, Icona};
use super::{App, tema};
use crate::riproduzione::{VELOCITA, mm_ss};
use crate::worker::{Comando, ComandoRiproduzione, StatoRiproduzione};

/// Altezza del cursore della posizione.
const ALTEZZA_CURSORE: f32 = 30.0;
/// Distanza (px) entro cui il puntatore «tocca» un marcatore.
const VICINO_MARCATORE: f32 = 6.0;

/// Etichetta di un fattore di velocità: `0,5×`, `10×`.
pub(crate) fn testo_velocita(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}×")
    } else {
        format!("{v:.1}×").replace('.', ",")
    }
}

/// Marcatore precedente (con mezzo secondo di tolleranza, così si può
/// tornare indietro di uno anche appena passato) e successivo.
pub(crate) fn marcatori_vicini(st: &StatoRiproduzione, pos: f64) -> (Option<f64>, Option<f64>) {
    let prima = st
        .marcatori
        .iter()
        .map(|m| m.0)
        .filter(|t| *t < pos - 0.5)
        .fold(None, |a: Option<f64>, t| Some(a.map_or(t, |a| a.max(t))));
    let dopo = st.marcatori.iter().map(|m| m.0).find(|t| *t > pos + 0.05);
    (prima, dopo)
}

impl App {
    fn comando_trasporto(&self, c: ComandoRiproduzione) {
        self.invia(Comando::Riproduzione(c));
    }

    pub(crate) fn barra_trasporto(&mut self, ui: &mut egui::Ui) {
        let Some(st) = self.riproduzione.clone() else {
            // Il primo stato arriva subito dopo il collegamento.
            return;
        };
        let stretto = self.disposizione.stretta();
        let pos = self.trascina_posizione.unwrap_or(st.posizione);
        egui::Frame::new()
            .fill(tema::SUPERFICIE)
            .stroke(Stroke::new(
                tema::BORDO_SOTTILE,
                tema::velatura(tema::DEMO, 0.6),
            ))
            .corner_radius(CornerRadius::same(tema::RAGGIO_SCHEDA))
            .inner_margin(egui::Margin::symmetric(tema::SP_2 as i8, tema::SP_2 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if stretto {
                    // Due righe: comandi, poi tempo e cursore a tutta larghezza.
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = tema::SP_1;
                        self.comandi_trasporto(ui, &st, pos);
                        self.velocita_trasporto(ui, &st, true);
                        if icone::pulsante(
                            ui,
                            Icona::Ripeti,
                            if st.ripeti {
                                "Ripeti alla fine: attivo"
                            } else {
                                "Ripeti alla fine: spento"
                            },
                            st.ripeti,
                            true,
                        )
                        .clicked()
                        {
                            self.comando_trasporto(ComandoRiproduzione::Ripeti(!st.ripeti));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.chiudi_trasporto(ui, true);
                        });
                    });
                    ui.horizontal(|ui| {
                        self.tempo_trasporto(ui, &st, pos);
                        if let Some(t) = self.cursore(ui, &st) {
                            self.comando_trasporto(ComandoRiproduzione::VaiA(t));
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        self.comandi_trasporto(ui, &st, pos);
                        self.tempo_trasporto(ui, &st, pos);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.chiudi_trasporto(ui, false);
                            let mut ripeti = st.ripeti;
                            if ui
                                .checkbox(&mut ripeti, "Ripeti")
                                .on_hover_text("Alla fine ricomincia dall'inizio")
                                .changed()
                            {
                                self.comando_trasporto(ComandoRiproduzione::Ripeti(ripeti));
                            }
                            self.velocita_trasporto(ui, &st, false);
                            ui.add_space(tema::SP_2);
                            if let Some(t) = self.cursore(ui, &st) {
                                self.comando_trasporto(ComandoRiproduzione::VaiA(t));
                            }
                        });
                    });
                }
            });
        ui.add_space(tema::SP_2);
    }

    /// Riproduci/pausa e salto ai marcatori, come icone.
    fn comandi_trasporto(&mut self, ui: &mut egui::Ui, st: &StatoRiproduzione, pos: f64) {
        let (icona, testo, comando) = if st.finita {
            (
                Icona::Ricomincia,
                "Ricomincia",
                ComandoRiproduzione::Riprendi,
            )
        } else if st.in_pausa {
            (Icona::Riproduci, "Riproduci", ComandoRiproduzione::Riprendi)
        } else {
            (Icona::Pausa, "Pausa", ComandoRiproduzione::Pausa)
        };
        if icone::pulsante(ui, icona, testo, true, true).clicked() {
            self.comando_trasporto(comando);
        }
        let (prima, dopo) = marcatori_vicini(st, pos);
        if icone::pulsante(
            ui,
            Icona::Precedente,
            "Salta al marcatore precedente",
            false,
            prima.is_some(),
        )
        .clicked()
            && let Some(t) = prima
        {
            self.comando_trasporto(ComandoRiproduzione::VaiA(t));
        }
        if icone::pulsante(
            ui,
            Icona::Successivo,
            "Salta al marcatore successivo",
            false,
            dopo.is_some(),
        )
        .clicked()
            && let Some(t) = dopo
        {
            self.comando_trasporto(ComandoRiproduzione::VaiA(t));
        }
    }

    fn tempo_trasporto(&self, ui: &mut egui::Ui, st: &StatoRiproduzione, pos: f64) {
        ui.label(
            RichText::new(format!("{} / {}", mm_ss(pos), mm_ss(st.durata)))
                .font(tema::font_numeri(tema::CARATTERE_CORPO))
                .color(tema::TESTO),
        );
    }

    /// Velocità: casella a discesa sullo stretto, pulsanti sul largo.
    fn velocita_trasporto(&mut self, ui: &mut egui::Ui, st: &StatoRiproduzione, stretto: bool) {
        if stretto {
            let mut v = st.velocita;
            egui::ComboBox::from_id_salt("trasporto_velocita")
                .width(36.0)
                .selected_text(testo_velocita(st.velocita))
                .show_ui(ui, |ui| {
                    for x in VELOCITA {
                        ui.selectable_value(&mut v, x, testo_velocita(x));
                    }
                })
                .response
                .on_hover_text("Velocità di riproduzione");
            if v != st.velocita {
                self.comando_trasporto(ComandoRiproduzione::Velocita(v));
            }
            return;
        }
        for v in VELOCITA.iter().rev() {
            if ui
                .selectable_label(st.velocita == *v, testo_velocita(*v))
                .clicked()
            {
                self.comando_trasporto(ComandoRiproduzione::Velocita(*v));
            }
        }
        ui.label(RichText::new("Velocità").color(tema::TESTO_2));
    }

    fn chiudi_trasporto(&mut self, ui: &mut egui::Ui, stretto: bool) {
        let chiudi = if stretto {
            icone::pulsante(
                ui,
                Icona::Chiudi,
                "Chiudi riproduzione: termina e torna scollegato",
                false,
                true,
            )
            .clicked()
        } else {
            ui.button("Chiudi riproduzione")
                .on_hover_text("Termina la riproduzione e torna scollegato")
                .clicked()
        };
        if chiudi {
            self.invia(Comando::Scollega);
        }
    }

    /// Posizione trascinabile con le tacche dei marcatori. Restituisce la
    /// posizione da raggiungere quando l'utente rilascia o fa clic.
    fn cursore(&mut self, ui: &mut egui::Ui, st: &StatoRiproduzione) -> Option<f64> {
        let largo = ui.available_width().max(120.0);
        let alto = ALTEZZA_CURSORE.max(ui.spacing().interact_size.y);
        let (rect, resp) = ui.allocate_exact_size(vec2(largo, alto), Sense::click_and_drag());
        let rect = rect.shrink2(vec2(8.0, 0.0));
        let durata = st.durata.max(1e-9);
        let x_di = |t: f64| rect.left() + ((t / durata).clamp(0.0, 1.0) as f32) * rect.width();
        let t_di = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * durata;
        let mut vai = None;
        if (resp.dragged() || resp.drag_started())
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.trascina_posizione = Some(t_di(p.x));
        }
        if resp.drag_stopped() {
            vai = self.trascina_posizione.take();
        } else if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            // Un clic vicino a una tacca salta esattamente al marcatore.
            vai = Some(
                st.marcatori
                    .iter()
                    .find(|m| (x_di(m.0) - p.x).abs() <= VICINO_MARCATORE)
                    .map_or(t_di(p.x), |m| m.0),
            );
            self.trascina_posizione = None;
        }
        let pos = self.trascina_posizione.or(vai).unwrap_or(st.posizione);

        let p = ui.painter_at(rect.expand2(vec2(8.0, 0.0)));
        let y = rect.center().y;
        let pista =
            egui::Rect::from_min_max(pos2(rect.left(), y - 3.0), pos2(rect.right(), y + 3.0));
        p.rect_filled(pista, CornerRadius::same(3), tema::BORDO_FORTE);
        let fatto = egui::Rect::from_min_max(pista.min, pos2(x_di(pos), pista.max.y));
        p.rect_filled(fatto, CornerRadius::same(3), tema::ACCENTO);
        for (t, _) in &st.marcatori {
            let x = x_di(*t);
            p.line_segment(
                [pos2(x, y - 11.0), pos2(x, y + 11.0)],
                Stroke::new(2.0, tema::ATTENZIONE),
            );
        }
        let colore_maniglia = if resp.hovered() || resp.dragged() {
            egui::Color32::WHITE
        } else {
            tema::TESTO
        };
        p.circle(
            pos2(x_di(pos), y),
            8.0,
            colore_maniglia,
            Stroke::new(2.0, tema::ACCENTO),
        );
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if let Some(h) = resp.hover_pos() {
            let testo = match st
                .marcatori
                .iter()
                .find(|m| (x_di(m.0) - h.x).abs() <= VICINO_MARCATORE)
            {
                Some((t, e)) => format!("{} — marcatore «{e}»", mm_ss(*t)),
                None => format!("Vai a {}", mm_ss(t_di(h.x))),
            };
            resp.on_hover_text_at_pointer(testo);
        }
        vai
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn velocita_e_marcatori_vicini() {
        assert_eq!(testo_velocita(0.5), "0,5×");
        assert_eq!(testo_velocita(10.0), "10×");
        let st = StatoRiproduzione {
            marcatori: vec![(5.0, "a".into()), (20.0, "b".into()), (40.0, "c".into())],
            durata: 60.0,
            ..Default::default()
        };
        assert_eq!(marcatori_vicini(&st, 0.0), (None, Some(5.0)));
        assert_eq!(marcatori_vicini(&st, 20.2), (Some(5.0), Some(40.0)));
        assert_eq!(marcatori_vicini(&st, 21.0), (Some(20.0), Some(40.0)));
        assert_eq!(marcatori_vicini(&st, 50.0), (Some(40.0), None));
    }
}
