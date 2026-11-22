//! Icone disegnate con il `Painter` di egui: il carattere predefinito non ha
//! molti simboli (un «●» diventa un quadratino), quindi nessuna icona dipende
//! da un glifo.
//!
//! Ogni icona è progettata su una griglia di 24 × 24 unità con il centro in
//! (12, 12), un'area utile di circa 18 unità e lo stesso tratto: a 20–24 px
//! restano nitide e della stessa grandezza ottica.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};

use super::tema;

/// Tutte le icone dell'app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icona {
    Connessione,
    Diagnosi,
    Cruscotto,
    Sensori,
    Sessioni,
    Impostazioni,
    Riproduci,
    Pausa,
    Ricomincia,
    Precedente,
    Successivo,
    Chiudi,
    Indietro,
    Ripeti,
}

/// Disegna `icona` centrata in `centro`, in un quadrato di lato `lato` px.
pub(crate) fn disegna(p: &Painter, centro: Pos2, lato: f32, colore: Color32, icona: Icona) {
    let s = lato / 24.0;
    let q = |x: f32, y: f32| centro + vec2((x - 12.0) * s, (y - 12.0) * s);
    // Il tratto cresce con l'icona ma resta di almeno 1,5 px.
    let tratto = Stroke::new(
        (tema::TRATTO_ICONA * lato / tema::LATO_ICONA).max(1.5),
        colore,
    );
    let linea = |punti: &[(f32, f32)]| {
        p.add(Shape::line(
            punti.iter().map(|&(x, y)| q(x, y)).collect(),
            tratto,
        ));
    };
    let chiusa = |punti: &[(f32, f32)]| {
        p.add(Shape::closed_line(
            punti.iter().map(|&(x, y)| q(x, y)).collect(),
            tratto,
        ));
    };
    let piena = |punti: &[(f32, f32)]| {
        p.add(Shape::convex_polygon(
            punti.iter().map(|&(x, y)| q(x, y)).collect(),
            colore,
            Stroke::new(tratto.width * 0.6, colore),
        ));
    };
    // Arco da `a0` ad `a1` (gradi, 0 = destra, senso orario sullo schermo).
    let arco = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let n = (((a1 - a0).abs() / 10.0).ceil() as usize).max(4);
        let punti: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                q(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        p.add(Shape::line(punti, tratto));
    };

    match icona {
        Icona::Connessione => {
            // Spina: due spinotti, corpo arrotondato, cavo.
            linea(&[(9.0, 3.0), (9.0, 7.5)]);
            linea(&[(15.0, 3.0), (15.0, 7.5)]);
            chiusa(&[
                (5.5, 7.5),
                (18.5, 7.5),
                (18.5, 11.0),
                (15.5, 15.5),
                (8.5, 15.5),
                (5.5, 11.0),
            ]);
            linea(&[(12.0, 15.5), (12.0, 18.5), (14.5, 21.0)]);
        }
        Icona::Diagnosi => {
            // Sagoma del motore, come la spia MIL.
            chiusa(&[
                (6.0, 8.0),
                (15.5, 8.0),
                (17.0, 10.0),
                (19.0, 10.0),
                (19.0, 8.5),
                (21.5, 8.5),
                (21.5, 17.5),
                (19.0, 17.5),
                (19.0, 16.0),
                (17.0, 16.0),
                (15.0, 18.5),
                (8.5, 18.5),
                (6.0, 16.0),
            ]);
            // Coperchio e attacco.
            linea(&[(8.0, 5.0), (13.5, 5.0)]);
            linea(&[(10.75, 5.0), (10.75, 8.0)]);
            // Supporto sinistro.
            linea(&[(2.5, 10.0), (2.5, 16.0)]);
            linea(&[(2.5, 13.0), (6.0, 13.0)]);
        }
        Icona::Cruscotto => {
            // Quadrante a 270° con lancetta e perno.
            arco(12.0, 13.0, 9.0, 135.0, 405.0);
            linea(&[(12.0, 13.0), (16.6, 8.4)]);
            p.circle_filled(q(12.0, 13.0), 1.9 * s, colore);
            // Due tacche.
            linea(&[(5.6, 13.0), (7.2, 13.0)]);
            linea(&[(12.0, 6.6), (12.0, 8.2)]);
        }
        Icona::Sensori => {
            // Forma d'onda.
            linea(&[
                (2.0, 12.5),
                (6.0, 12.5),
                (8.5, 6.0),
                (12.0, 19.0),
                (15.0, 4.5),
                (17.5, 15.0),
                (19.0, 12.5),
                (22.0, 12.5),
            ]);
        }
        Icona::Sessioni => {
            // Cartella.
            chiusa(&[
                (3.0, 5.5),
                (9.0, 5.5),
                (11.0, 8.0),
                (21.0, 8.0),
                (21.0, 19.0),
                (3.0, 19.0),
            ]);
            linea(&[(3.0, 11.0), (21.0, 11.0)]);
        }
        Icona::Impostazioni => {
            // Tre cursori: linea interrotta attorno a ogni manopola.
            for (y, x) in [(6.0, 15.5), (12.0, 8.0), (18.0, 13.0)] {
                let r = 2.3;
                linea(&[(3.0, y), (x - r, y)]);
                linea(&[(x + r, y), (21.0, y)]);
                p.circle_stroke(q(x, y), r * s, tratto);
            }
        }
        Icona::Riproduci => piena(&[(8.0, 5.0), (19.0, 12.0), (8.0, 19.0)]),
        Icona::Pausa => {
            for x in [7.0, 14.0] {
                p.rect_filled(
                    Rect::from_min_max(q(x, 5.0), q(x + 3.0, 19.0)),
                    egui::CornerRadius::same(1),
                    colore,
                );
            }
        }
        Icona::Ricomincia => {
            arco(12.0, 12.0, 7.5, -60.0, 230.0);
            // Punta della freccia all'inizio dell'arco (in alto a destra).
            let a = (-60.0f32).to_radians();
            let (x, y) = (12.0 + 7.5 * a.cos(), 12.0 + 7.5 * a.sin());
            piena(&[(x - 4.2, y - 1.6), (x + 1.2, y - 3.2), (x + 0.4, y + 2.4)]);
        }
        Icona::Precedente => {
            linea(&[(6.5, 6.0), (6.5, 18.0)]);
            piena(&[(18.0, 6.0), (18.0, 18.0), (9.0, 12.0)]);
        }
        Icona::Successivo => {
            linea(&[(17.5, 6.0), (17.5, 18.0)]);
            piena(&[(6.0, 6.0), (15.0, 12.0), (6.0, 18.0)]);
        }
        Icona::Chiudi => {
            linea(&[(6.0, 6.0), (18.0, 18.0)]);
            linea(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Icona::Indietro => linea(&[(15.0, 5.0), (8.0, 12.0), (15.0, 19.0)]),
        Icona::Ripeti => {
            // Due frecce che si rincorrono.
            linea(&[(4.0, 12.0), (4.0, 7.5), (17.0, 7.5)]);
            piena(&[(16.5, 4.0), (21.0, 7.5), (16.5, 11.0)]);
            linea(&[(20.0, 12.0), (20.0, 16.5), (7.0, 16.5)]);
            piena(&[(7.5, 13.0), (3.0, 16.5), (7.5, 20.0)]);
        }
    }
}

/// Pulsante quadrato con un'icona, del lato del bersaglio corrente.
/// `primario` lo riempie con l'accento.
pub(crate) fn pulsante(
    ui: &mut egui::Ui,
    icona: Icona,
    suggerimento: &str,
    primario: bool,
    abilitato: bool,
) -> egui::Response {
    let lato = ui.spacing().interact_size.y;
    let (rect, resp) = ui.allocate_exact_size(
        vec2(lato, lato),
        if abilitato {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact(&resp);
        let (fondo, bordo, colore) = if !abilitato {
            (
                tema::SUPERFICIE,
                tema::BORDO,
                tema::velatura(tema::TESTO_3, 0.8),
            )
        } else if primario {
            (
                if resp.hovered() {
                    tema::velatura(tema::ACCENTO, 0.85)
                } else {
                    tema::ACCENTO
                },
                tema::ACCENTO,
                tema::TESTO_SU_COLORE,
            )
        } else {
            (v.weak_bg_fill, v.bg_stroke.color, v.fg_stroke.color)
        };
        let p = ui.painter();
        p.rect(
            rect,
            egui::CornerRadius::same(tema::RAGGIO_CONTROLLO),
            fondo,
            Stroke::new(tema::BORDO_SOTTILE, bordo),
            egui::StrokeKind::Inside,
        );
        let lato_icona = (lato * 0.55).clamp(16.0, tema::LATO_ICONA);
        disegna(p, rect.center(), lato_icona, colore, icona);
    }
    let resp = resp.on_hover_text(suggerimento);
    if abilitato {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Punto colorato (indicatore di stato) allineato alla riga di testo.
pub(crate) fn pallino(ui: &mut egui::Ui, colore: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(tema::SP_2, tema::SP_4), Sense::hover());
    ui.painter()
        .circle_filled(pos2(rect.center().x, rect.center().y), 4.0, colore);
}
