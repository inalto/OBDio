//! Token visivi dell'interfaccia (docs/DESIGN.md, sezione 1): colori, passi
//! di spaziatura, raggi e misure dei caratteri. Sono definiti solo qui; le
//! pagine li usano senza scrivere colori o misure a mano.

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke};

// ----------------------------------------------------------------------
// Colori (tema scuro, unico). Scala Tailwind: zinc per i neutri.
// ----------------------------------------------------------------------

/// Fondo dell'app (zinc-950).
pub const SFONDO: Color32 = Color32::from_rgb(0x09, 0x09, 0x0B);
/// Pannelli, schede, strumenti (zinc-900).
pub const SUPERFICIE: Color32 = Color32::from_rgb(0x18, 0x18, 0x1B);
/// Elementi sopra una superficie, hover (zinc-800).
pub const SUPERFICIE_ALTA: Color32 = Color32::from_rgb(0x27, 0x27, 0x2A);
/// Separazione sottile fra superfici (zinc-800).
pub const BORDO: Color32 = Color32::from_rgb(0x27, 0x27, 0x2A);
/// Bordi dei controlli, tacche secondarie, arco di fondo (zinc-700).
pub const BORDO_FORTE: Color32 = Color32::from_rgb(0x3F, 0x3F, 0x46);
/// Testo e valori principali (zinc-100).
pub const TESTO: Color32 = Color32::from_rgb(0xF4, 0xF4, 0xF5);
/// Etichette, unità, testo di supporto (zinc-400).
pub const TESTO_2: Color32 = Color32::from_rgb(0xA1, 0xA1, 0xAA);
/// Note, tacche, stati inattivi (zinc-500).
pub const TESTO_3: Color32 = Color32::from_rgb(0x71, 0x71, 0x7A);
/// Unico colore d'accento: selezione, valore corrente, azione primaria (sky-400).
pub const ACCENTO: Color32 = Color32::from_rgb(0x38, 0xBD, 0xF8);
/// Stato positivo (emerald-400).
pub const OK: Color32 = Color32::from_rgb(0x34, 0xD3, 0x99);
/// Soglia di attenzione, dato mancante (amber-400).
pub const ATTENZIONE: Color32 = Color32::from_rgb(0xFB, 0xBF, 0x24);
/// Soglia critica, errori, azioni distruttive (rose-500).
pub const PERICOLO: Color32 = Color32::from_rgb(0xF4, 0x3F, 0x5E);
/// Solo per la fascia e le etichette della modalità demo/riproduzione (orange-500).
pub const DEMO: Color32 = Color32::from_rgb(0xF9, 0x73, 0x16);

/// Testo sopra un colore pieno chiaro (accento, demo, attenzione).
pub const TESTO_SU_COLORE: Color32 = SFONDO;

/// Serie dei grafici, in quest'ordine: sky, amber, emerald, pink, violet,
/// teal, orange, lime.
pub const SERIE: [Color32; 8] = [
    Color32::from_rgb(0x38, 0xBD, 0xF8),
    Color32::from_rgb(0xFB, 0xBF, 0x24),
    Color32::from_rgb(0x34, 0xD3, 0x99),
    Color32::from_rgb(0xF4, 0x72, 0xB6),
    Color32::from_rgb(0xA7, 0x8B, 0xFA),
    Color32::from_rgb(0x5E, 0xEA, 0xD4),
    Color32::from_rgb(0xFB, 0x92, 0x3C),
    Color32::from_rgb(0xA3, 0xE6, 0x35),
];

/// Colore `c` con opacità `alfa` (0–1), per velature su una superficie.
pub fn velatura(c: Color32, alfa: f32) -> Color32 {
    c.gamma_multiply(alfa.clamp(0.0, 1.0))
}

// ----------------------------------------------------------------------
// Misure
// ----------------------------------------------------------------------

/// Passi di spaziatura ammessi: 4, 8, 12, 16, 24 px.
pub const SP_1: f32 = 4.0;
pub const SP_2: f32 = 8.0;
pub const SP_3: f32 = 12.0;
pub const SP_4: f32 = 16.0;
pub const SP_6: f32 = 24.0;

/// Raggio di schede e strumenti.
pub const RAGGIO_SCHEDA: u8 = 8;
/// Raggio di pulsanti e campi.
pub const RAGGIO_CONTROLLO: u8 = 6;
/// Bordo normale.
pub const BORDO_SOTTILE: f32 = 1.0;
/// Bordo dello strumento selezionato in modifica.
pub const BORDO_SELEZIONE: f32 = 2.0;

/// Bersaglio minimo su layout largo e su layout stretto.
pub const BERSAGLIO_LARGO: f32 = 32.0;
pub const BERSAGLIO_STRETTO: f32 = 44.0;
/// Sotto questa larghezza disponibile il layout è «stretto».
pub const LARGHEZZA_STRETTA: f32 = 900.0;

/// Tipografia: nota, corpo, titolo di sezione.
pub const CARATTERE_NOTA: f32 = 12.0;
pub const CARATTERE_CORPO: f32 = 14.5;
pub const CARATTERE_TITOLO: f32 = 19.0;
/// Intervallo del valore grande degli strumenti.
pub const VALORE_MIN: f32 = 28.0;
pub const VALORE_MAX: f32 = 64.0;

/// Cifre a larghezza fissa per i numeri degli strumenti.
pub fn font_numeri(misura: f32) -> FontId {
    FontId::monospace(misura)
}

/// Testo proporzionale (titoli, unità, note degli strumenti).
pub fn font_testo(misura: f32) -> FontId {
    FontId::proportional(misura)
}

/// Bordo sottile di una superficie.
pub fn tratto_bordo() -> Stroke {
    Stroke::new(BORDO_SOTTILE, BORDO)
}

/// Applica i token ai visuali scuri di egui.
pub fn applica(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        for (stile, font) in s.text_styles.iter_mut() {
            font.size = match stile {
                egui::TextStyle::Heading => CARATTERE_TITOLO,
                egui::TextStyle::Small => CARATTERE_NOTA,
                _ => CARATTERE_CORPO,
            };
        }
        s.spacing.item_spacing = egui::vec2(SP_2, SP_2);
        s.spacing.button_padding = egui::vec2(SP_3, SP_1);
        s.spacing.interact_size.y = BERSAGLIO_LARGO - SP_1;
        // Caselle di spunta più grandi: con il raggio di 6 px una casella da
        // 14 px sembrerebbe un pulsante di opzione rotondo.
        s.spacing.icon_width = 18.0;
        s.spacing.icon_width_inner = 10.0;
        s.spacing.menu_margin = egui::Margin::same(SP_2 as i8);
        s.spacing.window_margin = egui::Margin::same(SP_4 as i8);

        let v = &mut s.visuals;
        v.dark_mode = true;
        v.override_text_color = None;
        v.panel_fill = SFONDO;
        v.window_fill = SUPERFICIE;
        v.window_stroke = Stroke::new(BORDO_SOTTILE, BORDO_FORTE);
        v.window_corner_radius = CornerRadius::same(RAGGIO_SCHEDA);
        v.menu_corner_radius = CornerRadius::same(RAGGIO_CONTROLLO);
        v.faint_bg_color = SUPERFICIE;
        v.extreme_bg_color = SUPERFICIE;
        v.text_edit_bg_color = Some(SFONDO);
        v.code_bg_color = SUPERFICIE_ALTA;
        v.hyperlink_color = ACCENTO;
        v.warn_fg_color = ATTENZIONE;
        v.error_fg_color = PERICOLO;
        v.selection.bg_fill = velatura(ACCENTO, 0.32);
        v.selection.stroke = Stroke::new(BORDO_SOTTILE, TESTO);
        v.text_cursor.stroke = Stroke::new(2.0, ACCENTO);

        let raggio = CornerRadius::same(RAGGIO_CONTROLLO);
        let w = &mut v.widgets;
        // Testo, etichette e cornici non interattive.
        w.noninteractive.bg_fill = SUPERFICIE;
        w.noninteractive.weak_bg_fill = SUPERFICIE;
        w.noninteractive.bg_stroke = Stroke::new(BORDO_SOTTILE, BORDO);
        w.noninteractive.fg_stroke = Stroke::new(BORDO_SOTTILE, TESTO);
        w.noninteractive.corner_radius = CornerRadius::same(RAGGIO_SCHEDA);
        // Controlli a riposo.
        w.inactive.bg_fill = SUPERFICIE_ALTA;
        w.inactive.weak_bg_fill = SUPERFICIE_ALTA;
        w.inactive.bg_stroke = Stroke::new(BORDO_SOTTILE, BORDO_FORTE);
        w.inactive.fg_stroke = Stroke::new(BORDO_SOTTILE, TESTO);
        w.inactive.corner_radius = raggio;
        // Sotto il puntatore.
        w.hovered.bg_fill = BORDO_FORTE;
        w.hovered.weak_bg_fill = BORDO_FORTE;
        w.hovered.bg_stroke = Stroke::new(BORDO_SOTTILE, TESTO_3);
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.hovered.corner_radius = raggio;
        w.hovered.expansion = 0.0;
        // Premuto. Il colore del testo è anche quello del testo «forte».
        w.active.bg_fill = BORDO_FORTE;
        w.active.weak_bg_fill = BORDO_FORTE;
        w.active.bg_stroke = Stroke::new(BORDO_SOTTILE, ACCENTO);
        w.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
        w.active.corner_radius = raggio;
        w.active.expansion = 0.0;
        // Menu e caselle a discesa aperti.
        w.open.bg_fill = SUPERFICIE_ALTA;
        w.open.weak_bg_fill = SUPERFICIE_ALTA;
        w.open.bg_stroke = Stroke::new(BORDO_SOTTILE, BORDO_FORTE);
        w.open.fg_stroke = Stroke::new(BORDO_SOTTILE, TESTO);
        w.open.corner_radius = raggio;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tavolozza_delle_serie_inizia_con_l_accento() {
        assert_eq!(SERIE[0], ACCENTO);
        assert_eq!(SERIE[1], ATTENZIONE);
        assert_eq!(SERIE[2], OK);
    }
}
