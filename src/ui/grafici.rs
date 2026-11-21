//! Grafici delle sonde e delle correzioni carburante su asse temporale condiviso.

use eframe::egui::{self, Color32, Stroke};
use egui_plot::{Legend, Line, LineStyle, Plot, PlotPoints, Span, VLine};

use super::dati::Archivio;
use crate::obd::pids;

const BLU: Color32 = Color32::from_rgb(79, 195, 247);
const ARANCIO: Color32 = Color32::from_rgb(255, 167, 38);
const VERDE: Color32 = Color32::from_rgb(129, 199, 132);
const ROSA: Color32 = Color32::from_rgb(240, 98, 146);
const VIOLA: Color32 = Color32::from_rgb(186, 104, 200);
const BUCO: Color32 = Color32::from_rgba_premultiplied(120, 20, 20, 70);
const MARCATORE: Color32 = Color32::from_rgb(255, 238, 88);

pub struct Opzioni<'a> {
    /// Identificativo del gruppo di grafici (assi X collegati).
    pub id: &'a str,
    pub b1s1: Option<&'a str>,
    pub b1s2: Option<&'a str>,
    /// Parametro aggiuntivo scelto dall'utente.
    pub extra: Option<&'a str>,
    /// Ampiezza della finestra scorrevole in secondi; `None` mostra tutto.
    pub finestra: Option<f64>,
    /// I dati sono simulati: lo si scrive accanto a ogni curva.
    pub sintetico: bool,
}

struct Pannello {
    unita: String,
    serie: Vec<(String, Color32)>,
}

fn unita_di(arch: &Archivio, key: &str) -> String {
    pids::per_key(key)
        .map(|p| p.unita.to_string())
        .or_else(|| arch.serie.get(key).map(|s| s.unita.clone()))
        .unwrap_or_default()
}

fn nome_di(key: &str, sintetico: bool) -> String {
    let nome = pids::per_key(key)
        .map(|p| p.nome.clone())
        .unwrap_or_else(|| key.to_string());
    if sintetico {
        format!("{nome} (sintetico)")
    } else {
        nome
    }
}

/// Disegna i pannelli impilati. Restituisce `false` se non c'è nulla da mostrare.
pub fn disegna(ui: &mut egui::Ui, arch: &Archivio, o: &Opzioni<'_>) -> bool {
    let mut pannelli: Vec<Pannello> = Vec::new();
    // Sonde: stesso pannello solo se hanno la stessa unità.
    let sonde: Vec<(&str, Color32)> = [(o.b1s1, BLU), (o.b1s2, ARANCIO)]
        .into_iter()
        .filter_map(|(k, c)| k.map(|k| (k, c)))
        .collect();
    for (key, colore) in sonde {
        let unita = unita_di(arch, key);
        match pannelli.iter_mut().find(|p| p.unita == unita) {
            Some(p) => p.serie.push((key.to_string(), colore)),
            None => pannelli.push(Pannello {
                unita,
                serie: vec![(key.to_string(), colore)],
            }),
        }
    }
    let correzioni: Vec<(String, Color32)> = [("stft_b1", VERDE), ("ltft_b1", ROSA)]
        .into_iter()
        .filter(|(k, _)| arch.serie.contains_key(*k))
        .map(|(k, c)| (k.to_string(), c))
        .collect();
    if !correzioni.is_empty() {
        pannelli.push(Pannello {
            unita: "%".into(),
            serie: correzioni,
        });
    }
    if let Some(k) = o.extra
        && arch.serie.contains_key(k)
    {
        pannelli.push(Pannello {
            unita: unita_di(arch, k),
            serie: vec![(k.to_string(), VIOLA)],
        });
    }
    pannelli.retain(|p| p.serie.iter().any(|(k, _)| arch.serie.contains_key(k)));
    if pannelli.is_empty() {
        return false;
    }

    let n = pannelli.len() as f32;
    let spazio = ui.spacing().item_spacing.y;
    let altezza = ((ui.available_height() - spazio * (n - 1.0)) / n).max(110.0);
    let da = match o.finestra {
        Some(f) => (arch.t_max - f).max(0.0),
        None => 0.0,
    };

    let visibili: Vec<String> = arch
        .marcatori
        .iter()
        .filter(|m| m.t >= da)
        .map(|m| format!("{} s «{}»", crate::model::num_it(m.t, 1), m.etichetta))
        .collect();
    if !visibili.is_empty() {
        ui.label(
            egui::RichText::new(format!(
                "Marcatori (linee tratteggiate): {}",
                visibili.join("  ·  ")
            ))
            .small()
            .color(MARCATORE),
        );
    }
    let altezza = if visibili.is_empty() {
        altezza
    } else {
        ((ui.available_height() - spazio * (n - 1.0)) / n).max(110.0)
    };

    for (i, pannello) in pannelli.iter().enumerate() {
        let ultimo = i + 1 == pannelli.len();
        let mut plot = Plot::new(format!("{}_{}", o.id, i))
            .height(altezza)
            .link_axis(o.id.to_string(), [true, false])
            .link_cursor(o.id.to_string(), [true, false])
            .legend(Legend::default().background_alpha(0.55))
            .y_axis_label(pannello.unita.clone())
            .y_axis_min_width(56.0)
            .allow_scroll(false)
            // Etichette degli assi leggibili anche con pannelli bassi.
            .grid_spacing(egui::Rangef::new(4.0, 40.0))
            .set_margin_fraction(egui::vec2(0.0, 0.08));
        if ultimo {
            plot = plot.x_axis_label("Tempo (s)");
        }
        if o.finestra.is_some() {
            // In scorrimento l'asse X è guidato dall'app.
            plot = plot
                .allow_drag([false, true])
                .allow_zoom([false, true])
                .allow_boxed_zoom(false);
        }
        plot.show(ui, |p| {
            for (key, colore) in &pannello.serie {
                let tr = arch.tracciato(key, da);
                let nome = nome_di(key, o.sintetico);
                for (a, b) in &tr.buchi {
                    p.span(
                        Span::new("Dato mancante", *a..=*b)
                            .fill(BUCO)
                            .border(Stroke::NONE),
                    );
                }
                for tratto in tr.tratti {
                    // Un punto isolato non forma una linea: lo si raddoppia.
                    let punti = if tratto.len() == 1 {
                        vec![tratto[0], [tratto[0][0] + 0.01, tratto[0][1]]]
                    } else {
                        tratto
                    };
                    p.line(
                        Line::new(nome.clone(), PlotPoints::new(punti))
                            .color(*colore)
                            .width(1.6),
                    );
                }
            }
            for m in arch.marcatori.iter().filter(|m| m.t >= da) {
                p.vline(
                    // Senza nome: i marcatori sono elencati sopra i grafici, non in legenda.
                    VLine::new("", m.t)
                        .color(MARCATORE)
                        .style(LineStyle::dashed_dense()),
                );
            }
            if let Some(f) = o.finestra {
                let fine = arch.t_max.max(f);
                p.set_plot_bounds_x((fine - f)..=fine);
            }
        });
    }
    true
}
