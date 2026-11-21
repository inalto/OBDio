//! Disegno dei sette tipi di strumento con il `Painter` di egui
//! (docs/DESIGN.md, sezione 3). Ogni strumento disegna la propria scheda,
//! adatta caratteri e spessori alla misura in pixel e non disegna mai una
//! lancetta, un riempimento o un numero per un dato che manca.

use std::f32::consts::PI;
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Galley, Painter, Pos2, Rect, Shape, Stroke,
    StrokeKind, Vec2, epaint::Mesh, pos2, vec2,
};

use super::modello::{Livello, Strumento, TipoStrumento};
use super::sorgenti::StatoValore;
use crate::model::{Stato, num_it, num_it_segno};
use crate::obd::pids;
use crate::summary::numero_param;
use crate::ui::dati::Archivio;
use crate::ui::tema::{self, font_numeri, font_testo};

/// Durata dello smorzamento della lancetta.
const SMORZAMENTO_S: f32 = 0.15;

/// Stato di una spia, già calcolato dalla pagina.
#[derive(Clone, Debug, PartialEq)]
pub struct StatoSpia {
    /// `None` se lo stato non è noto (nessuna diagnosi letta, dato mancante).
    pub accesa: Option<bool>,
    pub colore: Color32,
    pub testo: String,
}

/// Tutto ciò che serve per disegnare uno strumento.
pub struct Vista<'a> {
    pub id: egui::Id,
    /// Strumento con la scala già risolta.
    pub s: &'a Strumento,
    pub titolo: &'a str,
    /// Parametro risolto (vuoto per le spie di stato).
    pub chiave: &'a str,
    pub unita: &'a str,
    pub valore: StatoValore,
    pub picco: Option<f64>,
    pub min_sessione: Option<f64>,
    pub max_sessione: Option<f64>,
    /// Contachilometri sostituito dalla distanza parziale calcolata.
    pub parziale: bool,
    pub spia: Option<StatoSpia>,
    pub archivio: &'a Archivio,
}

// ----------------------------------------------------------------------
// Logica pura: passi della scala, formati
// ----------------------------------------------------------------------

/// Passo «tondo» delle tacche principali (1, 2, 2,5, 5 × 10ⁿ) per avere fra
/// 5 e `max_tacche` tacche numerate sulla scala; restituisce anche il numero
/// di suddivisioni secondarie fra due tacche principali.
pub fn passo_tacche(min: f64, max: f64, max_tacche: usize) -> (f64, u32) {
    let ampiezza = (max - min).abs();
    if !ampiezza.is_finite() || ampiezza <= 0.0 {
        return (1.0, 1);
    }
    let max_tacche = max_tacche.max(3);
    let min_tacche = 5.min(max_tacche);
    let conta = |passo: f64| {
        let primo = (min / passo - 1e-9).ceil();
        let ultimo = (max / passo + 1e-9).floor();
        (ultimo - primo + 1.0).max(0.0) as usize
    };
    let mut esponente = (ampiezza / max_tacche as f64).log10().floor() as i32 - 1;
    let mut ripiego = None;
    for _ in 0..6 {
        for m in [1.0, 2.0, 2.5, 5.0] {
            let passo = m * 10f64.powi(esponente);
            let n = conta(passo);
            if n <= max_tacche {
                let sotto = if m == 2.0 { 4 } else { 5 };
                if n >= min_tacche {
                    return (passo, sotto);
                }
                ripiego.get_or_insert((passo, sotto));
            }
        }
        esponente += 1;
    }
    ripiego.unwrap_or((ampiezza, 1))
}

/// Decimali necessari per scrivere i multipli di `passo`.
pub fn decimali_passo(passo: f64) -> usize {
    for d in 0..6 {
        let scalato = passo * 10f64.powi(d as i32);
        if (scalato - scalato.round()).abs() < 1e-6 {
            return d;
        }
    }
    6
}

/// Numero di un parametro con i decimali dello strumento (o del parametro).
pub fn formatta(chiave: &str, v: f64, decimali: Option<usize>) -> String {
    match decimali {
        None => numero_param(chiave, v),
        Some(d) => {
            if pids::per_key(chiave).is_some_and(|p| p.con_segno) {
                num_it_segno(v, d)
            } else {
                num_it(v, d)
            }
        }
    }
}

/// Etichetta di un estremo della scala: senza segno forzato e senza zeri
/// decimali superflui («−25», «100», «0,7»).
pub fn etichetta_scala(v: f64) -> String {
    num_it(v, decimali_passo(v).min(3))
}

/// Posizione di `v` sulla scala, fra 0 e 1.
pub fn frazione(v: f64, min: f64, max: f64) -> f32 {
    if max <= min {
        return 0.0;
    }
    (((v - min) / (max - min)).clamp(0.0, 1.0)) as f32
}

/// Origine del riempimento: lo zero se la scala lo contiene, altrimenti il minimo.
pub fn origine(min: f64, max: f64) -> f64 {
    if min < 0.0 && max > 0.0 { 0.0 } else { min }
}

/// Testo e colore della nota di stato, se lo stato ne richiede una.
pub fn nota_stato(v: &StatoValore) -> Option<(String, Color32)> {
    match *v {
        StatoValore::Valido(_) => None,
        StatoValore::InAttesa => Some(("In attesa del dato".into(), tema::TESTO_3)),
        StatoValore::Vecchio { secondi, .. } => Some((
            format!("fermo da {} s", num_it(secondi.floor(), 0)),
            tema::TESTO_3,
        )),
        StatoValore::Mancante(Stato::ErroreComunicazione) => {
            Some(("Errore di comunicazione".into(), tema::PERICOLO))
        }
        StatoValore::Mancante(_) => Some(("La centralina non risponde".into(), tema::ATTENZIONE)),
        StatoValore::NonSupportato => {
            Some(("Non supportato da questa centralina".into(), tema::TESTO_2))
        }
    }
}

// ----------------------------------------------------------------------
// Aiuti di disegno
// ----------------------------------------------------------------------

fn colore_livello(l: Livello) -> Option<Color32> {
    match l {
        Livello::Normale => None,
        Livello::Attenzione => Some(tema::ATTENZIONE),
        Livello::Critico => Some(tema::PERICOLO),
    }
}

impl Vista<'_> {
    fn livello(&self) -> Livello {
        match self.valore {
            StatoValore::Valido(v) => self.s.livello(v),
            _ => Livello::Normale,
        }
    }

    fn vecchio(&self) -> bool {
        matches!(self.valore, StatoValore::Vecchio { .. })
    }

    /// Colore del numero: zona se superata, attenuato se vecchio.
    fn colore_numero(&self) -> Color32 {
        if self.vecchio() {
            return tema::TESTO_3;
        }
        colore_livello(self.livello()).unwrap_or(tema::TESTO)
    }

    /// Colore dell'arco o della barra del valore.
    fn colore_valore(&self) -> Color32 {
        if self.vecchio() {
            return tema::TESTO_3;
        }
        colore_livello(self.livello()).unwrap_or(tema::ACCENTO)
    }

    fn testo_numero(&self) -> String {
        match self.valore.valore() {
            Some(v) => formatta(self.chiave, v, self.s.decimali),
            None => "—".into(),
        }
    }

    fn numero(&self, v: f64) -> String {
        formatta(self.chiave, v, self.s.decimali)
    }

    fn estremo(&self, v: f64) -> String {
        etichetta_scala(v)
    }

    /// Frazione della lancetta, smorzata verso il valore nuovo.
    fn frazione_smorzata(&self, ctx: &egui::Context) -> Option<f32> {
        let v = self.valore.valore()?;
        let f = frazione(v, self.s.min, self.s.max);
        Some(ctx.animate_value_with_time(self.id.with("lancetta"), f, SMORZAMENTO_S))
    }
}

/// Testo su una riga, accorciato con «…» se non entra in `larghezza`.
fn elidi(p: &Painter, testo: &str, font: FontId, colore: Color32, larghezza: f32) -> Arc<Galley> {
    let g = p.layout_no_wrap(testo.to_string(), font.clone(), colore);
    if g.size().x <= larghezza || testo.is_empty() {
        return g;
    }
    let caratteri: Vec<char> = testo.chars().collect();
    let (mut lo, mut hi) = (0usize, caratteri.len());
    while lo < hi {
        let m = (lo + hi).div_ceil(2);
        let prova: String = caratteri[..m]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
            + "…";
        if p.layout_no_wrap(prova, font.clone(), colore).size().x <= larghezza {
            lo = m;
        } else {
            hi = m - 1;
        }
    }
    let testo: String = caratteri[..lo]
        .iter()
        .collect::<String>()
        .trim_end()
        .to_string()
        + "…";
    p.layout_no_wrap(testo, font, colore)
}

/// Testo alla misura più grande (fino a `massima`) che entra in `larghezza`.
fn adatta(
    p: &Painter,
    testo: &str,
    mono: bool,
    massima: f32,
    larghezza: f32,
    colore: Color32,
) -> Arc<Galley> {
    let font = |m: f32| if mono { font_numeri(m) } else { font_testo(m) };
    let g = p.layout_no_wrap(testo.to_string(), font(massima), colore);
    if g.size().x <= larghezza || g.size().x <= 0.0 {
        return g;
    }
    let misura = (massima * larghezza / g.size().x).floor().max(9.0);
    p.layout_no_wrap(testo.to_string(), font(misura), colore)
}

fn disegna_galley(p: &Painter, g: &Arc<Galley>, pos: Pos2, ancora: Align2) -> Rect {
    let r = ancora.anchor_size(pos, g.size());
    p.galley(r.min, g.clone(), tema::TESTO);
    r
}

/// Punti di un arco (angoli in radianti, y verso il basso: senso orario).
fn punti_arco(c: Pos2, r: f32, da: f32, a: f32) -> Vec<Pos2> {
    let passi = (((a - da).abs() * r / 3.0).ceil() as usize).clamp(2, 256);
    (0..=passi)
        .map(|i| {
            let t = da + (a - da) * i as f32 / passi as f32;
            c + r * vec2(t.cos(), t.sin())
        })
        .collect()
}

fn arco(p: &Painter, c: Pos2, r: f32, da: f32, a: f32, stroke: Stroke) {
    if (a - da).abs() < 1e-4 {
        return;
    }
    p.add(Shape::line(punti_arco(c, r, da, a), stroke));
}

/// Rettangolo arrotondato con sfumatura verticale (rulli del contachilometri).
fn rullo(p: &Painter, r: Rect) {
    let mut mesh = Mesh::default();
    let fasce = [
        (0.0, tema::SFONDO),
        (0.28, tema::SUPERFICIE_ALTA),
        (0.72, tema::SUPERFICIE_ALTA),
        (1.0, tema::SFONDO),
    ];
    for (i, (t, c)) in fasce.iter().enumerate() {
        let y = r.top() + r.height() * t;
        mesh.colored_vertex(pos2(r.left(), y), *c);
        mesh.colored_vertex(pos2(r.right(), y), *c);
        if i > 0 {
            let b = (i as u32) * 2;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b, b + 1);
        }
    }
    p.add(Shape::mesh(mesh));
    p.rect_stroke(
        r,
        CornerRadius::same(3),
        Stroke::new(1.0, tema::BORDO_FORTE),
        StrokeKind::Inside,
    );
}

/// Riquadro dello strumento: superficie, bordo, margine interno.
fn scheda(p: &Painter, rect: Rect) -> Rect {
    p.rect(
        rect,
        CornerRadius::same(tema::RAGGIO_SCHEDA),
        tema::SUPERFICIE,
        tema::tratto_bordo(),
        StrokeKind::Inside,
    );
    let lato = rect.width().min(rect.height());
    let margine = if lato < 120.0 {
        tema::SP_2
    } else if lato < 260.0 {
        tema::SP_3
    } else {
        tema::SP_4
    };
    rect.shrink(margine)
}

/// Misura del titolo secondo la misura dello strumento.
fn misura_titolo(rect: Rect) -> f32 {
    if rect.width().min(rect.height()) < 120.0 {
        tema::CARATTERE_NOTA
    } else {
        tema::CARATTERE_CORPO
    }
}

/// Titolo in alto a sinistra; restituisce l'area sotto il titolo.
fn titolo(p: &Painter, area: Rect, v: &Vista<'_>, riserva_destra: f32) -> Rect {
    let m = misura_titolo(area);
    let g = elidi(
        p,
        v.titolo,
        font_testo(m),
        tema::TESTO_2,
        (area.width() - riserva_destra).max(20.0),
    );
    disegna_galley(p, &g, area.left_top(), Align2::LEFT_TOP);
    let alto = m * 1.35;
    Rect::from_min_max(pos2(area.left(), area.top() + alto), area.max)
}

/// Strumento attenuato con il motivo al centro (non supportato).
fn messaggio_centrale(p: &Painter, area: Rect, testo: &str, colore: Color32) {
    let m = if area.width() < 160.0 {
        tema::CARATTERE_NOTA
    } else {
        tema::CARATTERE_CORPO
    };
    let g = p.layout(
        testo.to_string(),
        font_testo(m),
        colore,
        (area.width() - 8.0).max(40.0),
    );
    disegna_galley(p, &g, area.center(), Align2::CENTER_CENTER);
}

// ----------------------------------------------------------------------
// Ingresso
// ----------------------------------------------------------------------

/// Disegna lo strumento nel rettangolo dato.
pub fn disegna(ui: &egui::Ui, rect: Rect, v: &Vista<'_>) {
    let p = ui.painter_at(rect.expand(2.0));
    let interno = scheda(&p, rect);
    let non_supportato = v.valore == StatoValore::NonSupportato && v.spia.is_none();
    if non_supportato {
        let mut attenuato = p.clone();
        attenuato.multiply_opacity(0.32);
        corpo(ui.ctx(), &attenuato, interno, v, false);
        let sotto = titolo(&p, interno, v, 0.0);
        messaggio_centrale(
            &p,
            sotto,
            "Non supportato da questa centralina",
            tema::TESTO_2,
        );
    } else {
        corpo(ui.ctx(), &p, interno, v, true);
    }
}

fn corpo(ctx: &egui::Context, p: &Painter, interno: Rect, v: &Vista<'_>, con_valore: bool) {
    match v.s.tipo {
        TipoStrumento::Quadrante => quadrante(ctx, p, interno, v, con_valore),
        TipoStrumento::Arco => arco_compatto(ctx, p, interno, v, con_valore),
        TipoStrumento::Barra => barra(ctx, p, interno, v, con_valore),
        TipoStrumento::Numerico => numerico(p, interno, v, con_valore),
        TipoStrumento::Grafico => grafico(p, interno, v, con_valore),
        TipoStrumento::Contachilometri => contachilometri(p, interno, v, con_valore),
        TipoStrumento::Spia => spia(p, interno, v),
    }
}

// ----------------------------------------------------------------------
// Quadrante (270°)
// ----------------------------------------------------------------------

const INIZIO_QUADRANTE: f32 = 0.75 * PI; // 135°
const AMPIEZZA_QUADRANTE: f32 = 1.5 * PI; // 270°

fn quadrante(ctx: &egui::Context, p: &Painter, interno: Rect, v: &Vista<'_>, con_valore: bool) {
    let area = titolo(p, interno, v, 0.0);
    let s = v.s;
    // Il quadrante occupa 1 + sin 45° ≈ 1,71 raggi in altezza.
    let r = (area.width() / 2.0).min(area.height() / 1.74).max(10.0);
    let c = pos2(
        area.center().x,
        area.top() + r + (area.height() - 1.74 * r).max(0.0) / 2.0,
    );
    let angolo = |f: f32| INIZIO_QUADRANTE + AMPIEZZA_QUADRANTE * f;

    let spessore_zona = (r * 0.035).clamp(2.0, 5.0);
    let spessore = (r * 0.075).clamp(4.0, 14.0);
    let distacco = (r * 0.03).max(2.0);
    let r_zona = r - spessore_zona / 2.0;
    let r_traccia = r - spessore_zona - distacco - spessore / 2.0;
    let fine = INIZIO_QUADRANTE + AMPIEZZA_QUADRANTE;

    // Zone di attenzione e critiche (arco esterno sottile).
    for (soglia, colore) in [
        (s.soglia_attenzione, tema::ATTENZIONE),
        (s.soglia_critica, tema::PERICOLO),
    ] {
        if let Some(sg) = soglia {
            for (a, b) in sg.intervalli(s.min, s.max) {
                arco(
                    p,
                    c,
                    r_zona,
                    angolo(frazione(a, s.min, s.max)),
                    angolo(frazione(b, s.min, s.max)),
                    Stroke::new(spessore_zona, colore),
                );
            }
        }
    }
    // Arco di fondo.
    arco(
        p,
        c,
        r_traccia,
        INIZIO_QUADRANTE,
        fine,
        Stroke::new(spessore, tema::BORDO_FORTE),
    );

    // Tacche e numeri.
    let lato = r * 2.0;
    let max_tacche = if lato < 220.0 { 6 } else { 10 };
    let (passo, sotto) = passo_tacche(s.min, s.max, max_tacche);
    let migliaia = s.max.abs().max(s.min.abs()) >= 1000.0;
    let divisore = if migliaia { 1000.0 } else { 1.0 };
    let decimali = decimali_passo(passo / divisore);
    let misura_numeri = (r * 0.12).clamp(10.0, 18.0);
    let r_tacca = r_traccia - spessore / 2.0 - distacco;
    let lunga = (r * 0.08).max(5.0);
    let corta = (r * 0.04).max(3.0);
    let primo = (s.min / passo - 1e-9).ceil() as i64;
    let ultimo = (s.max / passo + 1e-9).floor() as i64;
    let passo_minore = passo / sotto as f64;
    let primo_minore = (s.min / passo_minore - 1e-9).ceil() as i64;
    let ultimo_minore = (s.max / passo_minore + 1e-9).floor() as i64;
    for i in primo_minore..=ultimo_minore {
        if i % sotto as i64 == 0 {
            continue;
        }
        let a = angolo(frazione(i as f64 * passo_minore, s.min, s.max));
        let d = vec2(a.cos(), a.sin());
        p.line_segment(
            [c + d * r_tacca, c + d * (r_tacca - corta)],
            Stroke::new(1.0, tema::BORDO_FORTE),
        );
    }
    for i in primo..=ultimo {
        let valore = i as f64 * passo;
        let a = angolo(frazione(valore, s.min, s.max));
        let d = vec2(a.cos(), a.sin());
        p.line_segment(
            [c + d * r_tacca, c + d * (r_tacca - lunga)],
            Stroke::new(1.5, tema::TESTO_3),
        );
        let testo = num_it(valore / divisore, decimali);
        let g = p.layout_no_wrap(testo, font_numeri(misura_numeri), tema::TESTO_2);
        // Distanza dal bordo che tiene conto dell'ingombro del numero.
        let mezzo = g.size() / 2.0;
        let ingombro = (mezzo.x * d.x.abs() + mezzo.y * d.y.abs()) + 2.0;
        let pos = c + d * (r_tacca - lunga - ingombro);
        disegna_galley(p, &g, pos, Align2::CENTER_CENTER);
    }
    if migliaia {
        p.text(
            c - vec2(0.0, r * 0.36),
            Align2::CENTER_CENTER,
            "×1000",
            font_testo((r * 0.085).clamp(10.0, 14.0)),
            tema::TESTO_3,
        );
    }

    // Valore: arco, picco, lancetta.
    let f = if con_valore {
        v.frazione_smorzata(ctx)
    } else {
        None
    };
    let f0 = frazione(origine(s.min, s.max), s.min, s.max);
    if let Some(f) = f {
        let (a, b) = if f >= f0 { (f0, f) } else { (f, f0) };
        arco(
            p,
            c,
            r_traccia,
            angolo(a),
            angolo(b),
            Stroke::new(spessore, v.colore_valore()),
        );
    }
    if s.mostra_picco
        && con_valore
        && let Some(pk) = v.picco
    {
        let a = angolo(frazione(pk, s.min, s.max));
        let d = vec2(a.cos(), a.sin());
        let t = vec2(-d.y, d.x);
        let base = r_traccia + spessore / 2.0 + spessore_zona + distacco;
        let lato = (r * 0.055).clamp(4.0, 9.0);
        p.add(Shape::convex_polygon(
            vec![
                c + d * (r_traccia + spessore * 0.15),
                c + d * base + t * lato * 0.6,
                c + d * base - t * lato * 0.6,
            ],
            tema::TESTO,
            Stroke::NONE,
        ));
    }
    if let Some(f) = f {
        let a = angolo(f);
        let d = vec2(a.cos(), a.sin());
        let colore = if v.vecchio() {
            tema::TESTO_3
        } else {
            tema::TESTO
        };
        let largo = (r * 0.022).clamp(1.5, 4.0);
        p.line_segment(
            [c - d * (r * 0.12), c + d * (r_traccia + spessore / 2.0)],
            Stroke::new(largo, colore),
        );
    }
    let perno = (r * 0.055).clamp(4.0, 10.0);
    p.circle(
        c,
        perno,
        tema::SUPERFICIE_ALTA,
        Stroke::new(
            1.5,
            if f.is_some() {
                tema::TESTO_2
            } else {
                tema::BORDO_FORTE
            },
        ),
    );

    if !con_valore {
        return;
    }
    // Numero grande nella metà inferiore, con l'unità sotto.
    // Largo al massimo 0,9 raggi: così non tocca i numeri agli estremi della
    // scala, che stanno a circa ±0,53 raggi in basso.
    let misura_valore = (r * 0.27).clamp(16.0, tema::VALORE_MAX);
    let g = adatta(
        p,
        &v.testo_numero(),
        true,
        misura_valore,
        r * 0.9,
        v.colore_numero(),
    );
    let centro_valore = c + vec2(0.0, perno + g.size().y * 0.6);
    let rv = Align2::CENTER_CENTER.anchor_size(centro_valore, g.size());
    p.rect_filled(rv.expand(2.0), CornerRadius::same(4), tema::SUPERFICIE);
    p.galley(rv.min, g, tema::TESTO);
    let mut y = rv.bottom() + 1.0;
    if !v.unita.is_empty() {
        let gu = elidi(
            p,
            v.unita,
            font_testo((r * 0.10).clamp(11.0, 18.0)),
            tema::TESTO_2,
            r * 0.8,
        );
        let ru = disegna_galley(p, &gu, pos2(c.x, y), Align2::CENTER_TOP);
        y = ru.bottom() + 1.0;
    }
    if let Some((nota, colore)) = nota_stato(&v.valore) {
        // Sotto le etichette degli estremi della scala la nota ha spazio.
        let y = y.max(c.y + r * 0.64);
        let gn = elidi(p, &nota, font_testo(tema::CARATTERE_NOTA), colore, r * 1.5);
        disegna_galley(p, &gn, pos2(c.x, y), Align2::CENTER_TOP);
    }
}

// ----------------------------------------------------------------------
// Arco (180°)
// ----------------------------------------------------------------------

fn arco_compatto(ctx: &egui::Context, p: &Painter, interno: Rect, v: &Vista<'_>, con_valore: bool) {
    let area = titolo(p, interno, v, 0.0);
    let s = v.s;
    let misura_etichette = (area.height() * 0.09).clamp(10.0, 14.0);
    let alto_etichette = misura_etichette * 1.3;
    let r = (area.width() / 2.0 - 2.0)
        .min(area.height() - alto_etichette - 2.0)
        .max(10.0);
    let c = pos2(area.center().x, area.top() + r + 2.0);
    let angolo = |f: f32| PI + PI * f;
    let spessore_zona = (r * 0.04).clamp(2.0, 5.0);
    let spessore = (r * 0.11).clamp(5.0, 16.0);
    let distacco = (r * 0.03).max(2.0);
    let r_zona = r - spessore_zona / 2.0;
    let r_traccia = r - spessore_zona - distacco - spessore / 2.0;

    for (soglia, colore) in [
        (s.soglia_attenzione, tema::ATTENZIONE),
        (s.soglia_critica, tema::PERICOLO),
    ] {
        if let Some(sg) = soglia {
            for (a, b) in sg.intervalli(s.min, s.max) {
                arco(
                    p,
                    c,
                    r_zona,
                    angolo(frazione(a, s.min, s.max)),
                    angolo(frazione(b, s.min, s.max)),
                    Stroke::new(spessore_zona, colore),
                );
            }
        }
    }
    arco(
        p,
        c,
        r_traccia,
        PI,
        2.0 * PI,
        Stroke::new(spessore, tema::BORDO_FORTE),
    );
    // Tacche principali senza numero, all'interno.
    let (passo, _) = passo_tacche(s.min, s.max, 6);
    let primo = (s.min / passo - 1e-9).ceil() as i64;
    let ultimo = (s.max / passo + 1e-9).floor() as i64;
    let r_tacca = r_traccia - spessore / 2.0 - distacco;
    for i in primo..=ultimo {
        let f = frazione(i as f64 * passo, s.min, s.max);
        // Agli estremi la tacca sembrerebbe un segno meno accanto all'etichetta.
        if f <= 0.001 || f >= 0.999 {
            continue;
        }
        let a = angolo(f);
        let d = vec2(a.cos(), a.sin());
        p.line_segment(
            [c + d * r_tacca, c + d * (r_tacca - (r * 0.07).max(4.0))],
            Stroke::new(1.0, tema::TESTO_3),
        );
    }
    // Estremi della scala sotto le punte.
    let fe = font_numeri(misura_etichette);
    p.text(
        pos2(c.x - r_traccia, c.y + 2.0),
        Align2::CENTER_TOP,
        v.estremo(s.min),
        fe.clone(),
        tema::TESTO_3,
    );
    p.text(
        pos2(c.x + r_traccia, c.y + 2.0),
        Align2::CENTER_TOP,
        v.estremo(s.max),
        fe,
        tema::TESTO_3,
    );

    let f = if con_valore {
        v.frazione_smorzata(ctx)
    } else {
        None
    };
    if let Some(f) = f {
        let f0 = frazione(origine(s.min, s.max), s.min, s.max);
        let (a, b) = if f >= f0 { (f0, f) } else { (f, f0) };
        arco(
            p,
            c,
            r_traccia,
            angolo(a),
            angolo(b),
            Stroke::new(spessore, v.colore_valore()),
        );
        // Punta del valore: breve segno chiaro sul bordo dell'arco.
        let ang = angolo(f);
        let d = vec2(ang.cos(), ang.sin());
        p.line_segment(
            [
                c + d * (r_traccia - spessore / 2.0 - 1.0),
                c + d * (r_traccia + spessore / 2.0 + 1.0),
            ],
            Stroke::new(
                2.5,
                if v.vecchio() {
                    tema::TESTO_3
                } else {
                    tema::TESTO
                },
            ),
        );
    }
    if s.mostra_picco
        && con_valore
        && let Some(pk) = v.picco
    {
        let a = angolo(frazione(pk, s.min, s.max));
        let d = vec2(a.cos(), a.sin());
        let t = vec2(-d.y, d.x);
        let base = r_traccia + spessore / 2.0 + spessore_zona + distacco;
        let lato = (r * 0.06).clamp(4.0, 8.0);
        p.add(Shape::convex_polygon(
            vec![
                c + d * (r_traccia + spessore * 0.1),
                c + d * base + t * lato * 0.6,
                c + d * base - t * lato * 0.6,
            ],
            tema::TESTO,
            Stroke::NONE,
        ));
    }
    if !con_valore {
        return;
    }
    // Valore e unità dentro il semicerchio.
    let interno_r = r_tacca - (r * 0.07).max(4.0);
    let misura = (r * 0.36).clamp(16.0, 52.0);
    let gu = (!v.unita.is_empty()).then(|| {
        p.layout_no_wrap(
            v.unita.to_string(),
            font_testo((misura * 0.4).clamp(11.0, 18.0)),
            tema::TESTO_2,
        )
    });
    let largo_unita = gu.as_ref().map_or(0.0, |g| g.size().x + 4.0);
    let g = adatta(
        p,
        &v.testo_numero(),
        true,
        misura,
        (interno_r * 1.6 - largo_unita).max(30.0),
        v.colore_numero(),
    );
    let totale = g.size().x + largo_unita;
    let base = c.y - 2.0;
    let rv = Rect::from_min_size(pos2(c.x - totale / 2.0, base - g.size().y), g.size());
    p.galley(rv.min, g, tema::TESTO);
    if let Some(gu) = gu {
        let pos = pos2(
            rv.right() + 4.0,
            rv.bottom() - gu.size().y - rv.height() * 0.1,
        );
        p.galley(pos, gu, tema::TESTO_2);
    }
    if let Some((nota, colore)) = nota_stato(&v.valore) {
        let gn = elidi(
            p,
            &nota,
            font_testo(tema::CARATTERE_NOTA),
            colore,
            interno_r * 1.5,
        );
        disegna_galley(p, &gn, pos2(c.x, rv.top() - 2.0), Align2::CENTER_BOTTOM);
    }
}

// ----------------------------------------------------------------------
// Barra
// ----------------------------------------------------------------------

fn barra(ctx: &egui::Context, p: &Painter, interno: Rect, v: &Vista<'_>, con_valore: bool) {
    if interno.width() >= interno.height() * 0.8 {
        barra_orizzontale(ctx, p, interno, v, con_valore);
    } else {
        barra_verticale(ctx, p, interno, v, con_valore);
    }
}

fn triangolo(p: &Painter, punta: Pos2, verso: Vec2, lato: f32, colore: Color32) {
    let t = vec2(-verso.y, verso.x);
    p.add(Shape::convex_polygon(
        vec![
            punta,
            punta - verso * lato + t * lato * 0.6,
            punta - verso * lato - t * lato * 0.6,
        ],
        colore,
        Stroke::NONE,
    ));
}

fn barra_orizzontale(
    ctx: &egui::Context,
    p: &Painter,
    interno: Rect,
    v: &Vista<'_>,
    con_valore: bool,
) {
    let s = v.s;
    let alto = interno.height();
    let alta = alto >= 130.0;
    if alta {
        // Riquadro alto: numero grande sotto il titolo, a sinistra.
        let sotto = titolo(p, interno, v, 0.0);
        if con_valore {
            let misura = (alto * 0.30).clamp(tema::VALORE_MIN, tema::VALORE_MAX);
            let gu = (!v.unita.is_empty()).then(|| {
                p.layout_no_wrap(
                    v.unita.to_string(),
                    font_testo((misura * 0.38).clamp(12.0, 20.0)),
                    tema::TESTO_2,
                )
            });
            let largo_unita = gu.as_ref().map_or(0.0, |g| g.size().x + tema::SP_2);
            let g = adatta(
                p,
                &v.testo_numero(),
                true,
                misura,
                (interno.width() - largo_unita).max(30.0),
                v.colore_numero(),
            );
            let rv = Rect::from_min_size(sotto.left_top() + vec2(0.0, tema::SP_1), g.size());
            p.galley(rv.min, g, tema::TESTO);
            if let Some(gu) = gu {
                let pos = pos2(
                    rv.right() + tema::SP_2,
                    rv.bottom() - gu.size().y - rv.height() * 0.12,
                );
                p.galley(pos, gu, tema::TESTO_2);
            }
        }
    } else {
        // Riquadro basso: numero e unità a destra del titolo.
        let misura_valore = (alto * 0.30).clamp(16.0, 48.0);
        let mut riserva = 0.0;
        if con_valore {
            let testo = if v.unita.is_empty() {
                v.testo_numero()
            } else {
                format!("{} {}", v.testo_numero(), v.unita)
            };
            let g = adatta(
                p,
                &testo,
                true,
                misura_valore,
                interno.width() * 0.62,
                v.colore_numero(),
            );
            riserva = g.size().x + tema::SP_2;
            disegna_galley(p, &g, interno.right_top(), Align2::RIGHT_TOP);
        }
        let _ = titolo(p, interno, v, riserva);
    }

    let misura_etichette = (alto * 0.11).clamp(10.0, 13.0);
    let alto_etichette = misura_etichette * 1.25;
    let spessore = if alta {
        (alto * 0.13).clamp(12.0, 22.0)
    } else {
        (alto * 0.16).clamp(8.0, 18.0)
    };
    let traccia = Rect::from_min_max(
        pos2(interno.left(), interno.bottom() - alto_etichette - spessore),
        pos2(interno.right(), interno.bottom() - alto_etichette),
    );
    let x_di = |val: f64| traccia.left() + traccia.width() * frazione(val, s.min, s.max);
    let raggio = CornerRadius::same((spessore / 2.0).min(6.0) as u8);
    p.rect_filled(traccia, raggio, tema::BORDO_FORTE);
    // Zone: striscia sottile sopra la barra.
    let striscia = (spessore * 0.3).clamp(2.0, 4.0);
    for (soglia, colore) in [
        (s.soglia_attenzione, tema::ATTENZIONE),
        (s.soglia_critica, tema::PERICOLO),
    ] {
        if let Some(sg) = soglia {
            for (a, b) in sg.intervalli(s.min, s.max) {
                p.rect_filled(
                    Rect::from_min_max(
                        pos2(x_di(a), traccia.top() - striscia - 3.0),
                        pos2(x_di(b), traccia.top() - 3.0),
                    ),
                    CornerRadius::same(1),
                    colore,
                );
            }
        }
    }
    let zero = origine(s.min, s.max);
    if zero > s.min {
        let x = x_di(zero);
        p.line_segment(
            [
                pos2(x, traccia.top() - 2.0),
                pos2(x, traccia.bottom() + 2.0),
            ],
            Stroke::new(1.5, tema::TESTO_3),
        );
    }
    if con_valore && let Some(f) = v.frazione_smorzata(ctx) {
        let x0 = x_di(zero);
        let x1 = traccia.left() + traccia.width() * f;
        let r = Rect::from_x_y_ranges(x0.min(x1)..=x0.max(x1), traccia.y_range());
        if r.width() > 0.5 {
            p.rect_filled(r, raggio, v.colore_valore());
        }
    }
    if s.mostra_picco
        && con_valore
        && let Some(pk) = v.picco
    {
        triangolo(
            p,
            pos2(x_di(pk), traccia.top() - 1.0),
            vec2(0.0, 1.0),
            (spessore * 0.5).clamp(4.0, 8.0),
            tema::TESTO,
        );
    }
    // Etichette: minimo, zero centrale, massimo.
    let fe = font_numeri(misura_etichette);
    let y = traccia.bottom() + 2.0;
    p.text(
        pos2(traccia.left(), y),
        Align2::LEFT_TOP,
        v.estremo(s.min),
        fe.clone(),
        tema::TESTO_3,
    );
    p.text(
        pos2(traccia.right(), y),
        Align2::RIGHT_TOP,
        v.estremo(s.max),
        fe.clone(),
        tema::TESTO_3,
    );
    if zero > s.min {
        p.text(
            pos2(x_di(zero), y),
            Align2::CENTER_TOP,
            "0",
            fe,
            tema::TESTO_3,
        );
    }
    if con_valore && let Some((nota, colore)) = nota_stato(&v.valore) {
        let alto_titolo = misura_titolo(interno) * 1.35;
        let spazio = traccia.top() - striscia - 4.0 - (interno.top() + alto_titolo);
        if spazio >= tema::CARATTERE_NOTA {
            let gn = elidi(
                p,
                &nota,
                font_testo(tema::CARATTERE_NOTA),
                colore,
                interno.width(),
            );
            let (x, ancora) = if alta {
                (interno.right(), Align2::RIGHT_BOTTOM)
            } else {
                (interno.left(), Align2::LEFT_BOTTOM)
            };
            disegna_galley(p, &gn, pos2(x, traccia.top() - striscia - 4.0), ancora);
        }
    }
}

fn barra_verticale(
    ctx: &egui::Context,
    p: &Painter,
    interno: Rect,
    v: &Vista<'_>,
    con_valore: bool,
) {
    let s = v.s;
    let mut area = titolo(p, interno, v, 0.0);
    if con_valore {
        let misura = (interno.width() * 0.26).clamp(14.0, 40.0);
        let g = adatta(
            p,
            &v.testo_numero(),
            true,
            misura,
            interno.width(),
            v.colore_numero(),
        );
        let r = disegna_galley(
            p,
            &g,
            pos2(interno.center().x, area.top()),
            Align2::CENTER_TOP,
        );
        area.min.y = r.bottom() + 2.0;
        if !v.unita.is_empty() {
            let gu = elidi(
                p,
                v.unita,
                font_testo(tema::CARATTERE_NOTA),
                tema::TESTO_2,
                interno.width(),
            );
            let r = disegna_galley(
                p,
                &gu,
                pos2(interno.center().x, area.top()),
                Align2::CENTER_TOP,
            );
            area.min.y = r.bottom() + tema::SP_1;
        }
        if let Some((nota, colore)) = nota_stato(&v.valore) {
            let gn = elidi(
                p,
                &nota,
                font_testo(tema::CARATTERE_NOTA),
                colore,
                interno.width(),
            );
            let r = disegna_galley(
                p,
                &gn,
                pos2(interno.center().x, area.top()),
                Align2::CENTER_TOP,
            );
            area.min.y = r.bottom() + tema::SP_1;
        }
    }
    let misura_etichette = 11.0;
    let spessore = (interno.width() * 0.2).clamp(10.0, 22.0);
    let margine = misura_etichette * 0.7;
    // Barra ed etichette (a destra) centrate insieme nella larghezza.
    let fe = font_numeri(misura_etichette);
    let largo_etichette = [s.min, s.max]
        .iter()
        .map(|x| {
            p.layout_no_wrap(v.estremo(*x), fe.clone(), tema::TESTO_3)
                .size()
                .x
        })
        .fold(0.0f32, f32::max);
    let gruppo = 7.0 + spessore + tema::SP_1 + largo_etichette;
    let sinistra = (area.center().x - gruppo / 2.0 + 7.0).max(area.left() + 7.0);
    let traccia = Rect::from_min_max(
        pos2(sinistra, area.top() + margine),
        pos2(sinistra + spessore, area.bottom() - margine),
    );
    if traccia.height() < 12.0 {
        return;
    }
    let y_di = |val: f64| traccia.bottom() - traccia.height() * frazione(val, s.min, s.max);
    let raggio = CornerRadius::same((spessore / 2.0).min(6.0) as u8);
    p.rect_filled(traccia, raggio, tema::BORDO_FORTE);
    for (soglia, colore) in [
        (s.soglia_attenzione, tema::ATTENZIONE),
        (s.soglia_critica, tema::PERICOLO),
    ] {
        if let Some(sg) = soglia {
            for (a, b) in sg.intervalli(s.min, s.max) {
                p.rect_filled(
                    Rect::from_min_max(
                        pos2(traccia.left() - 7.0, y_di(b)),
                        pos2(traccia.left() - 4.0, y_di(a)),
                    ),
                    CornerRadius::same(1),
                    colore,
                );
            }
        }
    }
    let zero = origine(s.min, s.max);
    if con_valore && let Some(f) = v.frazione_smorzata(ctx) {
        let y0 = y_di(zero);
        let y1 = traccia.bottom() - traccia.height() * f;
        let r = Rect::from_x_y_ranges(traccia.x_range(), y0.min(y1)..=y0.max(y1));
        if r.height() > 0.5 {
            p.rect_filled(r, raggio, v.colore_valore());
        }
    }
    if s.mostra_picco
        && con_valore
        && let Some(pk) = v.picco
    {
        triangolo(
            p,
            pos2(traccia.right() + 1.0, y_di(pk)),
            vec2(-1.0, 0.0),
            6.0,
            tema::TESTO,
        );
    }
    let x = traccia.right() + tema::SP_1;
    p.text(
        pos2(x, traccia.top()),
        Align2::LEFT_CENTER,
        v.estremo(s.max),
        fe.clone(),
        tema::TESTO_3,
    );
    p.text(
        pos2(x, traccia.bottom()),
        Align2::LEFT_CENTER,
        v.estremo(s.min),
        fe.clone(),
        tema::TESTO_3,
    );
    if zero > s.min {
        let y = y_di(zero);
        p.line_segment(
            [
                pos2(traccia.left() - 2.0, y),
                pos2(traccia.right() + 2.0, y),
            ],
            Stroke::new(1.5, tema::TESTO_3),
        );
        p.text(pos2(x, y), Align2::LEFT_CENTER, "0", fe, tema::TESTO_3);
    }
}

// ----------------------------------------------------------------------
// Numerico
// ----------------------------------------------------------------------

fn numerico(p: &Painter, interno: Rect, v: &Vista<'_>, con_valore: bool) {
    let area = titolo(p, interno, v, 0.0);
    if !con_valore {
        return;
    }
    let piede = if area.height() > 90.0 {
        tema::CARATTERE_NOTA * 1.4
    } else {
        0.0
    };
    let alto = area.height() - piede;
    // Fra 28 e 64 px quando c'è spazio; più piccolo solo se non entra.
    let misura = (alto * 0.62)
        .max(tema::VALORE_MIN.min(alto * 0.8))
        .min(tema::VALORE_MAX);
    let gu = (!v.unita.is_empty()).then(|| {
        p.layout_no_wrap(
            v.unita.to_string(),
            font_testo((misura * 0.36).clamp(12.0, 22.0)),
            tema::TESTO_2,
        )
    });
    let largo_unita = gu.as_ref().map_or(0.0, |g| g.size().x + tema::SP_2);
    let g = adatta(
        p,
        &v.testo_numero(),
        true,
        misura,
        (area.width() - largo_unita).max(30.0),
        v.colore_numero(),
    );
    let totale = g.size().x + largo_unita;
    let centro = pos2(area.center().x, area.top() + alto / 2.0);
    let rv = Rect::from_min_size(
        pos2(centro.x - totale / 2.0, centro.y - g.size().y / 2.0),
        g.size(),
    );
    p.galley(rv.min, g, tema::TESTO);
    if let Some(gu) = gu {
        let pos = pos2(
            rv.right() + tema::SP_2,
            rv.bottom() - gu.size().y - rv.height() * 0.12,
        );
        p.galley(pos, gu, tema::TESTO_2);
    }
    let nota = nota_stato(&v.valore);
    if piede > 0.0 {
        let y = interno.bottom();
        let piccolo = font_testo(tema::CARATTERE_NOTA);
        let mut destra = interno.right();
        if let Some((n, colore)) = &nota {
            let gn = elidi(p, n, piccolo.clone(), *colore, interno.width() * 0.55);
            let r = disegna_galley(p, &gn, pos2(destra, y), Align2::RIGHT_BOTTOM);
            destra = r.left() - tema::SP_2;
        }
        if let (Some(a), Some(b)) = (v.min_sessione, v.max_sessione) {
            let testo = format!("min {} · max {}", v.numero(a), v.numero(b));
            let g = elidi(
                p,
                &testo,
                font_numeri(tema::CARATTERE_NOTA - 1.0),
                tema::TESTO_3,
                destra - interno.left(),
            );
            disegna_galley(p, &g, pos2(interno.left(), y), Align2::LEFT_BOTTOM);
        }
    } else if let Some((n, colore)) = nota {
        let gn = elidi(
            p,
            &n,
            font_testo(tema::CARATTERE_NOTA),
            colore,
            interno.width() * 0.5,
        );
        disegna_galley(p, &gn, interno.right_top(), Align2::RIGHT_TOP);
    }
}

// ----------------------------------------------------------------------
// Grafico
// ----------------------------------------------------------------------

fn grafico(p: &Painter, interno: Rect, v: &Vista<'_>, con_valore: bool) {
    let s = v.s;
    let mut riserva = 0.0;
    let misura_valore = (interno.height() * 0.16).clamp(16.0, 28.0);
    if con_valore {
        let testo = if v.unita.is_empty() || v.valore.valore().is_none() {
            v.testo_numero()
        } else {
            format!("{} {}", v.testo_numero(), v.unita)
        };
        let g = adatta(
            p,
            &testo,
            true,
            misura_valore,
            interno.width() * 0.5,
            v.colore_numero(),
        );
        riserva = g.size().x + tema::SP_3;
        disegna_galley(p, &g, interno.right_top(), Align2::RIGHT_TOP);
    }
    let alto_testa = (misura_titolo(interno) * 1.35).max(misura_valore * 1.25);
    let _ = titolo(p, interno, v, riserva);
    let piede = tema::CARATTERE_NOTA * 1.3;
    let area = Rect::from_min_max(
        pos2(interno.left(), interno.top() + alto_testa + tema::SP_1),
        pos2(interno.right(), interno.bottom() - piede),
    );
    if area.height() < 16.0 {
        return;
    }
    p.rect_filled(area, CornerRadius::same(4), tema::SFONDO);

    let fine = v.archivio.t_max;
    let inizio = fine - s.secondi.max(1.0);
    let tracciato = if con_valore && !v.chiave.is_empty() {
        v.archivio.tracciato(v.chiave, inizio)
    } else {
        Default::default()
    };
    // Scala verticale: i valori visibili, con un margine.
    let visibili = tracciato
        .tratti
        .iter()
        .flatten()
        .filter(|q| q[0] >= inizio - 1.0)
        .map(|q| q[1]);
    let (mut lo, mut hi) = visibili.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), y| {
        (a.min(y), b.max(y))
    });
    let dati = lo.is_finite();
    if !dati {
        lo = s.min;
        hi = s.max;
    }
    let ampiezza = (hi - lo).max((s.max - s.min).abs() * 0.02).max(1e-6);
    let (y_lo, y_hi) = (lo - ampiezza * 0.12, hi + ampiezza * 0.12);
    let interno_area = area.shrink2(vec2(2.0, 4.0));
    let x_di = |t: f64| {
        interno_area.left()
            + interno_area.width() * ((t - inizio) / (fine - inizio)).clamp(0.0, 1.0) as f32
    };
    let y_di = |y: f64| {
        interno_area.bottom()
            - interno_area.height() * ((y - y_lo) / (y_hi - y_lo)).clamp(0.0, 1.0) as f32
    };
    let ritaglio = p.with_clip_rect(area);

    // Intervalli senza dato valido.
    for (a, b) in &tracciato.buchi {
        let (xa, xb) = (x_di(*a), x_di(*b));
        if xb - xa > 0.5 {
            ritaglio.rect_filled(
                Rect::from_x_y_ranges(xa..=xb, area.y_range()),
                CornerRadius::ZERO,
                tema::velatura(tema::PERICOLO, 0.14),
            );
        }
    }
    // Soglie come linee tratteggiate.
    for (soglia, colore) in [
        (s.soglia_attenzione, tema::ATTENZIONE),
        (s.soglia_critica, tema::PERICOLO),
    ] {
        if let Some(sg) = soglia {
            let valori = match sg.verso {
                super::modello::Verso::Simmetrica => vec![sg.valore.abs(), -sg.valore.abs()],
                _ => vec![sg.valore],
            };
            for val in valori {
                if dati && val > y_lo && val < y_hi {
                    let y = y_di(val);
                    ritaglio.extend(Shape::dashed_line(
                        &[pos2(area.left(), y), pos2(area.right(), y)],
                        Stroke::new(1.0, tema::velatura(colore, 0.7)),
                        4.0,
                        4.0,
                    ));
                }
            }
        }
    }
    let colore = v.colore_valore();
    for tratto in &tracciato.tratti {
        let punti: Vec<Pos2> = tratto
            .iter()
            .map(|q| pos2(x_di(q[0]), y_di(q[1])))
            .collect();
        if punti.len() >= 2 {
            // Velatura sotto la curva.
            let mut mesh = Mesh::default();
            let sopra = tema::velatura(colore, 0.22);
            let sotto = tema::velatura(colore, 0.0);
            for (i, q) in punti.iter().enumerate() {
                mesh.colored_vertex(*q, sopra);
                mesh.colored_vertex(pos2(q.x, area.bottom()), sotto);
                if i > 0 {
                    let b = (i as u32) * 2;
                    mesh.add_triangle(b - 2, b - 1, b);
                    mesh.add_triangle(b - 1, b, b + 1);
                }
            }
            ritaglio.add(Shape::mesh(mesh));
            ritaglio.add(Shape::line(punti, Stroke::new(2.0, colore)));
        } else if let Some(q) = punti.first() {
            ritaglio.circle_filled(*q, 2.0, colore);
        }
    }
    if matches!(v.valore, StatoValore::Valido(_))
        && let Some(q) = tracciato.tratti.last().and_then(|t| t.last())
    {
        ritaglio.circle_filled(pos2(x_di(q[0]), y_di(q[1])), 3.5, colore);
    }
    // Minimo e massimo visibili, durata della finestra.
    let fe = font_numeri(tema::CARATTERE_NOTA - 1.0);
    if dati {
        let g = p.layout_no_wrap(v.numero(hi), fe.clone(), tema::TESTO_3);
        let r = Align2::LEFT_TOP.anchor_size(area.left_top() + vec2(4.0, 2.0), g.size());
        p.rect_filled(
            r.expand(1.0),
            CornerRadius::same(2),
            tema::velatura(tema::SFONDO, 0.8),
        );
        p.galley(r.min, g, tema::TESTO_3);
        if v.numero(lo) != v.numero(hi) {
            let g = p.layout_no_wrap(v.numero(lo), fe.clone(), tema::TESTO_3);
            let r = Align2::LEFT_BOTTOM.anchor_size(area.left_bottom() + vec2(4.0, -2.0), g.size());
            p.rect_filled(
                r.expand(1.0),
                CornerRadius::same(2),
                tema::velatura(tema::SFONDO, 0.8),
            );
            p.galley(r.min, g, tema::TESTO_3);
        }
    }
    let piccolo = font_testo(tema::CARATTERE_NOTA);
    p.text(
        pos2(interno.left(), interno.bottom()),
        Align2::LEFT_BOTTOM,
        format!("−{} s", num_it(s.secondi, 0)),
        piccolo.clone(),
        tema::TESTO_3,
    );
    match nota_stato(&v.valore).filter(|_| con_valore) {
        Some((n, c)) => {
            let g = elidi(p, &n, piccolo, c, interno.width() * 0.7);
            disegna_galley(p, &g, interno.right_bottom(), Align2::RIGHT_BOTTOM);
        }
        None => {
            p.text(
                interno.right_bottom(),
                Align2::RIGHT_BOTTOM,
                "ora",
                piccolo,
                tema::TESTO_3,
            );
        }
    }
}

// ----------------------------------------------------------------------
// Contachilometri
// ----------------------------------------------------------------------

/// Scatto del rullo: fermo sulla cifra per i primi tre quarti del passo,
/// poi gira (con un avvio e un arresto morbidi) verso la cifra successiva.
/// Così le cifre restano leggibili anche quando il valore cambia in fretta.
fn scatto(frazione: f64) -> f64 {
    let t = ((frazione - 0.75) / 0.25).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Posizioni dei rulli (cifra più frazione di rotazione), dalla più
/// significativa. L'ultimo rullo scatta verso la cifra successiva; ogni rullo
/// gira insieme a quello alla sua destra quando questo passa da 9 a 0, come
/// in un contachilometri meccanico.
pub fn posizioni_rulli(valore: f64, interi: usize, decimali: usize) -> Vec<f64> {
    let n = interi + decimali;
    let scalato = (valore.max(0.0) * 10f64.powi(decimali as i32)) % 10f64.powi(n as i32);
    let mut pos = vec![0.0; n];
    let mut riporto = 0.0;
    for i in (0..n).rev() {
        let esponente = (n - 1 - i) as i32;
        let cifra = (scalato / 10f64.powi(esponente)).floor() % 10.0;
        let p = if i == n - 1 {
            cifra + scatto(scalato.fract())
        } else {
            cifra + riporto
        };
        riporto = (p - 9.0).max(0.0);
        pos[i] = p;
    }
    pos
}

fn contachilometri(p: &Painter, interno: Rect, v: &Vista<'_>, con_valore: bool) {
    // Intestazione: titolo e, se serve, l'etichetta «Parziale».
    let mut riserva = 0.0;
    let mut alto_testa = misura_titolo(interno) * 1.35;
    if v.parziale {
        let g = p.layout_no_wrap(
            "Parziale".into(),
            font_testo(tema::CARATTERE_NOTA),
            tema::TESTO,
        );
        let r = Align2::RIGHT_TOP.anchor_size(interno.right_top(), g.size() + vec2(12.0, 4.0));
        p.rect(
            r,
            CornerRadius::same(tema::RAGGIO_CONTROLLO),
            tema::SUPERFICIE_ALTA,
            Stroke::new(1.0, tema::BORDO_FORTE),
            StrokeKind::Inside,
        );
        disegna_galley(p, &g, r.center(), Align2::CENTER_CENTER);
        riserva = r.width() + tema::SP_2;
        alto_testa = alto_testa.max(r.height() + 2.0);
    }
    let _ = titolo(p, interno, v, riserva);
    let area = Rect::from_min_max(
        pos2(interno.left(), interno.top() + alto_testa + tema::SP_1),
        interno.max,
    );
    let (interi, decimali) = if v.parziale { (4, 2) } else { (6, 1) };
    let n = interi + decimali;
    let nota = nota_stato(&v.valore).filter(|_| con_valore);
    let piede = if nota.is_some() {
        tema::CARATTERE_NOTA * 1.4
    } else {
        0.0
    };
    let alto_max = (area.height() - piede).max(10.0);
    let gu = p.layout_no_wrap(
        if v.unita.is_empty() { "km" } else { v.unita }.to_string(),
        font_testo((alto_max * 0.3).clamp(12.0, 20.0)),
        tema::TESTO_2,
    );
    let spazio = 3.0;
    let virgola = (alto_max * 0.12).clamp(6.0, 12.0);
    let disponibile = area.width() - gu.size().x - tema::SP_2 - virgola;
    let largo = ((disponibile - spazio * (n - 1) as f32) / n as f32)
        .min(alto_max * 0.64)
        .max(6.0);
    let alto = (largo * 1.55).min(alto_max);
    let totale = largo * n as f32 + spazio * (n - 1) as f32 + virgola + tema::SP_2 + gu.size().x;
    let x0 = area.center().x - totale / 2.0;
    let y0 = area.top() + (alto_max - alto) / 2.0;
    let posizioni = v
        .valore
        .valore()
        .filter(|_| con_valore)
        .map(|val| posizioni_rulli(val, interi, decimali));
    let misura = (alto * 0.62).min(largo * 1.3);
    let colore = if matches!(v.valore, StatoValore::Vecchio { .. }) {
        tema::TESTO_3
    } else {
        tema::TESTO
    };
    let mut x = x0;
    for i in 0..n {
        if i == interi {
            // Virgola decimale.
            p.text(
                pos2(x + virgola / 2.0 - spazio / 2.0, y0 + alto * 0.78),
                Align2::CENTER_CENTER,
                ",",
                font_numeri(misura * 0.8),
                tema::TESTO_2,
            );
            x += virgola;
        }
        let r = Rect::from_min_size(pos2(x, y0), vec2(largo, alto));
        rullo(p, r);
        let ritaglio = p.with_clip_rect(r.shrink(1.0));
        let colore_cifra = if i >= interi { tema::ACCENTO } else { colore };
        match &posizioni {
            Some(pos) => {
                let q = pos[i];
                let base = q.floor();
                let scorrimento = (q - base) as f32;
                for k in 0..2 {
                    let cifra = ((base as i64 + k) % 10).to_string();
                    let cy = r.center().y + (k as f32 - scorrimento) * alto * 0.92;
                    ritaglio.text(
                        pos2(r.center().x, cy),
                        Align2::CENTER_CENTER,
                        cifra,
                        font_numeri(misura),
                        colore_cifra,
                    );
                }
            }
            None => {
                ritaglio.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    "–",
                    font_numeri(misura * 0.8),
                    tema::TESTO_3,
                );
            }
        }
        x += largo + spazio;
    }
    let g_pos = pos2(
        x - spazio + tema::SP_2,
        y0 + alto - gu.size().y - alto * 0.1,
    );
    p.galley(g_pos, gu, tema::TESTO_2);
    if let Some((testo, c)) = nota {
        let g = elidi(
            p,
            &testo,
            font_testo(tema::CARATTERE_NOTA),
            c,
            interno.width(),
        );
        disegna_galley(
            p,
            &g,
            pos2(area.center().x, interno.bottom()),
            Align2::CENTER_BOTTOM,
        );
    }
}

// ----------------------------------------------------------------------
// Spia
// ----------------------------------------------------------------------

fn spia(p: &Painter, interno: Rect, v: &Vista<'_>) {
    let stato = v.spia.clone().unwrap_or(StatoSpia {
        accesa: None,
        colore: tema::TESTO_3,
        testo: "—".into(),
    });
    let r = (interno.height() * 0.3).clamp(7.0, 20.0);
    let c = pos2(interno.left() + r * 1.5, interno.center().y);
    match stato.accesa {
        Some(true) => {
            // Alone entro la scheda (il margine interno è almeno 8 px).
            for (k, alfa) in [(1.75, 0.10), (1.45, 0.18), (1.2, 0.28)] {
                p.circle_filled(c, r * k, tema::velatura(stato.colore, alfa));
            }
            p.circle_filled(c, r, stato.colore);
            p.circle_stroke(
                c,
                r * 0.55,
                Stroke::new(1.5, tema::velatura(tema::TESTO, 0.5)),
            );
        }
        Some(false) => {
            p.circle(
                c,
                r,
                tema::SUPERFICIE_ALTA,
                Stroke::new(1.5, tema::BORDO_FORTE),
            );
        }
        None => {
            p.circle_stroke(c, r, Stroke::new(1.5, tema::TESTO_3));
        }
    }
    let x = c.x + r * 1.75 + tema::SP_2;
    let largo = (interno.right() - x).max(20.0);
    let alto = interno.height();
    let misura_titolo = if alto < 70.0 {
        tema::CARATTERE_NOTA
    } else {
        tema::CARATTERE_CORPO
    };
    let misura_stato = (alto * 0.26).clamp(14.0, 22.0);
    let gt = elidi(p, v.titolo, font_testo(misura_titolo), tema::TESTO_2, largo);
    let colore_testo = match stato.accesa {
        Some(true) => stato.colore,
        Some(false) => tema::TESTO,
        None => tema::TESTO_3,
    };
    let gs = elidi(
        p,
        &stato.testo,
        font_testo(misura_stato),
        colore_testo,
        largo,
    );
    let totale = gt.size().y + gs.size().y;
    let y = interno.center().y - totale / 2.0;
    p.galley(pos2(x, y), gt.clone(), tema::TESTO_2);
    p.galley(pos2(x, y + gt.size().y), gs, colore_testo);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tacche(min: f64, max: f64, passo: f64) -> usize {
        ((max / passo + 1e-9).floor() - (min / passo - 1e-9).ceil() + 1.0) as usize
    }

    #[test]
    fn passi_tondi_con_5_10_tacche() {
        assert_eq!(passo_tacche(0.0, 8000.0, 10).0, 1000.0);
        assert_eq!(passo_tacche(0.0, 220.0, 10).0, 25.0);
        assert_eq!(passo_tacche(40.0, 130.0, 10).0, 10.0);
        assert_eq!(passo_tacche(-25.0, 25.0, 10).0, 10.0);
        assert_eq!(passo_tacche(0.0, 1.1, 10).0, 0.2);
        assert_eq!(passo_tacche(0.7, 1.3, 10).0, 0.1);
        assert_eq!(passo_tacche(10.0, 16.0, 10).0, 1.0);
        assert_eq!(passo_tacche(0.0, 100.0, 10).0, 20.0);
        assert_eq!(passo_tacche(0.0, 255.0, 10).0, 50.0);
        // Con meno spazio, meno tacche.
        assert_eq!(passo_tacche(0.0, 8000.0, 6).0, 2000.0);
        for (min, max) in [
            (0.0, 8000.0),
            (0.0, 220.0),
            (40.0, 130.0),
            (-25.0, 25.0),
            (0.0, 1.1),
            (0.7, 1.3),
            (0.0, 255.0),
            (-20.0, 60.0),
            (0.0, 3.0),
            (0.0, 1_000_000.0),
            (-5.0, 5.0),
        ] {
            let (passo, sotto) = passo_tacche(min, max, 10);
            let mantissa = passo / 10f64.powf(passo.log10().floor());
            assert!(
                [1.0, 2.0, 2.5, 5.0]
                    .iter()
                    .any(|m| (m - mantissa).abs() < 1e-9),
                "{min}–{max}: passo {passo}"
            );
            let n = tacche(min, max, passo);
            assert!((5..=10).contains(&n), "{min}–{max}: {n} tacche");
            assert!(sotto >= 1);
        }
        // Scale degeneri: nessun panico.
        assert_eq!(passo_tacche(5.0, 5.0, 10), (1.0, 1));
        assert_eq!(passo_tacche(0.0, f64::NAN, 10), (1.0, 1));
    }

    #[test]
    fn decimali_dei_passi_e_frazioni() {
        assert_eq!(decimali_passo(1000.0), 0);
        assert_eq!(decimali_passo(2.5), 1);
        assert_eq!(decimali_passo(0.25), 2);
        assert_eq!(decimali_passo(0.1), 1);
        assert_eq!(frazione(4000.0, 0.0, 8000.0), 0.5);
        assert_eq!(frazione(-10.0, 0.0, 8000.0), 0.0);
        assert_eq!(frazione(9000.0, 0.0, 8000.0), 1.0);
        assert_eq!(frazione(1.0, 5.0, 5.0), 0.0);
        assert_eq!(origine(-25.0, 25.0), 0.0);
        assert_eq!(origine(40.0, 130.0), 40.0);
    }

    #[test]
    fn formato_dei_valori() {
        assert_eq!(formatta("giri", 2128.0, None), "2.128");
        assert_eq!(formatta("stft_b1", 1.5, None), "+1,50");
        assert_eq!(formatta("stft_b1", 1.5, Some(1)), "+1,5");
        assert_eq!(formatta("velocita", 69.4, Some(0)), "69");
        assert_eq!(formatta("ignoto", 1.23456, None), "1,23");
    }

    #[test]
    fn etichette_degli_estremi() {
        assert_eq!(etichetta_scala(-25.0), "−25");
        assert_eq!(etichetta_scala(100.0), "100");
        assert_eq!(etichetta_scala(0.7), "0,7");
        assert_eq!(etichetta_scala(8000.0), "8.000");
    }

    #[test]
    fn note_di_stato() {
        assert_eq!(nota_stato(&StatoValore::Valido(1.0)), None);
        assert!(
            nota_stato(&StatoValore::Vecchio {
                valore: 1.0,
                secondi: 7.4
            })
            .unwrap()
            .0
            .contains("fermo da 7 s")
        );
        assert_eq!(
            nota_stato(&StatoValore::Mancante(Stato::RispostaMancante))
                .unwrap()
                .1,
            tema::ATTENZIONE
        );
        assert_eq!(
            nota_stato(&StatoValore::Mancante(Stato::ErroreComunicazione))
                .unwrap()
                .1,
            tema::PERICOLO
        );
    }

    #[test]
    fn rulli_del_contachilometri() {
        let p = posizioni_rulli(1234.5, 4, 1);
        assert_eq!(p.len(), 5);
        assert_eq!(&p[..4], &[1.0, 2.0, 3.0, 4.0]);
        assert!((p[4] - 5.0).abs() < 1e-9);
        // 129,95: l'ultimo rullo è ancora fermo sul 9 (scatta nell'ultimo quarto).
        let p = posizioni_rulli(129.95, 3, 1);
        assert!(
            p.iter()
                .zip([1.0, 2.0, 9.0, 9.0])
                .all(|(a, b)| (a - b).abs() < 1e-6)
        );
        // 129,99: l'ultimo rullo sta scattando e trascina unità e decine.
        let p = posizioni_rulli(129.99, 3, 1);
        let k = 0.648;
        assert!((p[3] - (9.0 + k)).abs() < 1e-3, "{p:?}");
        assert!((p[2] - (9.0 + k)).abs() < 1e-3, "{p:?}");
        assert!((p[1] - (2.0 + k)).abs() < 1e-3, "{p:?}");
        assert!((p[0] - 1.0).abs() < 1e-9);
        // 120,4: nessun riporto.
        let p = posizioni_rulli(120.4, 3, 1);
        assert_eq!(&p[..3], &[1.0, 2.0, 0.0]);
        // Valori negativi o enormi: nessun panico, cifre fra 0 e 10.
        assert!(posizioni_rulli(-3.0, 4, 2).iter().all(|q| *q == 0.0));
        assert!(
            posizioni_rulli(123_456_789.0, 6, 1)
                .iter()
                .all(|q| (0.0..10.0).contains(q))
        );
    }
}
