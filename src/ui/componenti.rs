//! Elementi ricorrenti delle pagine, costruiti solo con i token di `tema`:
//! titoli di sezione, schede, pulsanti per ruolo, righe etichetta/valore,
//! stati vuoti, controlli segmentati.

use eframe::egui::{self, Color32, RichText};

use super::tema;

/// Cornice di una scheda: superficie, bordo da 1 px, raggio 8 px.
pub(crate) fn cornice_scheda() -> egui::Frame {
    egui::Frame::new()
        .fill(tema::SUPERFICIE)
        .stroke(tema::tratto_bordo())
        .corner_radius(egui::CornerRadius::same(tema::RAGGIO_SCHEDA))
        .inner_margin(egui::Margin::same(tema::SP_4 as i8))
}

/// Scheda a tutta larghezza con il contenuto dato.
pub(crate) fn scheda<R>(
    ui: &mut egui::Ui,
    contenuto: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let r = cornice_scheda().show(ui, |ui| {
        ui.set_width(ui.available_width());
        // Dentro `ui.columns` il layout è giustificato: qui il testo resta a
        // bandiera e i pulsanti della loro misura.
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), contenuto)
            .inner
    });
    ui.add_space(tema::SP_3);
    r
}

/// Cornice delle finestre modali: come una scheda, con margine pieno.
pub(crate) fn cornice_finestra(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.global_style())
        .fill(tema::SUPERFICIE)
        .stroke(egui::Stroke::new(tema::BORDO_SOTTILE, tema::BORDO_FORTE))
        .corner_radius(egui::CornerRadius::same(tema::RAGGIO_SCHEDA))
        .inner_margin(egui::Margin::same(tema::SP_4 as i8))
}

/// Riga di testo che va a capo, alta quanto il testo: le righe orizzontali
/// di egui partono alte quanto un bersaglio (44 px sullo stretto).
pub(crate) fn riga_testo<R>(ui: &mut egui::Ui, contenuto: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal_wrapped(contenuto).inner
    })
    .inner
}

/// Più pezzi di testo in un'unica etichetta che va a capo.
pub(crate) fn testo_composto(ui: &mut egui::Ui, parti: Vec<RichText>) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    for p in parti {
        p.append_to(
            &mut job,
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Center,
        );
    }
    ui.add(egui::Label::new(job).wrap())
}

/// Campo di testo alto quanto il bersaglio corrente.
pub(crate) fn campo_testo(ui: &mut egui::Ui, campo: egui::TextEdit<'_>) -> egui::Response {
    // In egui 0.35 `TextEdit::min_size` vale solo in larghezza: l'altezza si
    // ottiene col margine interno.
    let alto = ui.spacing().interact_size.y;
    let riga = ui.text_style_height(&egui::TextStyle::Body);
    let verticale = ((alto - riga) / 2.0).floor().clamp(2.0, 20.0) as i8;
    ui.add(campo.margin(egui::Margin::symmetric(tema::SP_2 as i8, verticale)))
}

/// Scheda con titolo (stessa misura del titolo di sezione).
pub(crate) fn scheda_titolata<R>(
    ui: &mut egui::Ui,
    titolo: &str,
    contenuto: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    scheda(ui, |ui| {
        ui.label(RichText::new(titolo).heading().color(tema::TESTO));
        ui.add_space(tema::SP_2);
        contenuto(ui)
    })
}

/// Testo di supporto (12 px, `testo_2`), sempre a capo.
pub(crate) fn nota(ui: &mut egui::Ui, testo: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(testo).small().color(tema::TESTO_2)).wrap());
}

/// Paragrafo di testo (corpo, `testo_2`).
pub(crate) fn paragrafo(ui: &mut egui::Ui, testo: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(testo).color(tema::TESTO_2)).wrap());
}

/// Pulsante dell'azione primaria della pagina (accento pieno).
pub(crate) fn primario(testo: &str) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(testo.to_string())
            .color(tema::TESTO_SU_COLORE)
            .strong(),
    )
    .fill(tema::ACCENTO)
}

/// Pulsante di un'azione distruttiva (fondo `pericolo` scuro).
pub(crate) fn pericolo(testo: &str) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(testo.to_string())
            .color(tema::TESTO_SU_SCURO)
            .strong(),
    )
    .fill(tema::PERICOLO_SCURO)
    .stroke(egui::Stroke::new(tema::BORDO_SOTTILE, tema::PERICOLO))
}

/// Pulsante distruttivo secondario (solo contorno e testo `pericolo`).
pub(crate) fn pericolo_tenue(testo: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(testo.to_string()).color(tema::PERICOLO)).stroke(
        egui::Stroke::new(tema::BORDO_SOTTILE, tema::velatura(tema::PERICOLO, 0.6)),
    )
}

/// Aggiunge un pulsante a tutta larghezza (layout stretto) o della sua
/// misura naturale (largo).
pub(crate) fn pulsante(
    ui: &mut egui::Ui,
    pieno: bool,
    abilitato: bool,
    b: egui::Button<'_>,
) -> egui::Response {
    if pieno {
        let alto = ui.spacing().interact_size.y;
        let largo = ui.available_width();
        ui.add_enabled_ui(abilitato, |ui| ui.add_sized([largo, alto], b))
            .inner
    } else {
        ui.add_enabled(abilitato, b)
    }
}

/// Riga etichetta/valore. Sul largo l'etichetta occupa una colonna fissa e
/// il valore va a capo nel resto; sullo stretto l'etichetta sta sopra il
/// valore se insieme non entrano in una riga.
pub(crate) fn riga_valore(ui: &mut egui::Ui, stretto: bool, etichetta: &str, valore: RichText) {
    let larghezza = ui.available_width();
    let colonna = if stretto {
        (larghezza * 0.52).floor()
    } else {
        200.0f32.min(larghezza * 0.4)
    };
    let testo_lungo = valore.text().chars().count() > 26;
    if stretto && testo_lungo {
        ui.add(egui::Label::new(RichText::new(etichetta).small().color(tema::TESTO_2)).wrap());
        ui.add(egui::Label::new(valore).wrap());
        ui.add_space(tema::SP_1);
        return;
    }
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(colonna, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(colonna);
                ui.add(egui::Label::new(RichText::new(etichetta).color(tema::TESTO_2)).wrap());
            },
        );
        ui.add(egui::Label::new(valore).wrap());
    });
}

/// Messaggio per un'area vuota: che cosa manca e che cosa fare. Restituisce
/// `true` se l'utente preme il pulsante dell'azione suggerita.
pub(crate) fn stato_vuoto(
    ui: &mut egui::Ui,
    titolo: &str,
    testo: &str,
    azione: Option<&str>,
) -> bool {
    let mut premuto = false;
    ui.add_space(tema::SP_6);
    ui.vertical_centered(|ui| {
        ui.set_max_width(ui.available_width().min(460.0));
        ui.label(RichText::new(titolo).heading().color(tema::TESTO));
        ui.add_space(tema::SP_1);
        ui.add(
            egui::Label::new(RichText::new(testo).color(tema::TESTO_2))
                .wrap()
                .halign(egui::Align::Center),
        );
        if let Some(a) = azione {
            ui.add_space(tema::SP_3);
            premuto = ui.add(primario(a)).clicked();
        }
    });
    ui.add_space(tema::SP_6);
    premuto
}

/// Etichetta a pastiglia (stato, «DEMO», «Parziale»): testo colorato su un
/// fondo velato dello stesso colore.
pub(crate) fn pastiglia(ui: &mut egui::Ui, testo: &str, colore: Color32) -> egui::Response {
    // Disegnata a mano: alta quanto il testo anche dentro una riga
    // orizzontale, e va a capo se non entra.
    let margine = egui::vec2(tema::SP_2, 3.0);
    let massima = (ui.available_size_before_wrap().x - 2.0 * margine.x)
        .max(ui.available_width() - 2.0 * margine.x)
        .max(40.0);
    let galley = ui.painter().layout(
        testo.to_string(),
        tema::font_testo(tema::CARATTERE_NOTA),
        colore,
        massima,
    );
    let (rect, resp) = ui.allocate_exact_size(galley.size() + 2.0 * margine, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(tema::RAGGIO_CONTROLLO),
            tema::velatura(colore, 0.16),
        );
        ui.painter().galley(rect.min + margine, galley, colore);
    }
    resp
}

/// Testo su una riga sola, troncato con «…» oltre `larghezza`.
pub(crate) fn testo_troncato(
    p: &egui::Painter,
    testo: &str,
    font: egui::FontId,
    colore: Color32,
    larghezza: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(testo.to_string(), font, colore);
    job.wrap = egui::text::TextWrapping {
        max_width: larghezza.max(8.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    p.layout_job(job)
}

/// Controllo segmentato a tutta larghezza (per esempio «Elenco | Grafici»).
/// Restituisce `true` se la scelta è cambiata.
pub(crate) fn segmentato<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    valore: &mut T,
    voci: &[(T, &str)],
) -> bool {
    let mut cambiato = false;
    let alto = ui.spacing().interact_size.y;
    let spazio = tema::SP_1;
    let n = voci.len().max(1) as f32;
    let largo = ((ui.available_width() - spazio * (n - 1.0)) / n).floor();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spazio;
        for (v, nome) in voci {
            let scelto = *valore == *v;
            let testo = if scelto {
                RichText::new(*nome).color(tema::TESTO_SU_COLORE).strong()
            } else {
                RichText::new(*nome).color(tema::TESTO)
            };
            let mut b = egui::Button::new(testo);
            if scelto {
                b = b.fill(tema::ACCENTO);
            }
            if ui.add_sized([largo, alto], b).clicked() && !scelto {
                *valore = *v;
                cambiato = true;
            }
        }
    });
    cambiato
}
