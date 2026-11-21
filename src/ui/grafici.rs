//! Grafici delle sonde e delle correzioni carburante su asse temporale condiviso.

use eframe::egui::{self, Color32, Stroke};
use egui_plot::{Legend, Line, LineStyle, Plot, PlotPoints, Span, VLine};

use super::dati::Archivio;
use super::tema;
use crate::obd::pids;

// Serie nell'ordine dei token di design (docs/DESIGN.md, sezione 1).
const BLU: Color32 = tema::SERIE[0];
const ARANCIO: Color32 = tema::SERIE[1];
const VERDE: Color32 = tema::SERIE[2];
const ROSA: Color32 = tema::SERIE[3];
const VIOLA: Color32 = tema::SERIE[4];
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
        disegna_pannello(
            ui,
            arch,
            &Riquadro {
                id_plot: format!("{}_{}", o.id, i),
                id_gruppo: o.id,
                unita: &pannello.unita,
                serie: &pannello.serie,
                altezza,
                finestra: o.finestra,
                sintetico: o.sintetico,
                asse_x: ultimo,
                legenda: true,
            },
        );
    }
    true
}

/// Parametri di un singolo riquadro con asse temporale.
struct Riquadro<'a> {
    id_plot: String,
    /// Gruppo con cui l'asse X e il cursore sono collegati.
    id_gruppo: &'a str,
    unita: &'a str,
    serie: &'a [(String, Color32)],
    altezza: f32,
    finestra: Option<f64>,
    sintetico: bool,
    /// Mostra l'etichetta «Tempo (s)».
    asse_x: bool,
    legenda: bool,
}

/// Inizio della finestra temporale visibile.
fn inizio_finestra(arch: &Archivio, finestra: Option<f64>) -> f64 {
    match finestra {
        Some(f) => (arch.t_max - f).max(0.0),
        None => 0.0,
    }
}

/// Disegna un riquadro: curve con i tratti continui, fasce rosse dove il dato
/// manca e linee dei marcatori.
fn disegna_pannello(ui: &mut egui::Ui, arch: &Archivio, pa: &Riquadro<'_>) {
    let da = inizio_finestra(arch, pa.finestra);
    let mut plot = Plot::new(&pa.id_plot)
        .height(pa.altezza)
        .link_axis(pa.id_gruppo.to_string(), [true, false])
        .link_cursor(pa.id_gruppo.to_string(), [true, false])
        .y_axis_label(pa.unita.to_string())
        .y_axis_min_width(56.0)
        .allow_scroll(false)
        // Etichette degli assi leggibili anche con pannelli bassi.
        .grid_spacing(egui::Rangef::new(4.0, 40.0))
        .set_margin_fraction(egui::vec2(0.0, 0.08));
    if pa.legenda {
        plot = plot.legend(Legend::default().background_alpha(0.55));
    }
    if pa.asse_x {
        plot = plot.x_axis_label("Tempo (s)");
    }
    if pa.finestra.is_some() {
        // In scorrimento l'asse X è guidato dall'app.
        plot = plot
            .allow_drag([false, true])
            .allow_zoom([false, true])
            .allow_boxed_zoom(false);
    }
    plot.show(ui, |p| {
        for (key, colore) in pa.serie {
            let tr = arch.tracciato(key, da);
            let nome = nome_di(key, pa.sintetico);
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
        if let Some(f) = pa.finestra {
            let fine = arch.t_max.max(f);
            p.set_plot_bounds_x((fine - f)..=fine);
        }
    });
}

/// Gruppo di assi condiviso da tutti i grafici singoli della pagina Sensori.
pub const GRUPPO_SENSORI: &str = "sensori";

/// Piccolo grafico di un solo sensore (per la scheda di «Un grafico per
/// sensore»): stessi tratti, fasce di dato mancante, marcatori, finestra e
/// asse X collegato degli altri grafici.
pub fn disegna_sensore(
    ui: &mut egui::Ui,
    arch: &Archivio,
    key: &str,
    altezza: f32,
    finestra: Option<f64>,
    sintetico: bool,
) {
    let serie = [(key.to_string(), colore_sensore(key))];
    disegna_pannello(
        ui,
        arch,
        &Riquadro {
            id_plot: format!("{GRUPPO_SENSORI}_{key}"),
            id_gruppo: GRUPPO_SENSORI,
            unita: &unita_di(arch, key),
            serie: &serie,
            altezza,
            finestra,
            sintetico,
            asse_x: false,
            legenda: false,
        },
    );
}

/// Colore stabile per sensore, scelto dalla chiave.
pub fn colore_sensore(key: &str) -> Color32 {
    const TAVOLOZZA: [Color32; 5] = [BLU, ARANCIO, VERDE, ROSA, VIOLA];
    let somma: usize = key.bytes().map(|b| b as usize).sum();
    TAVOLOZZA[somma % TAVOLOZZA.len()]
}
