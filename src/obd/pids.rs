//! Catalogo dei parametri dei servizi 01/02 e relative formule (SAE J1979).
//!
//! Ogni parametro ha una chiave stabile usata nei file. Un PID può produrre
//! più parametri (per esempio lambda e tensione della stessa sonda).

use std::collections::BTreeSet;
use std::sync::{OnceLock, RwLock};

/// Formula di decodifica; i byte sono quelli che seguono il PID.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A·100/255 %
    Percento,
    /// A − 40 °C
    Temperatura,
    /// (A − 128)·100/128 %
    Correzione,
    /// A, senza conversione
    Byte,
    /// (256A + B)/4 giri/min
    Giri,
    /// A/2 − 64 °
    Anticipo,
    /// (256A + B)/100 g/s
    Maf,
    /// A/200 V (sonda a banda stretta)
    O2Tensione,
    /// (B − 128)·100/128 %; B = FF: sonda non usata per la correzione
    O2Correzione,
    /// (256A + B)·2/65536 (rapporto di equivalenza λ)
    Lambda,
    /// (256C + D)·8/65536 V
    LambdaTensione,
    /// (256C + D)/256 − 128 mA
    LambdaCorrente,
    /// 256A + B
    Parola,
    /// (256A + B)/10 − 40 °C
    TemperaturaCat,
    /// (256A + B)/1000 V
    Millivolt,
    /// (256A + B)·100/255 %
    CaricoAssoluto,
    /// 3·A kPa (pressione carburante, PID 0A)
    PressioneTripla,
    /// 10·(256A + B) kPa (pressione rail relativa, PID 23)
    PressioneRail,
    /// B·100/255 % (il secondo byte, per esempio farfalla assoluta B)
    PercentoB,
    /// (256A + B)/20 L/h
    ConsumoOrario,
    /// A − 125 % (coppia, PID 61 e 62)
    Coppia,
    /// (A·2²⁴ + B·2¹⁶ + C·2⁸ + D)/10 (contachilometri, PID A6)
    Contatore32,
    /// Grandezza calcolata dall'app: nessun PID, mai richiesta alla centralina.
    Calcolato,
    /// Parametro «libero» venuto da un file importato (per esempio una
    /// colonna sconosciuta di un CSV di Torque): nessun PID, mai richiesto.
    Libero,
}

/// Raggruppamento dei parametri nella pagina Sensori.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Gruppo {
    Motore,
    CarburanteEMiscela,
    SondeLambda,
    TemperatureEPressioni,
    DistanzeETempi,
    Altro,
}

impl Gruppo {
    /// Ordine di comparsa nell'elenco.
    pub const TUTTI: [Gruppo; 6] = [
        Gruppo::Motore,
        Gruppo::CarburanteEMiscela,
        Gruppo::SondeLambda,
        Gruppo::TemperatureEPressioni,
        Gruppo::DistanzeETempi,
        Gruppo::Altro,
    ];

    pub fn nome(self) -> &'static str {
        match self {
            Gruppo::Motore => "Motore",
            Gruppo::CarburanteEMiscela => "Carburante e miscela",
            Gruppo::SondeLambda => "Sonde lambda",
            Gruppo::TemperatureEPressioni => "Temperature e pressioni",
            Gruppo::DistanzeETempi => "Distanze e tempi",
            Gruppo::Altro => "Altro",
        }
    }
}

fn gruppo_di(key: &str) -> Gruppo {
    if key.starts_with("o2s") {
        return Gruppo::SondeLambda;
    }
    if key.starts_with("temp_") || key.starts_with("pressione_barometrica") || key == "map" {
        return Gruppo::TemperatureEPressioni;
    }
    match key {
        "stft_b1"
        | "ltft_b1"
        | "stft_b2"
        | "ltft_b2"
        | "livello_carburante"
        | "pressione_carburante"
        | "pressione_rail"
        | "consumo_istantaneo"
        | "lambda_comandato" => Gruppo::CarburanteEMiscela,
        "tempo_motore"
        | "km_con_mil"
        | "km_da_cancellazione"
        | "avviamenti_da_cancellazione"
        | "minuti_con_mil"
        | "minuti_da_cancellazione"
        | "contachilometri"
        | CHIAVE_DISTANZA_PARZIALE => Gruppo::DistanzeETempi,
        "carico"
        | "giri"
        | "velocita"
        | "anticipo"
        | "maf"
        | "farfalla"
        | "farfalla_relativa"
        | "farfalla_assoluta_b"
        | "farfalla_comandata"
        | "pedale_d"
        | "pedale_e"
        | "pedale_relativo"
        | "carico_assoluto"
        | "coppia_richiesta"
        | "coppia_effettiva"
        | "coppia_riferimento" => Gruppo::Motore,
        _ => Gruppo::Altro,
    }
}

/// Chiave della distanza percorsa dalla connessione, calcolata dall'app.
pub const CHIAVE_DISTANZA_PARZIALE: &str = "distanza_parziale";

/// Se `key` è una grandezza calcolata, la chiave del parametro da cui deriva.
pub fn sorgente_derivato(key: &str) -> Option<&'static str> {
    match key {
        CHIAVE_DISTANZA_PARZIALE => Some("velocita"),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub key: String,
    pub pid: u8,
    pub nome: String,
    pub unita: &'static str,
    pub kind: Kind,
    pub decimali: usize,
    /// Mostrare sempre il segno (correzioni carburante).
    pub con_segno: bool,
    pub gruppo: Gruppo,
    /// Calcolato dall'app, non letto dalla centralina: escluso dalla
    /// decodifica per PID e mai richiesto.
    pub derivato: bool,
    /// Parametro libero (fuori dal catalogo) con nome e unità presi da un
    /// file importato: mai richiesto alla centralina. Vedi [`registra_libero`].
    pub libero: bool,
}

fn p(key: &str, pid: u8, nome: &str, unita: &'static str, kind: Kind, decimali: usize) -> Param {
    Param {
        key: key.to_string(),
        pid,
        nome: nome.to_string(),
        unita,
        kind,
        decimali,
        con_segno: matches!(kind, Kind::Correzione | Kind::O2Correzione),
        gruppo: gruppo_di(key),
        derivato: kind == Kind::Calcolato,
        libero: false,
    }
}

fn nome_sonda(n: u8) -> String {
    match n {
        1 => "Sonda B1S1 (a monte)".to_string(),
        2 => "Sonda B1S2 (a valle)".to_string(),
        _ => format!("Sonda O2 n. {n}"),
    }
}

fn costruisci() -> Vec<Param> {
    use Kind::*;
    let mut v = vec![
        p("carico", 0x04, "Carico motore calcolato", "%", Percento, 1),
        p(
            "temp_refrigerante",
            0x05,
            "Temperatura refrigerante",
            "°C",
            Temperatura,
            0,
        ),
        p("stft_b1", 0x06, "STFT bancata 1", "%", Correzione, 2),
        p("ltft_b1", 0x07, "LTFT bancata 1", "%", Correzione, 2),
        p("stft_b2", 0x08, "STFT bancata 2", "%", Correzione, 2),
        p("ltft_b2", 0x09, "LTFT bancata 2", "%", Correzione, 2),
        p("map", 0x0B, "Pressione collettore", "kPa", Byte, 0),
        p("giri", 0x0C, "Giri motore", "giri/min", Giri, 0),
        p("velocita", 0x0D, "Velocità", "km/h", Byte, 0),
        p("anticipo", 0x0E, "Anticipo di accensione", "°", Anticipo, 1),
        p(
            "temp_aspirazione",
            0x0F,
            "Temperatura aria aspirata",
            "°C",
            Temperatura,
            0,
        ),
        p("maf", 0x10, "Portata aria (MAF)", "g/s", Maf, 2),
        p("farfalla", 0x11, "Posizione farfalla", "%", Percento, 1),
        p(
            "tempo_motore",
            0x1F,
            "Tempo dall'avviamento",
            "s",
            Parola,
            0,
        ),
        p(
            "km_con_mil",
            0x21,
            "Distanza con MIL accesa",
            "km",
            Parola,
            0,
        ),
        p(
            "livello_carburante",
            0x2F,
            "Livello carburante",
            "%",
            Percento,
            1,
        ),
        p(
            "km_da_cancellazione",
            0x31,
            "Distanza dalla cancellazione DTC",
            "km",
            Parola,
            0,
        ),
        p(
            "pressione_barometrica",
            0x33,
            "Pressione barometrica",
            "kPa",
            Byte,
            0,
        ),
        p(
            "temp_cat_b1s1",
            0x3C,
            "Temperatura catalizzatore B1S1",
            "°C",
            TemperaturaCat,
            1,
        ),
        p(
            "temp_cat_b2s1",
            0x3D,
            "Temperatura catalizzatore B2S1",
            "°C",
            TemperaturaCat,
            1,
        ),
        p(
            "temp_cat_b1s2",
            0x3E,
            "Temperatura catalizzatore B1S2",
            "°C",
            TemperaturaCat,
            1,
        ),
        p(
            "temp_cat_b2s2",
            0x3F,
            "Temperatura catalizzatore B2S2",
            "°C",
            TemperaturaCat,
            1,
        ),
        p(
            "tensione_modulo",
            0x42,
            "Tensione modulo di controllo",
            "V",
            Millivolt,
            2,
        ),
        p(
            "carico_assoluto",
            0x43,
            "Carico assoluto",
            "%",
            CaricoAssoluto,
            1,
        ),
        p("lambda_comandato", 0x44, "Lambda comandato", "λ", Lambda, 3),
        p(
            "temp_ambiente",
            0x46,
            "Temperatura ambiente",
            "°C",
            Temperatura,
            0,
        ),
        p("temp_olio", 0x5C, "Temperatura olio", "°C", Temperatura, 0),
        p(
            "pressione_carburante",
            0x0A,
            "Pressione carburante",
            "kPa",
            PressioneTripla,
            0,
        ),
        p(
            "pressione_rail",
            0x23,
            "Pressione rail carburante (relativa)",
            "kPa",
            PressioneRail,
            0,
        ),
        p(
            "avviamenti_da_cancellazione",
            0x30,
            "Avviamenti a caldo dalla cancellazione DTC",
            "",
            Byte,
            0,
        ),
        p(
            "farfalla_relativa",
            0x45,
            "Posizione farfalla relativa",
            "%",
            Percento,
            1,
        ),
        p(
            "farfalla_assoluta_b",
            0x47,
            "Posizione farfalla assoluta B",
            "%",
            PercentoB,
            1,
        ),
        // Le formule dei PID 49, 4A e 4C sono dedotte da righe accorpate della
        // tabella di Wikipedia (docs/RICERCA.md, «Non verificato»): A·100/255 %.
        p(
            "pedale_d",
            0x49,
            "Posizione pedale acceleratore D",
            "%",
            Percento,
            1,
        ),
        p(
            "pedale_e",
            0x4A,
            "Posizione pedale acceleratore E",
            "%",
            Percento,
            1,
        ),
        p(
            "farfalla_comandata",
            0x4C,
            "Farfalla comandata dalla centralina",
            "%",
            Percento,
            1,
        ),
        p(
            "minuti_con_mil",
            0x4D,
            "Tempo con MIL accesa",
            "min",
            Parola,
            0,
        ),
        p(
            "minuti_da_cancellazione",
            0x4E,
            "Tempo dalla cancellazione DTC",
            "min",
            Parola,
            0,
        ),
        p(
            "pedale_relativo",
            0x5A,
            "Posizione relativa pedale acceleratore",
            "%",
            Percento,
            1,
        ),
        p(
            "consumo_istantaneo",
            0x5E,
            "Consumo istantaneo di carburante",
            "L/h",
            ConsumoOrario,
            2,
        ),
        p(
            "coppia_richiesta",
            0x61,
            "Coppia richiesta dal conducente",
            "%",
            Coppia,
            0,
        ),
        p("coppia_effettiva", 0x62, "Coppia effettiva", "%", Coppia, 0),
        p(
            "coppia_riferimento",
            0x63,
            "Coppia di riferimento del motore",
            "N·m",
            Parola,
            0,
        ),
        p(
            "contachilometri",
            0xA6,
            "Contachilometri",
            "km",
            Contatore32,
            1,
        ),
        p(
            CHIAVE_DISTANZA_PARZIALE,
            0x00,
            "Distanza parziale calcolata",
            "km",
            Calcolato,
            3,
        ),
    ];
    for n in 1..=8u8 {
        let s = nome_sonda(n);
        v.push(p(
            &format!("o2s{n}_tensione"),
            0x13 + n,
            &format!("{s} — tensione"),
            "V",
            O2Tensione,
            3,
        ));
        v.push(p(
            &format!("o2s{n}_stft"),
            0x13 + n,
            &format!("{s} — correzione associata"),
            "%",
            O2Correzione,
            2,
        ));
        v.push(p(
            &format!("o2s{n}_lambda"),
            0x23 + n,
            &format!("{s} — lambda"),
            "λ",
            Lambda,
            3,
        ));
        v.push(p(
            &format!("o2s{n}_tensione_wr"),
            0x23 + n,
            &format!("{s} — tensione (banda larga)"),
            "V",
            LambdaTensione,
            3,
        ));
        v.push(p(
            &format!("o2s{n}_lambda_i"),
            0x33 + n,
            &format!("{s} — lambda"),
            "λ",
            Lambda,
            3,
        ));
        v.push(p(
            &format!("o2s{n}_corrente"),
            0x33 + n,
            &format!("{s} — corrente"),
            "mA",
            LambdaCorrente,
            3,
        ));
    }
    v.sort_by_key(|x| x.pid);
    v
}

/// Tutti i parametri numerici conosciuti, ordinati per PID.
pub fn catalogo() -> &'static [Param] {
    static C: OnceLock<Vec<Param>> = OnceLock::new();
    C.get_or_init(costruisci)
}

/// Parametri letti dalla centralina con questo PID (i derivati sono esclusi).
pub fn per_pid(pid: u8) -> impl Iterator<Item = &'static Param> {
    catalogo()
        .iter()
        .filter(move |x| x.pid == pid && !x.derivato)
}

/// Parametro del catalogo o, in mancanza, parametro libero registrato.
pub fn per_key(key: &str) -> Option<&'static Param> {
    catalogo()
        .iter()
        .find(|x| x.key == key)
        .or_else(|| libero(key))
}

/// Vero se la chiave è nel catalogo dei parametri OBD (non libera).
pub fn nel_catalogo(key: &str) -> bool {
    catalogo().iter().any(|x| x.key == key)
}

fn registro_liberi() -> &'static RwLock<Vec<&'static Param>> {
    static R: OnceLock<RwLock<Vec<&'static Param>>> = OnceLock::new();
    R.get_or_init(|| RwLock::new(Vec::new()))
}

fn libero(key: &str) -> Option<&'static Param> {
    let r = registro_liberi().read().unwrap_or_else(|e| e.into_inner());
    r.iter().find(|p| p.key == key).copied()
}

/// Registra un parametro libero (fuori dal catalogo), con nome e unità presi
/// dall'intestazione di una sessione importata: da qui in poi l'elenco dei
/// sensori, i grafici, il cruscotto e le esportazioni lo mostrano con il suo
/// nome. Le chiavi del catalogo non si possono ridefinire (restituisce
/// `false`). Registrare di nuovo la stessa chiave con un altro nome o un'altra
/// unità sostituisce la definizione.
///
/// Ogni definizione nuova occupa memoria per tutta la vita del processo (i
/// parametri sono `&'static`): sono poche decine per file importato.
pub fn registra_libero(key: &str, nome: &str, unita: &str, decimali: usize) -> bool {
    if key.is_empty() || nel_catalogo(key) {
        return false;
    }
    let mut r = registro_liberi().write().unwrap_or_else(|e| e.into_inner());
    let nome = if nome.trim().is_empty() { key } else { nome };
    if let Some(i) = r.iter().position(|p| p.key == key) {
        let p = r[i];
        if p.nome == nome && p.unita == unita && p.decimali == decimali {
            return true;
        }
        r.remove(i);
    }
    let unita: &'static str = Box::leak(unita.to_string().into_boxed_str());
    r.push(Box::leak(Box::new(Param {
        key: key.to_string(),
        pid: 0x00,
        nome: nome.to_string(),
        unita,
        kind: Kind::Libero,
        decimali: decimali.min(6),
        con_segno: false,
        gruppo: Gruppo::Altro,
        derivato: false,
        libero: true,
    })));
    true
}

/// Parametri liberi registrati finora, in ordine di chiave.
pub fn liberi() -> Vec<&'static Param> {
    let r = registro_liberi().read().unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<&'static Param> = r.clone();
    v.sort_by(|a, b| a.key.cmp(&b.key));
    v
}

/// Esito della decodifica di un parametro.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decodifica {
    Valore(f64),
    /// La centralina segnala che il dato non è applicabile (es. byte FF).
    NonApplicabile,
    /// Risposta più corta del previsto.
    Corta,
}

fn parola(d: &[u8], i: usize) -> Option<f64> {
    Some(((*d.get(i)? as u32) << 8 | *d.get(i + 1)? as u32) as f64)
}

/// Applica la formula del parametro ai byte dati (dopo il PID).
pub fn decodifica(par: &Param, d: &[u8]) -> Decodifica {
    use Decodifica::*;
    let a = d.first().map(|x| *x as f64);
    let r = match par.kind {
        Kind::Percento => a.map(|a| a * 100.0 / 255.0),
        Kind::Temperatura => a.map(|a| a - 40.0),
        Kind::Correzione => a.map(|a| (a - 128.0) * 100.0 / 128.0),
        Kind::Byte => a,
        Kind::Giri => parola(d, 0).map(|w| w / 4.0),
        Kind::Anticipo => a.map(|a| a / 2.0 - 64.0),
        Kind::Maf => parola(d, 0).map(|w| w / 100.0),
        Kind::O2Tensione => a.map(|a| a / 200.0),
        Kind::O2Correzione => match d.get(1) {
            Some(0xFF) => return NonApplicabile,
            Some(b) => Some((*b as f64 - 128.0) * 100.0 / 128.0),
            None => None,
        },
        Kind::Lambda => parola(d, 0).map(|w| w * 2.0 / 65536.0),
        Kind::LambdaTensione => parola(d, 2).map(|w| w * 8.0 / 65536.0),
        Kind::LambdaCorrente => parola(d, 2).map(|w| w / 256.0 - 128.0),
        Kind::Parola => parola(d, 0),
        Kind::TemperaturaCat => parola(d, 0).map(|w| w / 10.0 - 40.0),
        Kind::Millivolt => parola(d, 0).map(|w| w / 1000.0),
        Kind::CaricoAssoluto => parola(d, 0).map(|w| w * 100.0 / 255.0),
        Kind::PressioneTripla => a.map(|a| a * 3.0),
        Kind::PressioneRail => parola(d, 0).map(|w| w * 10.0),
        Kind::PercentoB => d.get(1).map(|b| *b as f64 * 100.0 / 255.0),
        Kind::ConsumoOrario => parola(d, 0).map(|w| w / 20.0),
        Kind::Coppia => a.map(|a| a - 125.0),
        Kind::Contatore32 => match d.get(..4) {
            Some(&[a, b, c, e]) => Some(u32::from_be_bytes([a, b, c, e]) as f64 / 10.0),
            _ => None,
        },
        // Non esiste una risposta della centralina da decodificare.
        Kind::Calcolato | Kind::Libero => None,
    };
    match r {
        Some(v) => Valore(v),
        None => Corta,
    }
}

/// PID 03: stato del sistema carburante (primo sistema).
pub fn stato_carburante(d: &[u8]) -> Option<String> {
    let a = *d.first()?;
    Some(
        match a {
            0x00 => "Non riportato",
            0x01 => "Anello aperto: temperatura motore insufficiente",
            0x02 => "Anello chiuso: regolazione con sonda lambda",
            0x04 => "Anello aperto: carico motore o taglio in rilascio",
            0x08 => "Anello aperto: guasto del sistema",
            0x10 => "Anello chiuso con almeno una sonda in anomalia",
            altro => return Some(format!("Valore non standard (0x{altro:02X})")),
        }
        .to_string(),
    )
}

/// PID 51: tipo di carburante.
pub fn tipo_carburante(d: &[u8]) -> Option<String> {
    let a = *d.first()?;
    Some(
        match a {
            0 => "Non disponibile",
            1 => "Benzina",
            2 => "Metanolo",
            3 => "Etanolo",
            4 => "Diesel",
            5 => "GPL",
            6 => "Metano (CNG)",
            7 => "Propano",
            8 => "Elettrico",
            9 => "Bifuel a benzina",
            10 => "Bifuel a metanolo",
            11 => "Bifuel a etanolo",
            12 => "Bifuel a GPL",
            13 => "Bifuel a metano (CNG)",
            14 => "Bifuel a propano",
            15 => "Bifuel a elettricità",
            16 => "Bifuel a elettricità e combustione",
            17 => "Ibrido benzina",
            18 => "Ibrido etanolo",
            19 => "Ibrido diesel",
            20 => "Ibrido elettrico",
            21 => "Ibrido elettrico e combustione",
            22 => "Ibrido rigenerativo",
            23 => "Bifuel a diesel",
            altro => return Some(format!("valore {altro} non in tabella")),
        }
        .to_string(),
    )
}

/// PID 1C: norma OBD a cui il veicolo dichiara di conformarsi.
pub fn norma_obd(d: &[u8]) -> Option<String> {
    let a = *d.first()?;
    Some(
        match a {
            1 => "OBD-II (CARB)",
            2 => "OBD (EPA)",
            3 => "OBD e OBD-II",
            4 => "OBD-I",
            5 => "Non conforme OBD",
            6 => "EOBD",
            7 => "EOBD e OBD-II",
            8 => "EOBD e OBD",
            9 => "EOBD, OBD e OBD-II",
            10 => "JOBD",
            11 => "JOBD e OBD-II",
            12 => "JOBD e EOBD",
            13 => "JOBD, EOBD e OBD-II",
            altro => return Some(format!("valore {altro}")),
        }
        .to_string(),
    )
}

/// Chiave e nome del parametro testuale del PID 03.
pub const KEY_STATO_CARBURANTE: &str = "stato_carburante";
pub const NOME_STATO_CARBURANTE: &str = "Stato sistema carburante";

/// Selezione «Diagnosi P0420» calcolata sui PID realmente supportati.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresetP0420 {
    /// Segnale della sonda a monte scelto, se la centralina ne espone uno.
    pub b1s1: Option<String>,
    /// Segnale della sonda a valle scelto, se la centralina ne espone uno.
    pub b1s2: Option<String>,
    /// Parametri sempre inclusi: (chiave, supportato).
    pub base: Vec<(String, bool)>,
    /// Parametri selezionabili: (chiave, supportato).
    pub opzionali: Vec<(String, bool)>,
}

impl PresetP0420 {
    pub fn calcola(supportati: &BTreeSet<u8>) -> Self {
        let primo = |candidati: &[(&str, u8)]| {
            candidati
                .iter()
                .find(|(_, pid)| supportati.contains(pid))
                .map(|(k, _)| k.to_string())
        };
        let voce = |k: &str| {
            let pid = per_key(k).map(|x| x.pid).unwrap_or(0);
            (k.to_string(), supportati.contains(&pid))
        };
        PresetP0420 {
            // A monte: lambda se esposto, altrimenti tensione.
            b1s1: primo(&[
                ("o2s1_lambda", 0x24),
                ("o2s1_lambda_i", 0x34),
                ("o2s1_tensione", 0x14),
            ]),
            // A valle: tensione se esposta, altrimenti il segnale disponibile.
            b1s2: primo(&[
                ("o2s2_tensione", 0x15),
                ("o2s2_tensione_wr", 0x25),
                ("o2s2_lambda_i", 0x35),
            ]),
            base: [
                "stft_b1",
                "ltft_b1",
                "giri",
                "velocita",
                "temp_refrigerante",
            ]
            .iter()
            .map(|k| voce(k))
            .collect(),
            opzionali: ["carico", "map"].iter().map(|k| voce(k)).collect(),
        }
    }

    /// Chiavi delle sonde, a cui il polling dà priorità.
    pub fn sonde(&self) -> Vec<String> {
        self.b1s1.iter().chain(self.b1s2.iter()).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(key: &str, d: &[u8]) -> f64 {
        match decodifica(per_key(key).unwrap(), d) {
            Decodifica::Valore(v) => v,
            altro => panic!("{key}: {altro:?}"),
        }
    }

    #[test]
    fn parametri_liberi_registrati() {
        let k = "prova:pids_libero";
        assert!(per_key(k).is_none());
        assert!(registra_libero(k, "Pressione olio", "bar", 2));
        let p = per_key(k).unwrap();
        assert!(p.libero && !p.derivato && !nel_catalogo(k));
        assert_eq!(
            (p.nome.as_str(), p.unita, p.decimali),
            ("Pressione olio", "bar", 2)
        );
        assert_eq!(decodifica(p, &[1, 2]), Decodifica::Corta);
        assert!(liberi().iter().any(|x| x.key == k));
        // Mai nella decodifica per PID, e il catalogo non si ridefinisce.
        assert!(per_pid(0x00).all(|x| x.key != k));
        assert!(!registra_libero("giri", "Altro nome", "x", 0));
        assert_eq!(per_key("giri").unwrap().nome, "Giri motore");
        // Una nuova definizione sostituisce la precedente.
        assert!(registra_libero(k, "Pressione olio", "psi", 1));
        assert_eq!(per_key(k).unwrap().unita, "psi");
        assert_eq!(liberi().iter().filter(|x| x.key == k).count(), 1);
    }

    #[test]
    fn fermo_immagine_di_riferimento() {
        // Valori del caso demo: 2.128 giri/min, 69 km/h, 83 °C, +0,78 %, +8,59 %.
        assert_eq!(val("giri", &[0x21, 0x40]), 2128.0);
        assert_eq!(val("velocita", &[0x45]), 69.0);
        assert_eq!(val("temp_refrigerante", &[0x7B]), 83.0);
        assert!((val("stft_b1", &[0x81]) - 0.78125).abs() < 1e-9);
        assert!((val("ltft_b1", &[0x8B]) - 8.59375).abs() < 1e-9);
    }

    #[test]
    fn sonde() {
        assert!((val("o2s2_tensione", &[0x5A, 0xFF]) - 0.45).abs() < 1e-9);
        assert_eq!(
            decodifica(per_key("o2s2_stft").unwrap(), &[0x5A, 0xFF]),
            Decodifica::NonApplicabile
        );
        assert!((val("o2s1_lambda", &[0x80, 0x00, 0x40, 0x00]) - 1.0).abs() < 1e-9);
        assert!((val("o2s1_tensione_wr", &[0x80, 0x00, 0x40, 0x00]) - 2.0).abs() < 1e-9);
        assert!((val("o2s1_corrente", &[0x80, 0x00, 0x80, 0x00])).abs() < 1e-9);
        assert_eq!(
            decodifica(per_key("o2s1_lambda").unwrap(), &[0x80]),
            Decodifica::Corta
        );
    }

    #[test]
    fn altri() {
        assert!((val("carico", &[0xFF]) - 100.0).abs() < 1e-9);
        assert_eq!(val("map", &[0x2D]), 45.0);
        assert_eq!(val("anticipo", &[0x90]), 8.0);
        assert!((val("tensione_modulo", &[0x36, 0xB0]) - 14.0).abs() < 1e-9);
        assert_eq!(
            stato_carburante(&[0x02, 0x00]).unwrap(),
            "Anello chiuso: regolazione con sonda lambda"
        );
    }

    #[test]
    fn chiavi_uniche() {
        let mut viste = BTreeSet::new();
        for x in catalogo() {
            assert!(viste.insert(x.key.clone()), "chiave duplicata {}", x.key);
        }
    }

    #[test]
    fn nuove_formule_pressioni_e_conteggi() {
        // 0A: 3·A kPa.
        assert_eq!(val("pressione_carburante", &[0x64]), 300.0);
        // 23: 10·(256A + B) kPa.
        assert_eq!(val("pressione_rail", &[0x01, 0x2C]), 3000.0);
        // 30: A, conteggio.
        assert_eq!(val("avviamenti_da_cancellazione", &[0x2A]), 42.0);
        // 4D e 4E: 256A + B minuti.
        assert_eq!(val("minuti_con_mil", &[0x01, 0x02]), 258.0);
        assert_eq!(val("minuti_da_cancellazione", &[0x00, 0x3C]), 60.0);
        assert_eq!(
            decodifica(per_key("pressione_rail").unwrap(), &[0x01]),
            Decodifica::Corta
        );
    }

    #[test]
    fn nuove_formule_farfalla_e_pedale() {
        assert!((val("farfalla_relativa", &[0xFF]) - 100.0).abs() < 1e-9);
        assert!((val("farfalla_relativa", &[0x33]) - 20.0).abs() < 1e-9);
        // 47: usa il secondo byte.
        assert!((val("farfalla_assoluta_b", &[0xFF, 0x80]) - 50.196078).abs() < 1e-5);
        assert_eq!(
            decodifica(per_key("farfalla_assoluta_b").unwrap(), &[0x80]),
            Decodifica::Corta
        );
        for k in [
            "pedale_d",
            "pedale_e",
            "farfalla_comandata",
            "pedale_relativo",
        ] {
            assert!((val(k, &[0x66]) - 40.0).abs() < 1e-9, "{k}");
        }
    }

    #[test]
    fn nuove_formule_consumo_coppia_contachilometri() {
        // 5E: (256A + B)/20 L/h.
        assert!((val("consumo_istantaneo", &[0x00, 0x32]) - 2.5).abs() < 1e-9);
        // 61 e 62: A − 125 %.
        assert_eq!(val("coppia_richiesta", &[0x7D]), 0.0);
        assert_eq!(val("coppia_effettiva", &[0xFA]), 125.0);
        assert_eq!(val("coppia_effettiva", &[0x00]), -125.0);
        // 63: 256A + B N·m.
        assert_eq!(val("coppia_riferimento", &[0x01, 0xF4]), 500.0);
        // A6: (A·2^24 + B·2^16 + C·2^8 + D)/10 km.
        assert!((val("contachilometri", &[0x00, 0x01, 0x86, 0xA3]) - 10_000.3).abs() < 1e-6);
        assert!((val("contachilometri", &[0x01, 0x00, 0x00, 0x00]) - 1_677_721.6).abs() < 1e-6);
        assert_eq!(
            decodifica(per_key("contachilometri").unwrap(), &[0x00, 0x01, 0x86]),
            Decodifica::Corta
        );
    }

    #[test]
    fn derivati_esclusi_dalla_decodifica_per_pid() {
        let d = per_key(CHIAVE_DISTANZA_PARZIALE).unwrap();
        assert!(d.derivato);
        assert_eq!(d.unita, "km");
        assert_eq!(d.decimali, 3);
        assert_eq!(
            sorgente_derivato(CHIAVE_DISTANZA_PARZIALE),
            Some("velocita")
        );
        // Il PID 00 non produce parametri e nessun altro parametro è derivato.
        assert_eq!(per_pid(0x00).count(), 0);
        assert_eq!(catalogo().iter().filter(|x| x.derivato).count(), 1);
        assert!(per_key("velocita").is_some_and(|x| !x.derivato));
    }

    #[test]
    fn ogni_parametro_ha_un_gruppo_coerente() {
        assert_eq!(
            per_key("o2s2_tensione").unwrap().gruppo,
            Gruppo::SondeLambda
        );
        assert_eq!(per_key("giri").unwrap().gruppo, Gruppo::Motore);
        assert_eq!(
            per_key("stft_b1").unwrap().gruppo,
            Gruppo::CarburanteEMiscela
        );
        assert_eq!(
            per_key("temp_refrigerante").unwrap().gruppo,
            Gruppo::TemperatureEPressioni
        );
        assert_eq!(
            per_key("map").unwrap().gruppo,
            Gruppo::TemperatureEPressioni
        );
        assert_eq!(
            per_key("contachilometri").unwrap().gruppo,
            Gruppo::DistanzeETempi
        );
        assert_eq!(
            per_key(CHIAVE_DISTANZA_PARZIALE).unwrap().gruppo,
            Gruppo::DistanzeETempi
        );
        assert_eq!(per_key("tensione_modulo").unwrap().gruppo, Gruppo::Altro);
    }

    #[test]
    fn tipo_carburante_e_norma_obd() {
        assert_eq!(tipo_carburante(&[0x01]).unwrap(), "Benzina");
        assert_eq!(tipo_carburante(&[0x04]).unwrap(), "Diesel");
        assert_eq!(tipo_carburante(&[0x06]).unwrap(), "Metano (CNG)");
        assert_eq!(
            tipo_carburante(&[0x15]).unwrap(),
            "Ibrido elettrico e combustione"
        );
        assert_eq!(tipo_carburante(&[0x17]).unwrap(), "Bifuel a diesel");
        assert_eq!(
            tipo_carburante(&[0x63]).unwrap(),
            "valore 99 non in tabella"
        );
        assert_eq!(tipo_carburante(&[]), None);
        assert_eq!(norma_obd(&[0x06]).unwrap(), "EOBD");
        assert_eq!(norma_obd(&[0x01]).unwrap(), "OBD-II (CARB)");
        assert_eq!(norma_obd(&[0x0D]).unwrap(), "JOBD, EOBD e OBD-II");
        assert_eq!(norma_obd(&[0x63]).unwrap(), "valore 99");
        assert_eq!(norma_obd(&[]), None);
    }

    #[test]
    fn preset_banda_larga_a_monte() {
        let s: BTreeSet<u8> = [0x05, 0x06, 0x07, 0x0B, 0x0C, 0x0D, 0x15, 0x24]
            .into_iter()
            .collect();
        let pr = PresetP0420::calcola(&s);
        assert_eq!(pr.b1s1.as_deref(), Some("o2s1_lambda"));
        assert_eq!(pr.b1s2.as_deref(), Some("o2s2_tensione"));
        assert!(pr.base.iter().all(|(_, ok)| *ok));
        assert_eq!(
            pr.opzionali,
            vec![("carico".to_string(), false), ("map".to_string(), true)]
        );
    }

    #[test]
    fn preset_banda_stretta_e_assenze() {
        let s: BTreeSet<u8> = [0x0C, 0x14].into_iter().collect();
        let pr = PresetP0420::calcola(&s);
        assert_eq!(pr.b1s1.as_deref(), Some("o2s1_tensione"));
        assert_eq!(pr.b1s2, None);
        assert!(!pr.base.iter().find(|(k, _)| k == "stft_b1").unwrap().1);
    }
}
