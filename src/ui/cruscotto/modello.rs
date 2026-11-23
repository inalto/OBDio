//! Modello dei cruscotti: pagine, strumenti, soglie, griglia e modelli pronti.
//!
//! Tutto ciò che sta qui è logica pura e serializzabile (nessun disegno), così
//! si verifica con i test e i file restano leggibili e tolleranti: campi
//! mancanti prendono il valore predefinito, campi e tipi sconosciuti non
//! impediscono il caricamento.

use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::obd::pids::{self, Kind};

/// Versione corrente del formato di `cruscotti.json`.
pub const VERSIONE: u32 = 1;
/// Nome del file nella cartella di configurazione.
pub const NOME_FILE: &str = "cruscotti.json";

/// Colonne della griglia su layout largo e su layout stretto.
pub const COLONNE_LARGO: u32 = 12;
pub const COLONNE_STRETTO: u32 = 4;
/// Oltre questa riga uno strumento non viene posato (file malformati).
pub const RIGHE_MAX: u32 = 200;

/// Sorgenti speciali per la chiave di uno strumento.
///
/// Le sonde si risolvono con la selezione «Diagnosi P0420» della centralina
/// collegata (lambda o tensione, secondo cosa espone); le spie leggono stati
/// dell'app invece di un parametro.
pub const SONDA_A_MONTE: &str = "@sonda_a_monte";
pub const SONDA_A_VALLE: &str = "@sonda_a_valle";
pub const SPIA_MIL: &str = "@mil";
pub const SPIA_REGISTRAZIONE: &str = "@registrazione";

// ----------------------------------------------------------------------
// Tipi
// ----------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TipoStrumento {
    Quadrante,
    Arco,
    Barra,
    #[default]
    Numerico,
    Grafico,
    Contachilometri,
    Spia,
}

impl TipoStrumento {
    pub const TUTTI: [TipoStrumento; 7] = [
        TipoStrumento::Quadrante,
        TipoStrumento::Arco,
        TipoStrumento::Barra,
        TipoStrumento::Numerico,
        TipoStrumento::Grafico,
        TipoStrumento::Contachilometri,
        TipoStrumento::Spia,
    ];

    pub fn codice(self) -> &'static str {
        match self {
            TipoStrumento::Quadrante => "quadrante",
            TipoStrumento::Arco => "arco",
            TipoStrumento::Barra => "barra",
            TipoStrumento::Numerico => "numerico",
            TipoStrumento::Grafico => "grafico",
            TipoStrumento::Contachilometri => "contachilometri",
            TipoStrumento::Spia => "spia",
        }
    }

    pub fn da_codice(c: &str) -> Option<Self> {
        Self::TUTTI.into_iter().find(|t| t.codice() == c)
    }

    pub fn nome(self) -> &'static str {
        match self {
            TipoStrumento::Quadrante => "Quadrante",
            TipoStrumento::Arco => "Arco",
            TipoStrumento::Barra => "Barra",
            TipoStrumento::Numerico => "Numerico",
            TipoStrumento::Grafico => "Grafico",
            TipoStrumento::Contachilometri => "Contachilometri",
            TipoStrumento::Spia => "Spia",
        }
    }

    /// Misura minima (colonne, righe). La barra accetta anche 1 × 2
    /// (verticale): vedi [`misura_ammessa`].
    pub fn misura_minima(self) -> (u32, u32) {
        match self {
            TipoStrumento::Quadrante => (2, 2),
            TipoStrumento::Arco => (2, 2),
            TipoStrumento::Barra => (2, 1),
            TipoStrumento::Numerico => (2, 1),
            TipoStrumento::Grafico => (3, 2),
            TipoStrumento::Contachilometri => (3, 1),
            TipoStrumento::Spia => (2, 1),
        }
    }

    /// Misura di uno strumento appena aggiunto.
    pub fn misura_predefinita(self) -> (u32, u32) {
        match self {
            TipoStrumento::Quadrante => (3, 3),
            TipoStrumento::Arco => (3, 2),
            TipoStrumento::Barra => (4, 1),
            TipoStrumento::Numerico => (3, 2),
            TipoStrumento::Grafico => (6, 2),
            TipoStrumento::Contachilometri => (4, 1),
            TipoStrumento::Spia => (3, 1),
        }
    }
}

impl Serialize for TipoStrumento {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.codice())
    }
}

/// Un tipo sconosciuto (file di una versione futura) diventa «numerico»,
/// che sa mostrare qualunque parametro, invece di far fallire il caricamento.
impl<'de> Deserialize<'de> for TipoStrumento {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Ok(v.as_str()
            .and_then(TipoStrumento::da_codice)
            .unwrap_or(TipoStrumento::Numerico))
    }
}

/// Verso di una soglia.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verso {
    /// Superata quando il valore è maggiore o uguale.
    #[default]
    Sopra,
    /// Superata quando il valore è minore o uguale.
    Sotto,
    /// Simmetrica attorno a zero: superata quando |valore| ≥ soglia
    /// (correzioni carburante ±10 %).
    Simmetrica,
}

impl Verso {
    pub const TUTTI: [Verso; 3] = [Verso::Sopra, Verso::Sotto, Verso::Simmetrica];

    pub fn nome(self) -> &'static str {
        match self {
            Verso::Sopra => "sopra",
            Verso::Sotto => "sotto",
            Verso::Simmetrica => "fuori da ± (simmetrica)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Soglia {
    pub valore: f64,
    pub verso: Verso,
}

impl Soglia {
    pub fn sopra(valore: f64) -> Self {
        Soglia {
            valore,
            verso: Verso::Sopra,
        }
    }

    pub fn sotto(valore: f64) -> Self {
        Soglia {
            valore,
            verso: Verso::Sotto,
        }
    }

    pub fn simmetrica(valore: f64) -> Self {
        Soglia {
            valore: valore.abs(),
            verso: Verso::Simmetrica,
        }
    }

    /// Vero se `v` è nella zona della soglia.
    pub fn superata(&self, v: f64) -> bool {
        match self.verso {
            Verso::Sopra => v >= self.valore,
            Verso::Sotto => v <= self.valore,
            Verso::Simmetrica => v.abs() >= self.valore.abs(),
        }
    }

    /// Intervalli della scala `[min, max]` in cui la soglia è superata.
    pub fn intervalli(&self, min: f64, max: f64) -> Vec<(f64, f64)> {
        let taglia = |a: f64, b: f64| {
            let (a, b) = (a.max(min), b.min(max));
            (a < b).then_some((a, b))
        };
        let s = self.valore;
        match self.verso {
            Verso::Sopra => taglia(s, max).into_iter().collect(),
            Verso::Sotto => taglia(min, s).into_iter().collect(),
            Verso::Simmetrica => {
                let s = s.abs();
                taglia(min, -s).into_iter().chain(taglia(s, max)).collect()
            }
        }
    }
}

/// Zona in cui cade un valore rispetto alle soglie dello strumento.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Livello {
    Normale,
    Attenzione,
    Critico,
}

/// La soglia critica prevale su quella di attenzione.
pub fn classifica_soglie(v: f64, attenzione: Option<&Soglia>, critica: Option<&Soglia>) -> Livello {
    if critica.is_some_and(|s| s.superata(v)) {
        Livello::Critico
    } else if attenzione.is_some_and(|s| s.superata(v)) {
        Livello::Attenzione
    } else {
        Livello::Normale
    }
}

// ----------------------------------------------------------------------
// Strumenti, pagine, insieme dei cruscotti
// ----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Strumento {
    /// Identificativo unico nell'insieme dei cruscotti.
    pub id: u64,
    pub tipo: TipoStrumento,
    /// Chiave del parametro (`obd::pids`) o sorgente speciale (`@…`).
    pub chiave: String,
    /// Titolo mostrato; `None` usa il nome del parametro.
    pub titolo: Option<String>,
    /// Posizione e misura in celle della griglia a 12 colonne.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub min: f64,
    pub max: f64,
    pub soglia_attenzione: Option<Soglia>,
    pub soglia_critica: Option<Soglia>,
    /// Decimali mostrati; `None` usa quelli del parametro.
    pub decimali: Option<usize>,
    pub mostra_picco: bool,
    /// Durata della finestra del tipo Grafico, in secondi.
    pub secondi: f64,
    /// Scala e soglie prese dai valori predefiniti del parametro risolto
    /// (per le sonde, che possono essere lambda o tensione).
    pub scala_automatica: bool,
}

impl Default for Strumento {
    fn default() -> Self {
        Strumento {
            id: 0,
            tipo: TipoStrumento::Numerico,
            chiave: String::new(),
            titolo: None,
            x: 0,
            y: 0,
            w: 3,
            h: 2,
            min: 0.0,
            max: 100.0,
            soglia_attenzione: None,
            soglia_critica: None,
            decimali: None,
            mostra_picco: false,
            secondi: 60.0,
            scala_automatica: false,
        }
    }
}

/// Scala e soglie predefinite di un parametro.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scala {
    pub min: f64,
    pub max: f64,
    pub attenzione: Option<Soglia>,
    pub critica: Option<Soglia>,
}

impl Scala {
    const fn semplice(min: f64, max: f64) -> Self {
        Scala {
            min,
            max,
            attenzione: None,
            critica: None,
        }
    }
}

/// Valori predefiniti di scala e soglie per parametro.
///
/// La pressione del collettore usa 0–255 kPa (l'intero campo del PID 0B):
/// copre i motori sovralimentati; per un aspirato si può restringere a
/// 0–105 kPa dall'ispettore.
pub fn scala_predefinita(chiave: &str) -> Scala {
    let soglie = |min, max, a: Soglia, c: Soglia| Scala {
        min,
        max,
        attenzione: Some(a),
        critica: Some(c),
    };
    if chiave.starts_with("o2s") {
        if chiave.ends_with("_lambda") || chiave.ends_with("_lambda_i") {
            return Scala::semplice(0.7, 1.3);
        }
        if chiave.ends_with("_tensione") {
            return Scala::semplice(0.0, 1.1);
        }
        if chiave.ends_with("_tensione_wr") {
            return Scala::semplice(0.0, 8.0);
        }
        if chiave.ends_with("_corrente") {
            return Scala::semplice(-5.0, 5.0);
        }
        if chiave.ends_with("_stft") {
            return soglie(
                -25.0,
                25.0,
                Soglia::simmetrica(10.0),
                Soglia::simmetrica(20.0),
            );
        }
    }
    match chiave {
        "giri" => soglie(0.0, 8000.0, Soglia::sopra(5500.0), Soglia::sopra(6500.0)),
        "velocita" => Scala::semplice(0.0, 220.0),
        "temp_refrigerante" => soglie(40.0, 130.0, Soglia::sopra(105.0), Soglia::sopra(115.0)),
        "temp_olio" => soglie(40.0, 150.0, Soglia::sopra(120.0), Soglia::sopra(135.0)),
        "temp_aspirazione" | "temp_ambiente" => Scala::semplice(-20.0, 80.0),
        "temp_cat_b1s1" | "temp_cat_b2s1" | "temp_cat_b1s2" | "temp_cat_b2s2" => {
            soglie(0.0, 1000.0, Soglia::sopra(850.0), Soglia::sopra(950.0))
        }
        "stft_b1" | "ltft_b1" | "stft_b2" | "ltft_b2" => soglie(
            -25.0,
            25.0,
            Soglia::simmetrica(10.0),
            Soglia::simmetrica(20.0),
        ),
        "lambda_comandato" => Scala::semplice(0.7, 1.3),
        "tensione_modulo" => soglie(10.0, 16.0, Soglia::sotto(12.0), Soglia::sotto(11.0)),
        "map" => Scala::semplice(0.0, 255.0),
        "livello_carburante" => soglie(0.0, 100.0, Soglia::sotto(15.0), Soglia::sotto(8.0)),
        "anticipo" => Scala::semplice(-20.0, 60.0),
        "maf" => Scala::semplice(0.0, 250.0),
        "pressione_barometrica" => Scala::semplice(60.0, 110.0),
        "consumo_istantaneo" => Scala::semplice(0.0, 30.0),
        "tempo_motore" => Scala::semplice(0.0, 3600.0),
        pids::CHIAVE_DISTANZA_PARZIALE => Scala::semplice(0.0, 1000.0),
        "td5_temp_gasolio" => Scala::semplice(-20.0, 120.0),
        "td5_alimentazione_sensori" => Scala::semplice(0.0, 6.0),
        k if k.starts_with("td5_pedale") => Scala::semplice(0.0, 5.0),
        k if k.starts_with("td5_iniettore") => Scala::semplice(-50.0, 50.0),
        "td5_egr" | "td5_wastegate" => Scala::semplice(0.0, 100.0),
        k if k.starts_with("td5_") => Scala::semplice(0.0, 1.0),
        _ => match pids::per_key(chiave).map(|p| p.kind) {
            Some(k) => scala_da_kind(k),
            None => Scala::semplice(0.0, 100.0),
        },
    }
}

/// Scala ragionevole ricavata dalla formula del parametro.
fn scala_da_kind(k: Kind) -> Scala {
    use Kind::*;
    match k {
        Percento | CaricoAssoluto | PercentoB | Coppia => Scala::semplice(0.0, 100.0),
        Temperatura => Scala::semplice(0.0, 150.0),
        Correzione | O2Correzione => Scala {
            min: -25.0,
            max: 25.0,
            attenzione: Some(Soglia::simmetrica(10.0)),
            critica: Some(Soglia::simmetrica(20.0)),
        },
        Byte => Scala::semplice(0.0, 255.0),
        Giri => Scala::semplice(0.0, 8000.0),
        Anticipo => Scala::semplice(-20.0, 60.0),
        Maf => Scala::semplice(0.0, 250.0),
        O2Tensione => Scala::semplice(0.0, 1.1),
        Lambda => Scala::semplice(0.7, 1.3),
        LambdaTensione => Scala::semplice(0.0, 8.0),
        LambdaCorrente => Scala::semplice(-5.0, 5.0),
        Parola => Scala::semplice(0.0, 1000.0),
        TemperaturaCat => Scala::semplice(0.0, 1000.0),
        Millivolt => Scala::semplice(10.0, 16.0),
        PressioneTripla => Scala::semplice(0.0, 765.0),
        PressioneRail => Scala::semplice(0.0, 100_000.0),
        ConsumoOrario => Scala::semplice(0.0, 30.0),
        Contatore32 => Scala::semplice(0.0, 1_000_000.0),
        Calcolato | Libero | Td5 => Scala::semplice(0.0, 100.0),
    }
}

/// Tipo di strumento suggerito per un parametro appena scelto.
pub fn tipo_suggerito(chiave: &str) -> TipoStrumento {
    match chiave {
        "giri" | "velocita" | "temp_refrigerante" | "map" | "temp_olio" => TipoStrumento::Quadrante,
        "contachilometri" | pids::CHIAVE_DISTANZA_PARZIALE => TipoStrumento::Contachilometri,
        SPIA_MIL | SPIA_REGISTRAZIONE => TipoStrumento::Spia,
        SONDA_A_MONTE | SONDA_A_VALLE => TipoStrumento::Grafico,
        _ => match pids::per_key(chiave).map(|p| p.kind) {
            Some(Kind::Correzione | Kind::O2Correzione | Kind::Percento) => TipoStrumento::Barra,
            _ => TipoStrumento::Numerico,
        },
    }
}

impl Strumento {
    /// Strumento con scala, soglie e misura predefinite per il parametro.
    pub fn predefinito(chiave: &str, tipo: TipoStrumento) -> Self {
        let (w, h) = tipo.misura_predefinita();
        let mut s = Strumento {
            tipo,
            chiave: chiave.to_string(),
            w,
            h,
            secondi: 60.0,
            scala_automatica: matches!(chiave, SONDA_A_MONTE | SONDA_A_VALLE),
            ..Default::default()
        };
        s.ripristina_scala();
        s
    }

    /// Ripristina scala, soglie, decimali e picco predefiniti (non tocca
    /// posizione, tipo, titolo e durata).
    pub fn ripristina_scala(&mut self) {
        let sc = scala_predefinita(&self.chiave);
        self.min = sc.min;
        self.max = sc.max;
        self.soglia_attenzione = sc.attenzione;
        self.soglia_critica = sc.critica;
        self.decimali = None;
        self.mostra_picco = matches!(self.chiave.as_str(), "giri" | "velocita");
    }

    /// Copia dello strumento con la scala del parametro risolto, se la scala
    /// è automatica. Corregge sempre un intervallo invertito o vuoto.
    pub fn con_scala_per(&self, chiave_risolta: &str) -> Strumento {
        let mut s = self.clone();
        if s.scala_automatica {
            let sc = scala_predefinita(chiave_risolta);
            s.min = sc.min;
            s.max = sc.max;
            s.soglia_attenzione = sc.attenzione;
            s.soglia_critica = sc.critica;
        }
        if !(s.min.is_finite() && s.max.is_finite()) || s.max <= s.min {
            let sc = scala_predefinita(chiave_risolta);
            s.min = sc.min;
            s.max = sc.max;
        }
        s
    }

    pub fn cella(&self) -> Cella {
        Cella {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }

    pub fn imposta_cella(&mut self, c: Cella) {
        self.x = c.x;
        self.y = c.y;
        self.w = c.w;
        self.h = c.h;
    }

    pub fn livello(&self, v: f64) -> Livello {
        classifica_soglie(
            v,
            self.soglia_attenzione.as_ref(),
            self.soglia_critica.as_ref(),
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pagina {
    pub nome: String,
    pub strumenti: Vec<Strumento>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cruscotti {
    pub versione: u32,
    pub pagine: Vec<Pagina>,
}

impl Default for Cruscotti {
    fn default() -> Self {
        Cruscotti {
            versione: VERSIONE,
            pagine: Vec::new(),
        }
    }
}

impl Cruscotti {
    /// Primo identificativo libero in tutte le pagine.
    pub fn nuovo_id(&self) -> u64 {
        self.pagine
            .iter()
            .flat_map(|p| p.strumenti.iter().map(|s| s.id))
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Rende il contenuto coerente: almeno una pagina, identificativi unici,
    /// strumenti dentro la griglia e senza sovrapposizioni.
    pub fn normalizza(&mut self) {
        self.versione = VERSIONE;
        if self.pagine.is_empty() {
            self.pagine.push(Pagina {
                nome: "Pagina 1".into(),
                strumenti: Vec::new(),
            });
        }
        let mut visti = BTreeSet::new();
        let mut prossimo = self.nuovo_id();
        for (i, p) in self.pagine.iter_mut().enumerate() {
            if p.nome.trim().is_empty() {
                p.nome = format!("Pagina {}", i + 1);
            }
            for s in &mut p.strumenti {
                if s.id == 0 || !visti.insert(s.id) {
                    s.id = prossimo;
                    visti.insert(prossimo);
                    prossimo += 1;
                }
            }
            normalizza_pagina(p);
        }
    }

    /// Aggiunge pagine importate con nuovi identificativi; restituisce
    /// l'indice della prima pagina aggiunta.
    pub fn aggiungi_pagine(&mut self, pagine: Vec<Pagina>) -> usize {
        let primo = self.pagine.len();
        let mut id = self.nuovo_id();
        for mut p in pagine {
            for s in &mut p.strumenti {
                s.id = id;
                id += 1;
            }
            normalizza_pagina(&mut p);
            self.pagine.push(p);
        }
        primo
    }
}

/// Porta ogni strumento dentro la griglia a 12 colonne con una misura
/// ammessa; chi si sovrappone a uno precedente viene spostato nel primo
/// posto libero.
pub fn normalizza_pagina(p: &mut Pagina) {
    let mut posati: Vec<Strumento> = Vec::with_capacity(p.strumenti.len());
    for mut s in std::mem::take(&mut p.strumenti) {
        if !(s.secondi.is_finite() && s.secondi > 0.0) {
            s.secondi = 60.0;
        }
        let c = limita(s.cella(), s.tipo, COLONNE_LARGO);
        s.imposta_cella(c);
        if s.y > RIGHE_MAX || collisione(&posati, c, None) {
            let (x, y) = primo_libero(&posati, c.w, c.h, COLONNE_LARGO);
            s.x = x;
            s.y = y;
        }
        posati.push(s);
    }
    p.strumenti = posati;
}

// ----------------------------------------------------------------------
// Griglia
// ----------------------------------------------------------------------

/// Rettangolo di celle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cella {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Cella {
    pub fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Cella { x, y, w, h }
    }

    pub fn destra(&self) -> u32 {
        self.x + self.w
    }

    pub fn fondo(&self) -> u32 {
        self.y + self.h
    }

    pub fn sovrapposta(&self, o: &Cella) -> bool {
        self.x < o.destra() && o.x < self.destra() && self.y < o.fondo() && o.y < self.fondo()
    }
}

/// Vero se la misura è ammessa per il tipo (la barra anche verticale 1 × 2).
pub fn misura_ammessa(tipo: TipoStrumento, w: u32, h: u32) -> bool {
    let (mw, mh) = tipo.misura_minima();
    (w >= mw && h >= mh) || (tipo == TipoStrumento::Barra && w >= 1 && h >= 2)
}

/// Vero se `c` si sovrappone a uno strumento diverso da `escluso`.
pub fn collisione(strumenti: &[Strumento], c: Cella, escluso: Option<u64>) -> bool {
    strumenti
        .iter()
        .filter(|s| Some(s.id) != escluso)
        .any(|s| s.cella().sovrapposta(&c))
}

/// Riporta un rettangolo dentro la griglia con una misura ammessa.
pub fn limita(c: Cella, tipo: TipoStrumento, colonne: u32) -> Cella {
    let (mw, mh) = tipo.misura_minima();
    let mut w = c.w.clamp(1, colonne);
    let mut h = c.h.clamp(1, RIGHE_MAX);
    if !misura_ammessa(tipo, w, h) {
        w = w.max(mw.min(colonne));
        h = h.max(mh);
    }
    let x = c.x.min(colonne - w);
    Cella::new(x, c.y.min(RIGHE_MAX), w, h)
}

/// Primo posto libero (per righe, poi per colonne) per un rettangolo `w × h`.
pub fn primo_libero(strumenti: &[Strumento], w: u32, h: u32, colonne: u32) -> (u32, u32) {
    primo_libero_da(strumenti, w, h, colonne, (0, 0))
}

/// Come [`primo_libero`] ma senza tornare prima di `da` = (x, y) in ordine
/// di lettura.
fn primo_libero_da(
    strumenti: &[Strumento],
    w: u32,
    h: u32,
    colonne: u32,
    da: (u32, u32),
) -> (u32, u32) {
    let w = w.clamp(1, colonne);
    let ultima = strumenti.iter().map(|s| s.y + s.h).max().unwrap_or(0);
    for y in da.1..=ultima.max(da.1) {
        let x0 = if y == da.1 { da.0 } else { 0 };
        for x in x0..=(colonne - w) {
            if !collisione(strumenti, Cella::new(x, y, w, h), None) {
                return (x, y);
            }
        }
    }
    (0, ultima.max(da.1 + 1))
}

/// Righe occupate dalla pagina.
pub fn righe_usate(strumenti: &[Strumento]) -> u32 {
    strumenti.iter().map(|s| s.y + s.h).max().unwrap_or(0)
}

/// Ricompone la griglia a 12 colonne su `colonne` (4 su schermi stretti).
///
/// Deterministica: gli strumenti si prendono in ordine di lettura (riga,
/// poi colonna); la larghezza si scala in proporzione (arrotondata per
/// eccesso, mai sotto la minima del tipo; da metà griglia in su diventa
/// l'intera larghezza); l'altezza si riduce nella stessa proporzione, così un
/// quadrante resta quadrato (mai sotto la minima, mai sopra l'originale); ognuno
/// va nel primo posto libero che non precede quello del precedente.
pub fn ricomponi(strumenti: &[Strumento], colonne: u32) -> Vec<Strumento> {
    if colonne >= COLONNE_LARGO {
        return strumenti.to_vec();
    }
    let mut ordine: Vec<&Strumento> = strumenti.iter().collect();
    ordine.sort_by_key(|s| (s.y, s.x, s.id));
    let mut fuori: Vec<Strumento> = Vec::with_capacity(ordine.len());
    let mut da = (0, 0);
    for s in ordine {
        let w = if s.w * 2 >= COLONNE_LARGO {
            colonne
        } else {
            (s.w * colonne).div_ceil(COLONNE_LARGO)
        };
        let w = limita(Cella::new(0, 0, w, s.h), s.tipo, colonne).w;
        let h = ((s.h * w) as f32 / s.w.max(1) as f32).round() as u32;
        let c = limita(Cella::new(0, 0, w, h.clamp(1, s.h.max(1))), s.tipo, colonne);
        let (x, y) = primo_libero_da(&fuori, c.w, c.h, colonne, da);
        let mut n = s.clone();
        n.imposta_cella(Cella::new(x, y, c.w, c.h));
        da = (x, y);
        fuori.push(n);
    }
    fuori
}

// ----------------------------------------------------------------------
// Modelli pronti
// ----------------------------------------------------------------------

fn posa(id: &mut u64, chiave: &str, tipo: TipoStrumento, c: (u32, u32, u32, u32)) -> Strumento {
    let mut s = Strumento::predefinito(chiave, tipo);
    *id += 1;
    s.id = *id;
    s.imposta_cella(Cella::new(c.0, c.1, c.2, c.3));
    s
}

/// I tre modelli pronti: «Guida», «Motore», «Diagnosi P0420».
///
/// Usano chiavi fisse o sorgenti risolte alla connessione: uno strumento il
/// cui parametro non è esposto dalla centralina resta al suo posto e mostra
/// «Non supportato».
pub fn modelli() -> Cruscotti {
    use TipoStrumento::*;
    let mut id = 0;
    let mut guida = vec![
        posa(&mut id, "giri", Quadrante, (0, 0, 4, 4)),
        posa(&mut id, "velocita", Quadrante, (4, 0, 4, 4)),
        posa(&mut id, "temp_refrigerante", Arco, (8, 0, 4, 2)),
        posa(&mut id, "carico", Barra, (8, 2, 4, 2)),
        posa(&mut id, "contachilometri", Contachilometri, (0, 4, 4, 2)),
        posa(&mut id, "velocita", Grafico, (4, 4, 8, 2)),
    ];
    guida[1].mostra_picco = true;
    guida[5].titolo = Some("Velocità — ultimo minuto".into());

    let motore = vec![
        posa(&mut id, "map", Quadrante, (0, 0, 4, 4)),
        posa(&mut id, "anticipo", Quadrante, (4, 0, 4, 4)),
        posa(&mut id, "temp_refrigerante", Arco, (8, 0, 4, 2)),
        posa(&mut id, "temp_aspirazione", Arco, (8, 2, 4, 2)),
        posa(&mut id, "tensione_modulo", Numerico, (0, 4, 4, 2)),
        posa(&mut id, "farfalla", Barra, (4, 4, 4, 2)),
        posa(&mut id, "temp_olio", Numerico, (8, 4, 4, 2)),
    ];

    let mut p0420 = vec![
        posa(&mut id, SONDA_A_MONTE, Grafico, (0, 0, 8, 2)),
        posa(&mut id, SONDA_A_VALLE, Grafico, (0, 2, 8, 2)),
        posa(&mut id, "giri", Arco, (8, 0, 4, 2)),
        posa(&mut id, "stft_b1", Barra, (8, 2, 4, 1)),
        posa(&mut id, "ltft_b1", Barra, (8, 3, 4, 1)),
        posa(&mut id, SONDA_A_MONTE, Numerico, (0, 4, 4, 2)),
        posa(&mut id, SONDA_A_VALLE, Numerico, (4, 4, 4, 2)),
        posa(&mut id, SPIA_MIL, Spia, (8, 4, 4, 1)),
        posa(&mut id, SPIA_REGISTRAZIONE, Spia, (8, 5, 4, 1)),
    ];
    for s in &mut p0420[..2] {
        s.secondi = 30.0;
    }

    Cruscotti {
        versione: VERSIONE,
        pagine: vec![
            Pagina {
                nome: "Guida".into(),
                strumenti: guida,
            },
            Pagina {
                nome: "Motore".into(),
                strumenti: motore,
            },
            Pagina {
                nome: "Diagnosi P0420".into(),
                strumenti: p0420,
            },
        ],
    }
}

// ----------------------------------------------------------------------
// Importazione
// ----------------------------------------------------------------------

/// Legge un file esportato: l'intero insieme (`pagine`) o una sola pagina
/// (`strumenti`). Gli errori sono in italiano, pronti per l'utente.
pub fn leggi_importazione(testo: &str) -> Result<Vec<Pagina>, String> {
    let valore: serde_json::Value = serde_json::from_str(testo.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("il file non è un JSON valido ({e})"))?;
    let Some(oggetto) = valore.as_object() else {
        return Err("il file non contiene un cruscotto di OBDio".into());
    };
    let pagine = if oggetto.contains_key("pagine") {
        let c: Cruscotti = serde_json::from_value(valore.clone())
            .map_err(|e| format!("l'elenco delle pagine non è leggibile ({e})"))?;
        c.pagine
    } else if oggetto.contains_key("strumenti") {
        let p: Pagina = serde_json::from_value(valore.clone())
            .map_err(|e| format!("la pagina non è leggibile ({e})"))?;
        vec![p]
    } else {
        return Err("il file non contiene pagine né strumenti di un cruscotto".into());
    };
    if pagine.is_empty() {
        return Err("il file non contiene pagine".into());
    }
    Ok(pagine)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: u64, tipo: TipoStrumento, x: u32, y: u32, w: u32, h: u32) -> Strumento {
        let mut st = Strumento::predefinito("giri", tipo);
        st.id = id;
        st.imposta_cella(Cella::new(x, y, w, h));
        st
    }

    #[test]
    fn sovrapposizione_e_collisione() {
        let a = Cella::new(0, 0, 3, 3);
        assert!(a.sovrapposta(&Cella::new(2, 2, 2, 2)));
        // Bordi che si toccano non si sovrappongono.
        assert!(!a.sovrapposta(&Cella::new(3, 0, 2, 2)));
        assert!(!a.sovrapposta(&Cella::new(0, 3, 2, 2)));
        let v = vec![s(1, TipoStrumento::Quadrante, 0, 0, 3, 3)];
        assert!(collisione(&v, Cella::new(1, 1, 2, 2), None));
        // Lo strumento stesso non collide con sé (spostamento).
        assert!(!collisione(&v, Cella::new(1, 1, 2, 2), Some(1)));
    }

    #[test]
    fn primo_posto_libero() {
        let v = vec![
            s(1, TipoStrumento::Quadrante, 0, 0, 4, 4),
            s(2, TipoStrumento::Quadrante, 4, 0, 4, 4),
        ];
        assert_eq!(primo_libero(&v, 4, 4, 12), (8, 0));
        assert_eq!(primo_libero(&v, 5, 2, 12), (0, 4));
        assert_eq!(primo_libero(&[], 3, 3, 12), (0, 0));
        // Più largo della griglia: si limita alla larghezza.
        assert_eq!(primo_libero(&v, 20, 1, 12), (0, 4));
    }

    #[test]
    fn limiti_della_griglia_e_misure_minime() {
        // Esce a destra: rientra.
        assert_eq!(
            limita(Cella::new(10, 0, 4, 4), TipoStrumento::Quadrante, 12),
            Cella::new(8, 0, 4, 4)
        );
        // Troppo piccolo per un quadrante: cresce alla minima.
        assert_eq!(
            limita(Cella::new(0, 0, 1, 1), TipoStrumento::Quadrante, 12),
            Cella::new(0, 0, 2, 2)
        );
        // Barra verticale 1 × 2 ammessa, 1 × 1 no.
        assert!(misura_ammessa(TipoStrumento::Barra, 1, 2));
        assert!(!misura_ammessa(TipoStrumento::Barra, 1, 1));
        assert_eq!(
            limita(Cella::new(0, 0, 1, 3), TipoStrumento::Barra, 12),
            Cella::new(0, 0, 1, 3)
        );
        // Più largo della griglia stretta.
        assert_eq!(
            limita(Cella::new(2, 0, 8, 2), TipoStrumento::Grafico, 4),
            Cella::new(0, 0, 4, 2)
        );
    }

    #[test]
    fn ricomposizione_da_12_a_4_colonne() {
        let v = vec![
            s(1, TipoStrumento::Quadrante, 4, 0, 4, 4),
            s(2, TipoStrumento::Quadrante, 0, 0, 4, 4),
            s(3, TipoStrumento::Arco, 8, 0, 4, 2),
            s(4, TipoStrumento::Barra, 8, 2, 4, 2),
            s(5, TipoStrumento::Grafico, 0, 4, 12, 2),
            s(6, TipoStrumento::Spia, 0, 6, 3, 1),
            s(7, TipoStrumento::Spia, 3, 6, 3, 1),
        ];
        let r = ricomponi(&v, 4);
        let pos: Vec<(u64, Cella)> = r.iter().map(|s| (s.id, s.cella())).collect();
        assert_eq!(
            pos,
            vec![
                (2, Cella::new(0, 0, 2, 2)),
                (1, Cella::new(2, 0, 2, 2)),
                (3, Cella::new(0, 2, 2, 2)),
                (4, Cella::new(2, 2, 2, 1)),
                (5, Cella::new(0, 4, 4, 2)),
                (6, Cella::new(0, 6, 2, 1)),
                (7, Cella::new(2, 6, 2, 1)),
            ]
        );
        // Nessuna sovrapposizione e tutto dentro 4 colonne.
        for (i, a) in r.iter().enumerate() {
            assert!(a.x + a.w <= 4);
            for b in &r[i + 1..] {
                assert!(!a.cella().sovrapposta(&b.cella()), "{} e {}", a.id, b.id);
            }
        }
        // Deterministica.
        assert_eq!(ricomponi(&v, 4), r);
        // A 12 colonne resta identica.
        assert_eq!(ricomponi(&v, 12), v);
    }

    #[test]
    fn i_modelli_non_si_sovrappongono() {
        let m = modelli();
        assert_eq!(m.pagine.len(), 3);
        for p in &m.pagine {
            for (i, a) in p.strumenti.iter().enumerate() {
                assert!(a.x + a.w <= COLONNE_LARGO, "{}", p.nome);
                assert!(misura_ammessa(a.tipo, a.w, a.h), "{} {}", p.nome, a.chiave);
                for b in &p.strumenti[i + 1..] {
                    assert!(!a.cella().sovrapposta(&b.cella()), "{}", p.nome);
                }
            }
            let r = ricomponi(&p.strumenti, COLONNE_STRETTO);
            assert_eq!(r.len(), p.strumenti.len());
        }
        let mut copia = m.clone();
        copia.normalizza();
        assert_eq!(copia, m, "i modelli sono già normalizzati");
    }

    #[test]
    fn valori_predefiniti_per_parametro() {
        let g = Strumento::predefinito("giri", TipoStrumento::Quadrante);
        assert_eq!((g.min, g.max), (0.0, 8000.0));
        assert_eq!(g.soglia_critica, Some(Soglia::sopra(6500.0)));
        assert_eq!(g.soglia_attenzione, Some(Soglia::sopra(5500.0)));
        assert_eq!((g.w, g.h), (3, 3));
        let t = Strumento::predefinito("temp_refrigerante", TipoStrumento::Arco);
        assert_eq!((t.min, t.max), (40.0, 130.0));
        assert_eq!(t.soglia_critica, Some(Soglia::sopra(115.0)));
        let v = Strumento::predefinito("tensione_modulo", TipoStrumento::Numerico);
        assert_eq!(v.soglia_attenzione, Some(Soglia::sotto(12.0)));
        assert_eq!(v.soglia_critica, Some(Soglia::sotto(11.0)));
        let f = Strumento::predefinito("ltft_b1", TipoStrumento::Barra);
        assert_eq!((f.min, f.max), (-25.0, 25.0));
        assert_eq!(f.soglia_attenzione, Some(Soglia::simmetrica(10.0)));
        assert_eq!(scala_predefinita("o2s2_tensione").max, 1.1);
        assert_eq!(scala_predefinita("o2s1_lambda").min, 0.7);
        assert_eq!(scala_predefinita("map").max, 255.0);
        assert_eq!(scala_predefinita("velocita").max, 220.0);
        assert_eq!(scala_predefinita("pedale_d").max, 100.0);
        // Ricavata dalla formula: temperatura catalizzatore e pressione rail.
        assert_eq!(scala_predefinita("pressione_rail").max, 100_000.0);
        // Chiave sconosciuta: scala generica, mai un errore.
        assert_eq!(scala_predefinita("ignoto").max, 100.0);
        // Sonde: scala automatica, risolta sul parametro esposto.
        let sonda = Strumento::predefinito(SONDA_A_VALLE, TipoStrumento::Grafico);
        assert!(sonda.scala_automatica);
        let r = sonda.con_scala_per("o2s2_tensione");
        assert_eq!((r.min, r.max), (0.0, 1.1));
        let r = sonda.con_scala_per("o2s2_lambda_i");
        assert_eq!((r.min, r.max), (0.7, 1.3));
    }

    #[test]
    fn classificazione_delle_soglie() {
        let g = Strumento::predefinito("giri", TipoStrumento::Quadrante);
        assert_eq!(g.livello(3000.0), Livello::Normale);
        assert_eq!(g.livello(5500.0), Livello::Attenzione);
        assert_eq!(g.livello(7000.0), Livello::Critico);
        // Verso «sotto»: la tensione bassa è il problema.
        let v = Strumento::predefinito("tensione_modulo", TipoStrumento::Numerico);
        assert_eq!(v.livello(13.9), Livello::Normale);
        assert_eq!(v.livello(11.8), Livello::Attenzione);
        assert_eq!(v.livello(10.5), Livello::Critico);
        // Simmetrica: ±10 attenzione, ±20 critica.
        let f = Strumento::predefinito("stft_b1", TipoStrumento::Barra);
        assert_eq!(f.livello(4.0), Livello::Normale);
        assert_eq!(f.livello(-4.0), Livello::Normale);
        assert_eq!(f.livello(12.0), Livello::Attenzione);
        assert_eq!(f.livello(-12.0), Livello::Attenzione);
        assert_eq!(f.livello(-21.0), Livello::Critico);
        // Senza soglie è sempre normale.
        assert_eq!(classifica_soglie(1e9, None, None), Livello::Normale);
    }

    #[test]
    fn intervalli_delle_zone() {
        assert_eq!(
            Soglia::sopra(6500.0).intervalli(0.0, 8000.0),
            vec![(6500.0, 8000.0)]
        );
        assert_eq!(
            Soglia::sotto(11.0).intervalli(10.0, 16.0),
            vec![(10.0, 11.0)]
        );
        assert_eq!(
            Soglia::simmetrica(10.0).intervalli(-25.0, 25.0),
            vec![(-25.0, -10.0), (10.0, 25.0)]
        );
        // Fuori scala: nessuna zona.
        assert!(Soglia::sopra(9000.0).intervalli(0.0, 8000.0).is_empty());
        // Simmetrica su una scala solo positiva: resta la parte alta.
        assert_eq!(
            Soglia::simmetrica(10.0).intervalli(0.0, 25.0),
            vec![(10.0, 25.0)]
        );
    }

    #[test]
    fn andata_e_ritorno_in_json() {
        let m = modelli();
        let testo = serde_json::to_string_pretty(&m).unwrap();
        assert!(testo.contains("\"quadrante\""));
        assert!(testo.contains("\"simmetrica\""));
        let letto: Cruscotti = serde_json::from_str(&testo).unwrap();
        assert_eq!(letto, m);
    }

    #[test]
    fn caricamento_tollerante() {
        let testo = r#"{
            "versione": 7,
            "campo_futuro": {"qualcosa": 1},
            "pagine": [{
                "nome": "Mia",
                "colore": "blu",
                "strumenti": [
                    {"id": 4, "tipo": "lancetta_3d", "chiave": "parametro_inesistente", "x": 0, "y": 0, "w": 3, "h": 2},
                    {"tipo": "quadrante", "chiave": "giri", "x": 11, "y": 0, "w": 3, "h": 3, "soglia_critica": {"valore": 6000}},
                    {"id": 4, "tipo": "spia", "chiave": "@mil", "x": 0, "y": 0}
                ]
            }]
        }"#;
        let mut c: Cruscotti = serde_json::from_str(testo).unwrap();
        c.normalizza();
        assert_eq!(c.versione, VERSIONE);
        let p = &c.pagine[0];
        assert_eq!(p.nome, "Mia");
        assert_eq!(p.strumenti.len(), 3);
        // Tipo sconosciuto: numerico; chiave sconosciuta conservata.
        assert_eq!(p.strumenti[0].tipo, TipoStrumento::Numerico);
        assert_eq!(p.strumenti[0].chiave, "parametro_inesistente");
        // Fuori griglia: rientra; verso mancante: «sopra».
        assert_eq!(p.strumenti[1].x + p.strumenti[1].w, 12);
        assert_eq!(p.strumenti[1].soglia_critica, Some(Soglia::sopra(6000.0)));
        // Identificativi resi unici, sovrapposizioni risolte.
        let ids: BTreeSet<u64> = p.strumenti.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(!ids.contains(&0));
        for (i, a) in p.strumenti.iter().enumerate() {
            for b in &p.strumenti[i + 1..] {
                assert!(!a.cella().sovrapposta(&b.cella()));
            }
        }
        // Un file vuoto diventa una pagina vuota.
        let mut vuoto: Cruscotti = serde_json::from_str("{}").unwrap();
        vuoto.normalizza();
        assert_eq!(vuoto.pagine.len(), 1);
    }

    #[test]
    fn importazione_di_pagina_o_insieme_e_errori() {
        let m = modelli();
        let tutto = serde_json::to_string(&m).unwrap();
        assert_eq!(leggi_importazione(&tutto).unwrap().len(), 3);
        let una = serde_json::to_string(&m.pagine[2]).unwrap();
        let p = leggi_importazione(&una).unwrap();
        assert_eq!(p[0].nome, "Diagnosi P0420");

        assert!(leggi_importazione("non json").unwrap_err().contains("JSON"));
        assert!(leggi_importazione("[1, 2]").is_err());
        assert!(leggi_importazione(r#"{"a": 1}"#).is_err());
        assert!(leggi_importazione(r#"{"pagine": []}"#).is_err());
        assert!(leggi_importazione(r#"{"pagine": 3}"#).is_err());

        let mut c = modelli();
        let primo = c.aggiungi_pagine(p);
        assert_eq!(primo, 3);
        let ids: Vec<u64> = c
            .pagine
            .iter()
            .flat_map(|p| p.strumenti.iter().map(|s| s.id))
            .collect();
        let unici: BTreeSet<u64> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unici.len(), "identificativi duplicati");
    }
}
