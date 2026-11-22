//! Importazione ed esportazione delle sessioni (logica pura, senza
//! interfaccia). Formati descritti in `docs/FORMATO_CSV.md`.
//!
//! Esportazione:
//! - sessione `.obdio.jsonl` (copia del file);
//! - CSV OBDio «lungo» (una riga per campione, vedi [`recording::csv`]);
//! - CSV «largo» in stile Torque: una riga per istante di campionamento,
//!   una colonna `Nome(unità)` per parametro;
//! - diagnosi in JSON (mai il VIN).
//!
//! Importazione, con il formato riconosciuto dal contenuto: sessione
//! `.obdio.jsonl`, CSV OBDio (andata e ritorno esatti), CSV in stile Torque.
//!
//! In nessun caso un dato assente diventa zero: una cella `-` di Torque è un
//! campione «risposta mancante» senza valore, una cella vuota non è un campione.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{NaiveDateTime, TimeZone};

use crate::model::{Marker, Origine, Sample, Stato, adesso_iso};
use crate::obd::pids;
use crate::recording::{self, Importata, Intestazione, MetaParametro, SessionData};

// ---------------------------------------------------------------------------
// Formati
// ---------------------------------------------------------------------------

/// Formati di esportazione.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatoEsportazione {
    Sessione,
    CsvObdio,
    CsvTorque,
    DiagnosiJson,
}

impl FormatoEsportazione {
    pub const TUTTI: [FormatoEsportazione; 4] = [
        FormatoEsportazione::Sessione,
        FormatoEsportazione::CsvObdio,
        FormatoEsportazione::CsvTorque,
        FormatoEsportazione::DiagnosiJson,
    ];

    pub fn nome(self) -> &'static str {
        match self {
            FormatoEsportazione::Sessione => "Sessione OBDio (.obdio.jsonl)",
            FormatoEsportazione::CsvObdio => "CSV OBDio (una riga per campione)",
            FormatoEsportazione::CsvTorque => "CSV in stile Torque (una colonna per parametro)",
            FormatoEsportazione::DiagnosiJson => "Diagnosi in JSON",
        }
    }

    /// Estensione aggiunta se il nome scelto non ne ha una.
    pub fn estensione(self) -> &'static str {
        match self {
            FormatoEsportazione::Sessione => recording::ESTENSIONE,
            FormatoEsportazione::CsvObdio | FormatoEsportazione::CsvTorque => ".csv",
            FormatoEsportazione::DiagnosiJson => ".json",
        }
    }
}

/// Formati riconosciuti all'importazione.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatoImportazione {
    Sessione,
    CsvObdio,
    CsvTorque,
}

impl FormatoImportazione {
    pub fn nome(self) -> &'static str {
        match self {
            FormatoImportazione::Sessione => "Sessione OBDio",
            FormatoImportazione::CsvObdio => "CSV OBDio",
            FormatoImportazione::CsvTorque => "CSV in stile Torque",
        }
    }
}

// ---------------------------------------------------------------------------
// Esportazione
// ---------------------------------------------------------------------------

/// Base del nome della sessione (senza `.obdio.jsonl`).
fn base_sessione(s: &SessionData) -> String {
    let nome = s
        .percorso
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let base = nome.strip_suffix(recording::ESTENSIONE).unwrap_or(&nome);
    if base.is_empty() {
        "sessione".into()
    } else {
        base.to_string()
    }
}

/// Nome di file proposto per esportare la sessione nel formato.
pub fn nome_proposto(s: &SessionData, f: FormatoEsportazione) -> String {
    let base = base_sessione(s);
    match f {
        FormatoEsportazione::Sessione => format!("{base}{}", recording::ESTENSIONE),
        FormatoEsportazione::CsvObdio => format!("{base}.csv"),
        FormatoEsportazione::CsvTorque => format!("{base}_torque.csv"),
        FormatoEsportazione::DiagnosiJson => format!("{base}_diagnosi.json"),
    }
}

/// Aggiunge l'estensione del formato se il percorso non ne ha una.
pub fn con_estensione(p: &Path, f: FormatoEsportazione) -> PathBuf {
    if p.extension().is_some() {
        return p.to_path_buf();
    }
    let mut s = p.as_os_str().to_os_string();
    s.push(f.estensione());
    PathBuf::from(s)
}

/// Diagnosi della sessione in JSON leggibile. Il VIN non fa parte del
/// referto serializzato.
pub fn diagnosi_json(s: &SessionData) -> Result<String, String> {
    let d = s
        .diagnosi
        .as_ref()
        .ok_or("la sessione non contiene una diagnosi")?;
    serde_json::to_string_pretty(d).map_err(|e| e.to_string())
}

/// Scrive la sessione nel formato scelto e restituisce il percorso scritto.
/// Un file esistente viene sostituito, salvo che sia la sessione stessa.
pub fn esporta(
    s: &SessionData,
    f: FormatoEsportazione,
    destinazione: &Path,
) -> Result<PathBuf, String> {
    let dest = con_estensione(destinazione, f);
    if let (Ok(a), Ok(b)) = (dest.canonicalize(), s.percorso.canonicalize())
        && a == b
    {
        return Err(
            "il file di destinazione è la sessione stessa: scegli un altro nome o un'altra \
             cartella"
                .into(),
        );
    }
    if let Some(cartella) = dest.parent()
        && !cartella.as_os_str().is_empty()
        && !cartella.is_dir()
    {
        return Err(format!("la cartella «{}» non esiste", cartella.display()));
    }
    let scrivi = |testo: &str| {
        std::fs::write(&dest, testo)
            .map_err(|e| format!("impossibile scrivere «{}»: {e}", dest.display()))
    };
    match f {
        FormatoEsportazione::Sessione if s.percorso.is_file() => {
            std::fs::copy(&s.percorso, &dest)
                .map_err(|e| format!("impossibile copiare la sessione: {e}"))?;
        }
        FormatoEsportazione::Sessione => scrivi(&recording::testo_sessione(s)?)?,
        FormatoEsportazione::CsvObdio => scrivi(&recording::csv(s))?,
        FormatoEsportazione::CsvTorque => scrivi(&csv_torque(s))?,
        FormatoEsportazione::DiagnosiJson => scrivi(&diagnosi_json(s)?)?,
    }
    Ok(dest)
}

// ---------------------------------------------------------------------------
// Torque: nomi delle colonne e unità
// ---------------------------------------------------------------------------

/// Colonne di Torque riconosciute: nome (senza unità), chiave OBDio, unità
/// scritta da Torque nelle impostazioni metriche.
pub const COLONNE_TORQUE: [(&str, &str, &str); 16] = [
    ("Engine RPM", "giri", "rpm"),
    ("Speed (OBD)", "velocita", "km/h"),
    ("Engine Coolant Temperature", "temp_refrigerante", "°C"),
    ("Engine Load", "carico", "%"),
    ("Intake Manifold Pressure", "map", "kPa"),
    ("Intake Air Temperature", "temp_aspirazione", "°C"),
    ("Throttle Position(Manifold)", "farfalla", "%"),
    ("Fuel Trim Bank 1 Short Term", "stft_b1", "%"),
    ("Fuel Trim Bank 1 Long Term", "ltft_b1", "%"),
    ("Fuel Trim Bank 2 Short Term", "stft_b2", "%"),
    ("Fuel Trim Bank 2 Long Term", "ltft_b2", "%"),
    ("O2 Volts Bank 1 sensor 1", "o2s1_tensione", "V"),
    ("O2 Volts Bank 1 sensor 2", "o2s2_tensione", "V"),
    ("Mass Air Flow Rate", "maf", "g/s"),
    ("Timing Advance", "anticipo", "°"),
    ("Voltage (Control Module)", "tensione_modulo", "V"),
];

/// Prefisso delle chiavi dei parametri liberi importati da Torque.
pub const PREFISSO_LIBERI: &str = "torque:";

const MESI: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Nome e unità di un'intestazione `Nome(unità)`: l'unità è l'**ultimo**
/// gruppo fra parentesi (così `Throttle Position(Manifold)(%)` ha nome
/// `Throttle Position(Manifold)` e unità `%`). Senza parentesi finale l'unità
/// è vuota.
pub fn dividi_intestazione(h: &str) -> (String, String) {
    let h = h.trim();
    if !h.ends_with(')') {
        return (h.to_string(), String::new());
    }
    let mut profondita = 0usize;
    for (i, c) in h.char_indices().rev() {
        match c {
            ')' => profondita += 1,
            '(' => {
                profondita -= 1;
                if profondita == 0 {
                    let nome = h[..i].trim();
                    if nome.is_empty() {
                        break;
                    }
                    return (nome.to_string(), h[i + 1..h.len() - 1].trim().to_string());
                }
            }
            _ => {}
        }
    }
    (h.to_string(), String::new())
}

/// Nome confrontabile: minuscolo, spazi singoli.
fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Chiave stabile di un parametro libero: `torque:` più il nome in
/// minuscolo, con `_` al posto di tutto ciò che non è lettera o cifra.
pub fn chiave_libera(nome: &str) -> String {
    let mut out = String::new();
    for c in nome.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let pulita = out.trim_matches('_');
    format!(
        "{PREFISSO_LIBERI}{}",
        if pulita.is_empty() { "colonna" } else { pulita }
    )
}

/// Più parametri del catalogo hanno questo stesso nome (per esempio le due
/// lambda della sonda B1S1): nell'esportazione si aggiunge `[chiave]`.
fn nome_ambiguo(key: &str) -> bool {
    let Some(p) = pids::per_key(key) else {
        return false;
    };
    pids::catalogo().iter().filter(|x| x.nome == p.nome).count() > 1
}

/// Conversione di unità all'importazione.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Conversione {
    Nessuna,
    FahrenheitCelsius,
    MphKmh,
    PsiKpa,
    BarKpa,
}

impl Conversione {
    fn applica(self, v: f64) -> f64 {
        match self {
            Conversione::Nessuna => v,
            Conversione::FahrenheitCelsius => (v - 32.0) * 5.0 / 9.0,
            Conversione::MphKmh => v * 1.609_344,
            Conversione::PsiKpa => v * 6.894_757_293_168,
            Conversione::BarKpa => v * 100.0,
        }
    }
}

/// Come portare un valore dall'unità del file a quella di OBDio; `None` se
/// l'unità non è riconosciuta (la colonna resta allora un parametro libero).
fn conversione(unita_file: &str, unita_obdio: &str) -> Option<Conversione> {
    let f = norm(unita_file).replace(['°', 'º'], "").replace("deg", "");
    let f = f.trim();
    let uguale = f.is_empty() || f == norm(unita_obdio).replace('°', "").trim();
    use Conversione::*;
    match unita_obdio {
        "°C" => match f {
            "c" | "" => Some(Nessuna),
            "f" => Some(FahrenheitCelsius),
            _ => None,
        },
        "km/h" => match f {
            "km/h" | "kmh" | "kph" | "" => Some(Nessuna),
            "mph" => Some(MphKmh),
            _ => None,
        },
        "kPa" => match f {
            "kpa" | "" => Some(Nessuna),
            "psi" => Some(PsiKpa),
            "bar" => Some(BarKpa),
            _ => None,
        },
        "giri/min" => matches!(f, "rpm" | "giri/min" | "").then_some(Nessuna),
        "°" => matches!(f, "" | "btdc").then_some(Nessuna),
        _ => uguale.then_some(Nessuna),
    }
}

// ---------------------------------------------------------------------------
// Torque: esportazione
// ---------------------------------------------------------------------------

/// `Device Time` di Torque: `21-Nov-2026 19:37:41.512`.
pub fn ora_torque_testo(d: &NaiveDateTime) -> String {
    use chrono::{Datelike, Timelike};
    format!(
        "{:02}-{}-{:04} {:02}:{:02}:{:02}.{:03}",
        d.day(),
        MESI[d.month0() as usize],
        d.year(),
        d.hour(),
        d.minute(),
        d.second(),
        d.nanosecond() / 1_000_000 % 1000
    )
}

/// Intestazione di colonna per un parametro della sessione.
fn intestazione_torque(s: &SessionData, key: &str, unita_campione: &str) -> String {
    if let Some((nome, _, unita)) = COLONNE_TORQUE.iter().find(|(_, k, _)| *k == key) {
        return format!("{nome}({unita})");
    }
    if pids::nel_catalogo(key) {
        let p = pids::per_key(key).expect("nel catalogo");
        let nome = if nome_ambiguo(key) {
            format!("{} [{key}]", p.nome)
        } else {
            p.nome.clone()
        };
        return format!("{nome}({})", p.unita);
    }
    let meta = s.intestazione.parametri.get(key);
    let nome = meta
        .map(|m| m.nome.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| recording::nome_parametro(s, key));
    let unita = meta
        .map(|m| m.unita.clone())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| unita_campione.to_string());
    format!("{nome}({unita})")
}

/// CSV «largo» in stile Torque.
///
/// - Colonne: `GPS Time` (vuota: OBDio non ha il GPS), `Device Time`, poi una
///   per parametro nell'ordine di prima comparsa, `Nome(unità)`; per i
///   parametri della tabella [`COLONNE_TORQUE`] i nomi di Torque.
/// - Una riga per **istante di campionamento**: i campioni con lo stesso `t`
///   (tutte le grandezze di una stessa risposta) stanno sulla stessa riga.
/// - Cella con un numero: letto in quell'istante. `-`: letto in
///   quell'istante ma senza valore (mai zero). Vuota: non letto in
///   quell'istante (nessun valore riportato da righe precedenti).
/// - Di un parametro letto da più centraline nello stesso istante resta la
///   prima risposta. I marcatori non sono esportati (Torque non li prevede).
/// - Separatore virgola, punto decimale, fine riga LF, UTF-8 senza BOM.
pub fn csv_torque(s: &SessionData) -> String {
    let mut ordinati: Vec<&Sample> = s.campioni.iter().filter(|c| c.t.is_finite()).collect();
    ordinati.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut chiavi: Vec<&str> = Vec::new();
    let mut unita: BTreeMap<&str, &str> = BTreeMap::new();
    for c in &ordinati {
        if !chiavi.contains(&c.key.as_str()) {
            chiavi.push(&c.key);
            unita.insert(&c.key, &c.unita);
        }
    }
    let indice: BTreeMap<&str, usize> = chiavi.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let inizio = chrono::DateTime::parse_from_rfc3339(&s.intestazione.creata)
        .map(|d| d.naive_local())
        .unwrap_or_default();

    let mut out = String::from("GPS Time,Device Time");
    for k in &chiavi {
        out.push(',');
        out.push_str(&recording::campo(&intestazione_torque(s, k, unita[k])));
    }
    out.push('\n');
    let mut i = 0;
    while i < ordinati.len() {
        let t = ordinati[i].t;
        let mut j = i;
        let mut celle = vec![String::new(); chiavi.len()];
        let mut scritte = vec![false; chiavi.len()];
        while j < ordinati.len() && ordinati[j].t == t {
            let c = ordinati[j];
            let n = indice[c.key.as_str()];
            if !scritte[n] {
                scritte[n] = true;
                celle[n] = match (c.stato, c.valore) {
                    (Stato::Ok, Some(v)) if v.is_finite() => {
                        recording::numero_csv(v, recording::decimali_file(s, &c.key))
                    }
                    _ => "-".to_string(),
                };
            }
            j += 1;
        }
        let quando = chrono::DateTime::parse_from_rfc3339(&ordinati[i].ts)
            .map(|d| d.naive_local())
            .unwrap_or_else(|_| {
                chrono::TimeDelta::try_milliseconds((t * 1000.0) as i64)
                    .and_then(|d| inizio.checked_add_signed(d))
                    .unwrap_or(inizio)
            });
        out.push(',');
        out.push_str(&ora_torque_testo(&quando));
        for c in celle {
            out.push(',');
            out.push_str(&c);
        }
        out.push('\n');
        i = j;
    }
    out
}

// ---------------------------------------------------------------------------
// Lettura dei file
// ---------------------------------------------------------------------------

/// Testo del file: UTF-8 (con o senza BOM), altrimenti Latin-1 come i file
/// scritti da programmi Windows meno recenti. Il secondo valore dice se è
/// servito il ripiego.
fn decodifica(byte: &[u8]) -> (String, bool) {
    let byte = byte.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(byte);
    match std::str::from_utf8(byte) {
        Ok(t) => (t.to_string(), false),
        Err(_) => (byte.iter().map(|b| *b as char).collect(), true),
    }
}

/// Separatore più frequente fuori dalle virgolette nella riga d'intestazione.
fn separatore(riga: &str) -> char {
    let mut conta = [0usize; 3];
    let mut virgolette = false;
    for c in riga.chars() {
        match c {
            '"' => virgolette = !virgolette,
            ',' if !virgolette => conta[0] += 1,
            ';' if !virgolette => conta[1] += 1,
            '\t' if !virgolette => conta[2] += 1,
            _ => {}
        }
    }
    let max = *conta.iter().max().unwrap_or(&0);
    if max == 0 || conta[0] == max {
        ','
    } else if conta[1] == max {
        ';'
    } else {
        '\t'
    }
}

/// Righe di un CSV (RFC 4180: virgolette doppie, virgolette raddoppiate,
/// a capo dentro le virgolette). CR LF e LF sono entrambi accettati; una
/// virgoletta mai chiusa termina il campo alla fine del file.
fn righe_csv(testo: &str, sep: char) -> Vec<Vec<String>> {
    let mut righe = Vec::new();
    let mut riga: Vec<String> = Vec::new();
    let mut campo = String::new();
    let mut virgolette = false;
    let mut c = testo.chars().peekable();
    while let Some(ch) = c.next() {
        if virgolette {
            if ch == '"' {
                if c.peek() == Some(&'"') {
                    campo.push('"');
                    c.next();
                } else {
                    virgolette = false;
                }
            } else {
                campo.push(ch);
            }
            continue;
        }
        match ch {
            '"' if campo.trim().is_empty() => {
                campo.clear();
                virgolette = true;
            }
            '\r' if c.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                riga.push(std::mem::take(&mut campo));
                righe.push(std::mem::take(&mut riga));
            }
            x if x == sep => riga.push(std::mem::take(&mut campo)),
            x => campo.push(x),
        }
    }
    if !campo.is_empty() || !riga.is_empty() {
        riga.push(campo);
        righe.push(riga);
    }
    righe
}

/// Numero di una cella. Con `virgola_decimale` (CSV con `;` di Excel in
/// italiano) la virgola è il separatore decimale e il punto delle migliaia.
fn numero(cella: &str, virgola_decimale: bool) -> Option<f64> {
    let c = cella.trim();
    if c.is_empty() {
        return None;
    }
    let testo = if virgola_decimale && c.contains(',') {
        c.replace('.', "").replace(',', ".")
    } else {
        c.to_string()
    };
    testo.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn vuota(r: &[String]) -> bool {
    r.iter().all(|c| c.trim().is_empty())
}

/// Ora di Torque (`21-Nov-2026 19:37:41.512`), anche come la riscrive Excel
/// in italiano (`21-nov-2026 19:37:41,512`, `21/11/2026 19:37:41`).
pub fn leggi_ora_torque(s: &str) -> Option<NaiveDateTime> {
    let mut t = s.trim().replace(',', ".");
    for (it, en) in [
        ("gen", "Jan"),
        ("mag", "May"),
        ("giu", "Jun"),
        ("lug", "Jul"),
        ("ago", "Aug"),
        ("set", "Sep"),
        ("ott", "Oct"),
        ("dic", "Dec"),
    ] {
        t = t.replace(&format!("-{it}-"), &format!("-{en}-"));
    }
    [
        "%d-%b-%Y %H:%M:%S%.f",
        "%d-%b-%Y %H:%M:%S",
        "%d/%m/%Y %H:%M:%S%.f",
        "%d/%m/%Y %H:%M:%S",
        "%d/%m/%Y %H:%M",
        "%Y-%m-%d %H:%M:%S%.f",
    ]
    .iter()
    .find_map(|f| NaiveDateTime::parse_from_str(&t, f).ok())
}

/// ISO 8601 con il fuso locale per un'ora senza fuso.
fn iso_locale(d: &NaiveDateTime) -> String {
    match chrono::Local.from_local_datetime(d).earliest() {
        Some(l) => l.to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        None => d
            .and_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
    }
}

// ---------------------------------------------------------------------------
// Importazione
// ---------------------------------------------------------------------------

/// Colonna riconosciuta all'importazione.
#[derive(Clone, Debug, PartialEq)]
pub struct ColonnaRiconosciuta {
    /// Intestazione della colonna nel file.
    pub colonna: String,
    pub chiave: String,
    /// Conversione applicata, in chiaro (per esempio «mph → km/h»).
    pub conversione: Option<String>,
}

/// Colonna conservata come parametro libero.
#[derive(Clone, Debug, PartialEq)]
pub struct ColonnaLibera {
    pub colonna: String,
    pub chiave: String,
    pub unita: String,
}

/// Riepilogo mostrato prima di salvare una sessione importata.
#[derive(Clone, Debug, PartialEq)]
pub struct Riepilogo {
    pub formato: FormatoImportazione,
    pub file: String,
    pub campioni: usize,
    pub marcatori: usize,
    pub durata_s: f64,
    /// Parametri riconosciuti (chiavi del catalogo).
    pub riconosciuti: Vec<ColonnaRiconosciuta>,
    /// Parametri conservati come liberi (fuori dal catalogo).
    pub liberi: Vec<ColonnaLibera>,
    /// Righe saltate: ora illeggibile, stato sconosciuto, riga incompleta.
    pub righe_scartate: usize,
    /// Righe con un numero di colonne diverso dall'intestazione (importate
    /// per quanto possibile).
    pub righe_irregolari: usize,
    /// Intestazioni ripetute a metà file.
    pub intestazioni_ripetute: usize,
    /// Celle con testo non numerico (importate come «errore di comunicazione»).
    pub valori_non_numerici: usize,
    pub avvisi: Vec<String>,
}

impl Riepilogo {
    fn new(formato: FormatoImportazione, file: &str) -> Self {
        Riepilogo {
            formato,
            file: file.to_string(),
            campioni: 0,
            marcatori: 0,
            durata_s: 0.0,
            riconosciuti: Vec::new(),
            liberi: Vec::new(),
            righe_scartate: 0,
            righe_irregolari: 0,
            intestazioni_ripetute: 0,
            valori_non_numerici: 0,
            avvisi: Vec::new(),
        }
    }
}

/// Sessione letta da un file, pronta da salvare come sessione OBDio.
#[derive(Clone, Debug, PartialEq)]
pub struct Importazione {
    pub sessione: SessionData,
    pub riepilogo: Riepilogo,
}

/// Formato riconosciuto dal contenuto.
pub fn rileva_formato(testo: &str) -> Option<FormatoImportazione> {
    let testo = testo.trim_start_matches('\u{feff}');
    let prima = testo.lines().find(|r| !r.trim().is_empty())?;
    if prima.trim_start().starts_with('{') {
        return Some(FormatoImportazione::Sessione);
    }
    let sep = separatore(prima);
    let celle: Vec<String> = righe_csv(prima, sep)
        .into_iter()
        .next()
        .unwrap_or_default()
        .iter()
        .map(|c| norm(c))
        .collect();
    let obdio: Vec<&str> = recording::INTESTAZIONE_CSV.split(',').collect();
    if ["tempo_s", "parametro", "valore", "stato"]
        .iter()
        .all(|c| celle.iter().any(|x| x == c))
        && celle.len() >= obdio.len() - 1
    {
        return Some(FormatoImportazione::CsvObdio);
    }
    if celle.iter().any(|c| c == "device time" || c == "gps time") {
        return Some(FormatoImportazione::CsvTorque);
    }
    None
}

/// Legge un file da importare (formato dal contenuto).
pub fn importa_file(percorso: &Path) -> Result<Importazione, String> {
    let byte = std::fs::read(percorso)
        .map_err(|e| format!("impossibile leggere «{}»: {e}", percorso.display()))?;
    let nome = percorso
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    importa_byte(&byte, &nome)
}

/// Interpreta il contenuto di un file da importare. Non va mai in panico:
/// un file illeggibile dà un errore che dice cosa non va.
pub fn importa_byte(byte: &[u8], nome_file: &str) -> Result<Importazione, String> {
    let (testo, latin1) = decodifica(byte);
    if testo.trim().is_empty() {
        return Err("il file è vuoto".into());
    }
    let formato = rileva_formato(&testo).ok_or(
        "formato non riconosciuto: il file non è una sessione OBDio (.obdio.jsonl), né un CSV \
         esportato da OBDio, né un CSV in stile Torque (manca la colonna «Device Time»)",
    )?;
    let mut imp = match formato {
        FormatoImportazione::Sessione => importa_sessione(&testo, nome_file)?,
        FormatoImportazione::CsvObdio => importa_csv_obdio(&testo, nome_file)?,
        FormatoImportazione::CsvTorque => importa_torque(&testo, nome_file)?,
    };
    if latin1 {
        imp.riepilogo.avvisi.insert(
            0,
            "Il file non è in UTF-8: è stato letto come testo Latin-1 (Windows).".into(),
        );
    }
    let s = &mut imp.sessione;
    s.intestazione.importata = Some(Importata {
        file: nome_file.to_string(),
        formato: formato.nome().to_string(),
        il: adesso_iso(),
    });
    recording::registra_parametri(&s.intestazione);
    let r = &mut imp.riepilogo;
    r.campioni = s.campioni.len();
    r.marcatori = s.marcatori.len();
    r.durata_s = s
        .campioni
        .iter()
        .map(|c| c.t)
        .chain(s.marcatori.iter().map(|m| m.t))
        .filter(|t| t.is_finite())
        .fold(0.0, f64::max);
    Ok(imp)
}

/// Colonne riconosciute e libere di una sessione (formati OBDio).
fn classifica_chiavi(s: &SessionData, r: &mut Riepilogo) {
    let mut viste: BTreeSet<&str> = BTreeSet::new();
    for c in &s.campioni {
        if !viste.insert(&c.key) {
            continue;
        }
        if pids::nel_catalogo(&c.key) {
            r.riconosciuti.push(ColonnaRiconosciuta {
                colonna: c.key.clone(),
                chiave: c.key.clone(),
                conversione: None,
            });
        } else {
            r.liberi.push(ColonnaLibera {
                colonna: c.key.clone(),
                chiave: c.key.clone(),
                unita: c.unita.clone(),
            });
        }
    }
}

fn importa_sessione(testo: &str, nome_file: &str) -> Result<Importazione, String> {
    let s = recording::leggi(testo)?;
    let mut r = Riepilogo::new(FormatoImportazione::Sessione, nome_file);
    r.righe_scartate = s.righe_scartate;
    if s.righe_scartate > 0 {
        r.avvisi.push(format!(
            "{} righe illeggibili ignorate (file interrotto?).",
            s.righe_scartate
        ));
    }
    classifica_chiavi(&s, &mut r);
    Ok(Importazione {
        sessione: s,
        riepilogo: r,
    })
}

/// CSV OBDio (vedi `docs/FORMATO_CSV.md`): andata e ritorno esatti di
/// campioni, marcatori, stati e origine. La diagnosi non fa parte del CSV.
fn importa_csv_obdio(testo: &str, nome_file: &str) -> Result<Importazione, String> {
    let testo = testo.trim_start_matches('\u{feff}');
    let prima = testo.lines().find(|r| !r.trim().is_empty()).unwrap_or("");
    let sep = separatore(prima);
    let virgola = sep == ';';
    let mut righe = righe_csv(testo, sep).into_iter().filter(|r| !vuota(r));
    let intestazione: Vec<String> = righe
        .next()
        .unwrap_or_default()
        .iter()
        .map(|c| norm(c))
        .collect();
    let col = |nome: &str| intestazione.iter().position(|c| c == nome);
    let (Some(i_t), Some(i_par), Some(i_val), Some(i_stato)) = (
        col("tempo_s"),
        col("parametro"),
        col("valore"),
        col("stato"),
    ) else {
        return Err("intestazione del CSV OBDio incompleta".into());
    };
    let (i_ts, i_ecu, i_desc, i_unita, i_orig) = (
        col("timestamp"),
        col("ecu"),
        col("descrizione"),
        col("unita"),
        col("origine"),
    );
    let mut s = SessionData::default();
    let mut r = Riepilogo::new(FormatoImportazione::CsvObdio, nome_file);
    let mut origini: BTreeSet<String> = BTreeSet::new();
    let mut ecu: Vec<String> = Vec::new();
    for riga in righe {
        if riga.len() != intestazione.len() {
            r.righe_irregolari += 1;
        }
        let cella = |i: Option<usize>| i.and_then(|i| riga.get(i)).map_or("", |c| c.as_str());
        let parametro = cella(Some(i_par)).trim();
        // Un'intestazione ripetuta (file concatenati) si salta.
        if parametro == "parametro" {
            r.intestazioni_ripetute += 1;
            continue;
        }
        let Some(t) = numero(cella(Some(i_t)), virgola) else {
            r.righe_scartate += 1;
            continue;
        };
        if parametro.is_empty() {
            r.righe_scartate += 1;
            continue;
        }
        let ts = cella(i_ts).trim().to_string();
        let o = cella(i_orig).trim();
        if !o.is_empty() {
            origini.insert(o.to_string());
        }
        let stato = cella(Some(i_stato)).trim();
        // «marcatore» non è mai la chiave di un parametro.
        if parametro == "marcatore" {
            s.marcatori.push(Marker {
                t,
                ts,
                etichetta: cella(i_desc).to_string(),
            });
            continue;
        }
        let Some(mut stato) = Stato::da_codice(stato) else {
            r.righe_scartate += 1;
            continue;
        };
        let valore = if stato == Stato::Ok {
            let v = numero(cella(Some(i_val)), virgola);
            if v.is_none() {
                // «ok» senza un numero leggibile: mai un valore inventato.
                stato = Stato::ErroreComunicazione;
                r.valori_non_numerici += 1;
            }
            v
        } else {
            None
        };
        let e = cella(i_ecu).trim().to_string();
        if !e.is_empty() && !ecu.contains(&e) {
            ecu.push(e.clone());
        }
        let unita = cella(i_unita).to_string();
        if !pids::nel_catalogo(parametro) && !s.intestazione.parametri.contains_key(parametro) {
            s.intestazione.parametri.insert(
                parametro.to_string(),
                MetaParametro {
                    nome: cella(i_desc).to_string(),
                    unita: unita.clone(),
                    decimali: None,
                },
            );
        }
        s.campioni.push(Sample {
            t,
            ts,
            ecu: e,
            key: parametro.to_string(),
            pid: pids::per_key(parametro)
                .filter(|p| !p.libero)
                .map_or(0, |p| p.pid),
            valore,
            unita,
            stato,
        });
    }
    if s.campioni.is_empty() && s.marcatori.is_empty() {
        return Err(if r.righe_scartate > 0 {
            format!(
                "nessuna riga leggibile ({} righe scartate: tempo o stato non validi)",
                r.righe_scartate
            )
        } else {
            "il file contiene solo l'intestazione: nessun dato da importare".into()
        });
    }
    let origine = origini.iter().next().and_then(|o| Origine::da_codice(o));
    if origini.len() > 1 {
        r.avvisi.push(format!(
            "Il file mescola origini diverse ({}): vale la prima.",
            origini.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if origine == Some(Origine::Riproduzione) {
        r.avvisi.push(
            "L'origine «riproduzione» non è un dato del veicolo: la sessione è trattata come \
             dati del veicolo registrati."
                .into(),
        );
    }
    let primo_ts = s
        .campioni
        .iter()
        .map(|c| c.ts.as_str())
        .chain(s.marcatori.iter().map(|m| m.ts.as_str()))
        .find(|t| chrono::DateTime::parse_from_rfc3339(t).is_ok())
        .map(str::to_string);
    s.intestazione = Intestazione {
        versione: 1,
        creata: primo_ts.unwrap_or_else(adesso_iso),
        demo: origine == Some(Origine::Demo),
        ecu,
        parametri: std::mem::take(&mut s.intestazione.parametri),
        ..Default::default()
    };
    classifica_chiavi(&s, &mut r);
    Ok(Importazione {
        sessione: s,
        riepilogo: r,
    })
}

/// Come leggere una colonna di Torque.
#[derive(Clone, Debug)]
struct ColonnaTorque {
    indice: usize,
    chiave: String,
    unita: String,
    conversione: Conversione,
}

/// Colonne di un'intestazione di Torque.
struct MappaTorque {
    n: usize,
    ora: Option<usize>,
    colonne: Vec<ColonnaTorque>,
}

fn e_intestazione_torque(r: &[String]) -> bool {
    r.iter().any(|c| norm(c) == "device time")
}

/// Decide la chiave di ogni colonna. `assegnate` ricorda le decisioni (per
/// nome di colonna) fra un'intestazione ripetuta e l'altra.
fn mappa_torque(
    riga: &[String],
    assegnate: &mut BTreeMap<String, (String, String, Conversione)>,
    s: &mut SessionData,
    r: &mut Riepilogo,
) -> MappaTorque {
    let mut m = MappaTorque {
        n: riga.len(),
        ora: None,
        colonne: Vec::new(),
    };
    let mut usate: BTreeSet<String> = BTreeSet::new();
    let mut occorrenze: BTreeMap<String, usize> = BTreeMap::new();
    for (indice, cella) in riga.iter().enumerate() {
        let grezza = cella.trim();
        let nome_norm = norm(grezza);
        if nome_norm == "device time" {
            m.ora = Some(indice);
            continue;
        }
        if nome_norm == "gps time" || nome_norm.is_empty() {
            continue;
        }
        // Stesso nome ripetuto nella stessa intestazione: colonne distinte.
        let occ = occorrenze.entry(nome_norm.clone()).or_default();
        *occ += 1;
        let n = format!("{nome_norm}#{occ}");
        if let Some((chiave, unita, conv)) = assegnate.get(&n)
            && !usate.contains(chiave)
        {
            usate.insert(chiave.clone());
            m.colonne.push(ColonnaTorque {
                indice,
                chiave: chiave.clone(),
                unita: unita.clone(),
                conversione: *conv,
            });
            continue;
        }
        let gia = |k: &str, usate: &BTreeSet<String>| {
            usate.contains(k) || assegnate.values().any(|(x, _, _)| x == k)
        };
        let (nome, unita_file) = dividi_intestazione(grezza);
        // 1. Tabella di Torque. 2. Nome OBDio (con «[chiave]» se ambiguo).
        let candidato = COLONNE_TORQUE
            .iter()
            .find(|(t, _, _)| norm(t) == norm(&nome))
            .map(|(_, k, _)| k.to_string())
            .or_else(|| {
                let (base, tra_quadre) = match nome.trim_end().strip_suffix(']') {
                    Some(x) => match x.rsplit_once('[') {
                        Some((b, k)) => (b.trim().to_string(), Some(k.trim().to_string())),
                        None => (nome.clone(), None),
                    },
                    None => (nome.clone(), None),
                };
                if let Some(k) = tra_quadre.filter(|k| pids::nel_catalogo(k)) {
                    return Some(k);
                }
                let uguali: Vec<&pids::Param> = pids::catalogo()
                    .iter()
                    .filter(|p| norm(&p.nome) == norm(&base))
                    .collect();
                (uguali.len() == 1).then(|| uguali[0].key.clone())
            });
        let riconosciuta = candidato.and_then(|k| {
            let p = pids::per_key(&k)?;
            let conv = conversione(&unita_file, p.unita)?;
            (!gia(&k, &usate)).then(|| (k, p.unita.to_string(), conv))
        });
        let (chiave, unita, conv) = match riconosciuta {
            Some((k, u, conv)) => {
                r.riconosciuti.push(ColonnaRiconosciuta {
                    colonna: grezza.to_string(),
                    chiave: k.clone(),
                    conversione: (conv != Conversione::Nessuna)
                        .then(|| format!("{unita_file} → {u}")),
                });
                (k, u, conv)
            }
            None => {
                let base = chiave_libera(&nome);
                let mut k = base.clone();
                let mut i = 2;
                while gia(&k, &usate) {
                    k = format!("{base}_{i}");
                    i += 1;
                }
                s.intestazione.parametri.insert(
                    k.clone(),
                    MetaParametro {
                        nome: nome.clone(),
                        unita: unita_file.clone(),
                        decimali: None,
                    },
                );
                r.liberi.push(ColonnaLibera {
                    colonna: grezza.to_string(),
                    chiave: k.clone(),
                    unita: unita_file.clone(),
                });
                (k, unita_file, Conversione::Nessuna)
            }
        };
        usate.insert(chiave.clone());
        assegnate.insert(n, (chiave.clone(), unita.clone(), conv));
        m.colonne.push(ColonnaTorque {
            indice,
            chiave,
            unita,
            conversione: conv,
        });
    }
    m
}

/// CSV in stile Torque: una riga per istante, una colonna per parametro.
fn importa_torque(testo: &str, nome_file: &str) -> Result<Importazione, String> {
    let testo = testo.trim_start_matches('\u{feff}');
    let prima = testo.lines().find(|r| !r.trim().is_empty()).unwrap_or("");
    let sep = separatore(prima);
    let virgola = sep == ';';
    let mut s = SessionData::default();
    let mut r = Riepilogo::new(FormatoImportazione::CsvTorque, nome_file);
    let mut assegnate = BTreeMap::new();
    let mut mappa: Option<MappaTorque> = None;
    let mut t0: Option<NaiveDateTime> = None;
    let mut righe_dati = 0usize;
    for riga in righe_csv(testo, sep) {
        if vuota(&riga) {
            continue;
        }
        if e_intestazione_torque(&riga) {
            if mappa.is_some() {
                r.intestazioni_ripetute += 1;
            }
            mappa = Some(mappa_torque(&riga, &mut assegnate, &mut s, &mut r));
            continue;
        }
        let Some(m) = &mappa else {
            r.righe_scartate += 1;
            continue;
        };
        righe_dati += 1;
        if riga.len() != m.n {
            r.righe_irregolari += 1;
        }
        let Some(quando) = m
            .ora
            .and_then(|i| riga.get(i))
            .and_then(|c| leggi_ora_torque(c))
        else {
            r.righe_scartate += 1;
            continue;
        };
        let inizio = *t0.get_or_insert(quando);
        let t = (quando - inizio)
            .num_microseconds()
            .map_or(0.0, |us| us as f64 / 1e6);
        let ts = iso_locale(&quando);
        for col in &m.colonne {
            let cella = riga.get(col.indice).map_or("", |c| c.trim());
            if cella.is_empty() {
                continue;
            }
            let (valore, stato) = if cella == "-" {
                (None, Stato::RispostaMancante)
            } else {
                match numero(cella, virgola) {
                    Some(v) => (Some(col.conversione.applica(v)), Stato::Ok),
                    None => {
                        r.valori_non_numerici += 1;
                        (None, Stato::ErroreComunicazione)
                    }
                }
            };
            s.campioni.push(Sample {
                t,
                ts: ts.clone(),
                ecu: String::new(),
                key: col.chiave.clone(),
                pid: pids::per_key(&col.chiave)
                    .filter(|p| !p.libero)
                    .map_or(0, |p| p.pid),
                valore,
                unita: col.unita.clone(),
                stato,
            });
        }
    }
    let Some(m) = &mappa else {
        return Err("manca l'intestazione con la colonna «Device Time»".into());
    };
    if m.ora.is_none() {
        return Err("manca la colonna «Device Time»".into());
    }
    if righe_dati == 0 {
        return Err("il file contiene solo l'intestazione: nessun dato da importare".into());
    }
    if t0.is_none() {
        return Err(format!(
            "nessuna riga con un'ora leggibile nella colonna «Device Time» ({} righe scartate)",
            r.righe_scartate
        ));
    }
    if s.campioni.windows(2).any(|w| w[1].t < w[0].t) {
        r.avvisi
            .push("Le righe non sono in ordine di tempo: la riproduzione le riordina.".into());
    }
    s.intestazione.versione = 1;
    s.intestazione.creata = t0.map(|d| iso_locale(&d)).unwrap_or_else(adesso_iso);
    s.intestazione.adattatore = "Torque (file importato)".into();
    r.avvisi.push(
        "Torque non indica la centralina che ha risposto né il motivo di un valore assente: \
         ogni «-» è importato come «risposta mancante»."
            .into(),
    );
    Ok(Importazione {
        sessione: s,
        riepilogo: r,
    })
}

/// Nome sicuro per un file a partire da un testo.
fn pulisci_nome(s: &str) -> String {
    let p: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    let p = p.trim_matches('_').to_string();
    if p.is_empty() { "file".into() } else { p }
}

/// Salva la sessione importata nella cartella delle sessioni come sessione
/// OBDio: `<data della sessione>[_DEMO]_IMPORTATA_<nome del file>.obdio.jsonl`.
/// Restituisce la sessione con il percorso definitivo.
pub fn salva_importata(cartella: &Path, imp: &Importazione) -> Result<SessionData, String> {
    let i = &imp.sessione.intestazione;
    let data = chrono::DateTime::parse_from_rfc3339(&i.creata)
        .map(|d| d.format("%Y-%m-%d_%H-%M-%S").to_string())
        .unwrap_or_else(|_| chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
    let origine = i
        .importata
        .as_ref()
        .map(|x| x.file.as_str())
        .unwrap_or(imp.riepilogo.file.as_str());
    let stem = origine
        .strip_suffix(recording::ESTENSIONE)
        .or_else(|| Path::new(origine).file_stem().and_then(|x| x.to_str()))
        .unwrap_or(origine);
    let base = format!(
        "{data}{}_IMPORTATA_{}",
        if i.demo { "_DEMO" } else { "" },
        pulisci_nome(stem)
    );
    let percorso = recording::salva_nuova(cartella, &base, &imp.sessione)?;
    let mut s = imp.sessione.clone();
    s.percorso = percorso;
    s.righe_scartate = 0;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnosis::DiagnosisReport;

    fn campione(t: f64, key: &str, v: Option<f64>, stato: Stato) -> Sample {
        let p = pids::per_key(key);
        Sample {
            t,
            ts: format!("2026-11-21T19:37:{:06.3}+01:00", 41.0 + (t % 15.0)),
            ecu: if stato == Stato::Ok {
                "7E8".into()
            } else {
                String::new()
            },
            key: key.into(),
            pid: p.filter(|p| !p.libero).map_or(0, |p| p.pid),
            valore: v,
            unita: p.map_or("bar".into(), |p| p.unita.to_string()),
            stato,
        }
    }

    /// Confronto dopo un passaggio per il JSON delle sessioni, che può
    /// cambiare l'ultima cifra binaria di un numero.
    fn uguali_circa(a: &[Sample], b: &[Sample]) {
        assert_eq!(a.len(), b.len());
        let vicini = |x: f64, y: f64| (x - y).abs() <= 1e-12 * x.abs().max(1.0);
        for (x, y) in a.iter().zip(b) {
            assert!(vicini(x.t, y.t), "{} {}", x.t, y.t);
            match (x.valore, y.valore) {
                (Some(p), Some(q)) => assert!(vicini(p, q), "{p} {q}"),
                (p, q) => assert_eq!(p, q),
            }
            assert_eq!(
                (&x.ts, &x.ecu, &x.key, x.pid, &x.unita, x.stato),
                (&y.ts, &y.ecu, &y.key, y.pid, &y.unita, y.stato)
            );
        }
    }

    /// Sessione con valori «scomodi»: decimali non rappresentabili, letture
    /// fallite di ogni tipo, due grandezze nello stesso istante, un parametro
    /// libero, marcatori con virgole e virgolette.
    fn sessione_di_prova() -> SessionData {
        let mut s = SessionData {
            intestazione: Intestazione {
                versione: 1,
                creata: "2026-11-21T19:37:41.000+01:00".into(),
                demo: true,
                adattatore: "ELM327 v1.5".into(),
                ecu: vec!["7E8".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        s.intestazione.parametri.insert(
            "prova:pressione_olio".into(),
            MetaParametro {
                nome: "Pressione olio, motore".into(),
                unita: "bar".into(),
                decimali: Some(2),
            },
        );
        recording::registra_parametri(&s.intestazione);
        let t0 = 0.104_238_000_000_000_01;
        s.campioni = vec![
            campione(t0, "o2s1_lambda", Some(1.012_451_171_875), Stato::Ok),
            campione(t0, "o2s1_tensione_wr", Some(1.529_052_734_375), Stato::Ok),
            campione(0.158, "o2s2_tensione", Some(0.625), Stato::Ok),
            campione(0.212, "stft_b1", Some(0.781_25), Stato::Ok),
            campione(0.5, "giri", Some(2128.25), Stato::Ok),
            campione(0.6, "velocita", Some(69.0), Stato::Ok),
            campione(0.6, pids::CHIAVE_DISTANZA_PARZIALE, Some(0.0), Stato::Ok),
            campione(1.1, "o2s2_tensione", None, Stato::RispostaMancante),
            campione(1.2, "temp_olio", None, Stato::NonSupportato),
            campione(1.3, "giri", None, Stato::ErroreComunicazione),
            campione(
                1.4,
                "prova:pressione_olio",
                Some(std::f64::consts::PI),
                Stato::Ok,
            ),
            campione(
                5.122_000_000_000_001,
                "temp_refrigerante",
                Some(83.0),
                Stato::Ok,
            ),
        ];
        s.marcatori = vec![
            Marker {
                t: 0.55,
                ts: "2026-11-21T19:37:41.550+01:00".into(),
                etichetta: "Minimo, a \"caldo\"".into(),
            },
            Marker {
                t: 5.0,
                ts: "2026-11-21T19:37:46.000+01:00".into(),
                etichetta: "2.500 giri".into(),
            },
        ];
        s
    }

    #[test]
    fn csv_obdio_andata_e_ritorno_esatti() {
        let s = sessione_di_prova();
        let testo = recording::csv(&s);
        let imp = importa_byte(testo.as_bytes(), "prova.csv").unwrap();
        assert_eq!(imp.riepilogo.formato, FormatoImportazione::CsvObdio);
        let r = &imp.sessione;
        assert_eq!(r.campioni, s.campioni, "campioni identici, bit per bit");
        assert_eq!(r.marcatori, s.marcatori);
        assert!(r.intestazione.demo, "origine demo conservata");
        assert_eq!(r.intestazione.origine(), Origine::Demo);
        assert_eq!(
            r.intestazione.parametri["prova:pressione_olio"].nome,
            "Pressione olio, motore"
        );
        assert_eq!(r.intestazione.importata.as_ref().unwrap().file, "prova.csv");
        assert_eq!(imp.riepilogo.righe_scartate, 0);
        assert_eq!(imp.riepilogo.liberi.len(), 1);
        // E il CSV riesportato è identico.
        assert_eq!(recording::csv(r), testo);
        // Origine veicolo.
        let mut v = s.clone();
        v.intestazione.demo = false;
        let imp = importa_byte(recording::csv(&v).as_bytes(), "v.csv").unwrap();
        assert_eq!(imp.sessione.intestazione.origine(), Origine::Veicolo);
    }

    #[test]
    fn csv_obdio_da_excel_italiano_con_punto_e_virgola() {
        let testo = "timestamp;tempo_s;ecu;parametro;descrizione;valore;unita;stato;origine\r\n\
                     2026-11-21T19:37:41.512+01:00;0,104;7E8;o2s1_lambda;Sonda;1,0124;λ;ok;veicolo\r\n\
                     2026-11-21T19:37:41.600+01:00;0,2;7E8;giri;Giri motore;2.128,5;giri/min;ok;veicolo\r\n\
                     2026-11-21T19:37:42.000+01:00;0,6;;o2s2_tensione;Sonda;;V;risposta_mancante;veicolo\r\n\
                     2026-11-21T19:37:46.000+01:00;5;;marcatore;Minimo; a caldo;;;marcatore;veicolo\r\n";
        let imp = importa_byte(testo.as_bytes(), "excel.csv").unwrap();
        let c = &imp.sessione.campioni;
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].valore, Some(1.0124));
        assert_eq!(c[0].t, 0.104);
        assert_eq!(c[1].valore, Some(2128.5));
        assert_eq!((c[2].valore, c[2].stato), (None, Stato::RispostaMancante));
        // La riga del marcatore ha una colonna in più (punto e virgola nel testo).
        assert_eq!(imp.riepilogo.righe_irregolari, 1);
        assert_eq!(imp.sessione.marcatori.len(), 1);
        assert_eq!(imp.sessione.marcatori[0].t, 5.0);
    }

    #[test]
    fn csv_obdio_malformato() {
        let righe = [
            recording::INTESTAZIONE_CSV,
            // Tempo illeggibile.
            "2026-11-21T19:37:41.512+01:00,abc,7E8,giri,Giri,900.0,giri/min,ok,veicolo",
            // Stato sconosciuto.
            "2026-11-21T19:37:41.512+01:00,1.0,7E8,giri,Giri,900.0,giri/min,boh,veicolo",
            // «ok» senza numero: errore di comunicazione, mai zero.
            "2026-11-21T19:37:41.512+01:00,2.0,7E8,giri,Giri,x,giri/min,ok,veicolo",
            // Stato non ok con un numero: il numero non viene conservato.
            "2026-11-21T19:37:41.512+01:00,3.0,,giri,Giri,0.0,giri/min,risposta_mancante,veicolo",
            // Colonne mancanti (riga troncata).
            "2026-11-21T19:37:41.512+01:00,4.0,7E8,gi",
        ];
        let testo = righe.join("\n");
        let imp = importa_byte(testo.as_bytes(), "m.csv").unwrap();
        let c = &imp.sessione.campioni;
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].valore, c[0].stato),
            (None, Stato::ErroreComunicazione)
        );
        assert_eq!((c[1].valore, c[1].stato), (None, Stato::RispostaMancante));
        assert_eq!(imp.riepilogo.righe_scartate, 3);
        assert_eq!(imp.riepilogo.valori_non_numerici, 1);
        assert_eq!(imp.riepilogo.righe_irregolari, 1);
        // Solo intestazione.
        let e = importa_byte(recording::INTESTAZIONE_CSV.as_bytes(), "x.csv").unwrap_err();
        assert!(e.contains("solo l'intestazione"), "{e}");
    }

    #[test]
    fn esportazione_torque() {
        let s = sessione_di_prova();
        let testo = csv_torque(&s);
        let righe: Vec<&str> = testo.lines().collect();
        assert!(!testo.starts_with('\u{feff}'));
        let intestazione: Vec<&str> = righe[0].split(',').collect();
        assert_eq!(&intestazione[..2], &["GPS Time", "Device Time"]);
        assert!(intestazione.contains(&"Engine RPM(rpm)"));
        assert!(intestazione.contains(&"Speed (OBD)(km/h)"));
        assert!(intestazione.contains(&"O2 Volts Bank 1 sensor 2(V)"));
        assert!(intestazione.contains(&"Sonda B1S1 (a monte) — lambda [o2s1_lambda](λ)"));
        assert!(intestazione.contains(&"Distanza parziale calcolata(km)"));
        // Il parametro libero ha una virgola nel nome: tra virgolette.
        assert!(righe[0].contains("\"Pressione olio, motore(bar)\""));
        // Un istante per riga: lambda e tensione della stessa risposta insieme.
        assert_eq!(righe.len(), 1 + 10);
        let prima: Vec<&str> = righe[1].split(',').collect();
        assert_eq!(prima[0], "");
        assert_eq!(prima[1], "21-Nov-2026 19:37:41.104");
        assert_eq!(prima[2], "1.012451171875");
        assert_eq!(prima[3], "1.529052734375");
        assert!(prima[4..].iter().all(|c| c.is_empty()), "{prima:?}");
        // Letture fallite: «-», mai zero.
        let manca = righe.iter().find(|r| r.contains("19:37:42.100")).unwrap();
        assert!(manca.split(',').any(|c| c == "-"));
        assert!(!manca.split(',').skip(2).any(|c| c == "0"));
    }

    #[test]
    fn torque_andata_e_ritorno() {
        let s = sessione_di_prova();
        let testo = csv_torque(&s);
        let imp = importa_byte(testo.as_bytes(), "torque.csv").unwrap();
        assert_eq!(imp.riepilogo.formato, FormatoImportazione::CsvTorque);
        let r = &imp.sessione;
        assert_eq!(r.campioni.len(), s.campioni.len());
        // Il tempo riparte dalla prima riga (Device Time al millisecondo).
        let t0 = s.campioni[0].t;
        for (a, b) in r.campioni.iter().zip(&s.campioni) {
            if pids::nel_catalogo(&b.key) {
                assert_eq!(a.key, b.key, "chiavi conservate");
            } else {
                assert_eq!(a.key, "torque:pressione_olio_motore");
            }
            assert!((a.t - (b.t - t0)).abs() < 0.002, "{} {}", a.t, b.t);
            assert_eq!(a.valore, b.valore, "{}", a.key);
            // Ogni lettura fallita torna come «risposta mancante».
            let atteso = if b.stato == Stato::Ok {
                Stato::Ok
            } else {
                Stato::RispostaMancante
            };
            assert_eq!(a.stato, atteso);
        }
        assert!(imp.riepilogo.liberi.iter().any(|l| l.unita == "bar"));
        // Il parametro libero diventa una chiave «torque:» stabile.
        assert!(
            r.campioni
                .iter()
                .any(|c| c.key == "torque:pressione_olio_motore")
        );
    }

    /// Registro scritto a mano secondo le convenzioni di Torque (non è un
    /// file prodotto dall'app Torque).
    const TORQUE_A_MANO: &str = "\u{feff}GPS Time, Device Time, Longitude, Latitude,GPS Speed (Meters/second), Engine RPM(rpm), Speed (OBD)(mph), Engine Coolant Temperature(°F),Intake Manifold Pressure(psi),Throttle Position(Manifold)(%),Turbo Boost & Vacuum Gauge(psi)\r\n\
        Sat Nov 21 19:37:41 GMT+01:00 2026,21-Nov-2026 19:37:41.512,7.6,45.07,0.0,812.5,0,176,14.5,12.2,-\r\n\
        Sat Nov 21 19:37:42 GMT+01:00 2026,21-Nov-2026 19:37:42.512,7.6,45.07,1.2,-,10,177.8,-,13,0.5\r\n\
        -,illeggibile,7.6,45.07,1.2,900,10,178,14,13,0.5\r\n\
        GPS Time, Device Time, Longitude, Latitude,GPS Speed (Meters/second), Engine RPM(rpm), Speed (OBD)(mph), Engine Coolant Temperature(°F),Intake Manifold Pressure(psi),Throttle Position(Manifold)(%),Turbo Boost & Vacuum Gauge(psi)\r\n\
        ,21-Nov-2026 19:37:43.012,7.6,45.07,2.0,1500,20,179.6,10,N/A\r\n\
        ,21-Nov-2026 19:3";

    #[test]
    fn torque_scritto_a_mano() {
        let imp = importa_byte(TORQUE_A_MANO.as_bytes(), "trackLog.csv").unwrap();
        let r = &imp.riepilogo;
        assert_eq!(r.formato, FormatoImportazione::CsvTorque);
        assert_eq!(r.intestazioni_ripetute, 1);
        // Ora illeggibile e ultima riga troncata (ora incompleta).
        assert_eq!(r.righe_scartate, 2);
        // La riga dopo l'intestazione ripetuta ha una colonna in meno, come la troncata.
        assert_eq!(r.righe_irregolari, 2);
        assert_eq!(r.valori_non_numerici, 1);
        let chiavi: BTreeSet<&str> = r.riconosciuti.iter().map(|c| c.chiave.as_str()).collect();
        assert_eq!(
            chiavi,
            BTreeSet::from(["giri", "velocita", "temp_refrigerante", "map", "farfalla"])
        );
        assert!(
            r.riconosciuti
                .iter()
                .any(|c| c.conversione.as_deref() == Some("mph → km/h"))
        );
        let liberi: Vec<&str> = r.liberi.iter().map(|c| c.chiave.as_str()).collect();
        assert_eq!(
            liberi,
            vec![
                "torque:longitude",
                "torque:latitude",
                "torque:gps_speed",
                "torque:turbo_boost_vacuum_gauge"
            ]
        );
        let s = &imp.sessione;
        let di = |k: &str| -> Vec<(f64, Option<f64>, Stato)> {
            s.campioni
                .iter()
                .filter(|c| c.key == k)
                .map(|c| (c.t, c.valore, c.stato))
                .collect()
        };
        let giri = di("giri");
        assert_eq!(giri[0], (0.0, Some(812.5), Stato::Ok));
        assert_eq!(
            giri[1],
            (1.0, None, Stato::RispostaMancante),
            "«-» non è zero"
        );
        assert_eq!(giri[2].1, Some(1500.0));
        assert!((giri[2].0 - 1.5).abs() < 1e-9);
        let v = di("velocita");
        assert_eq!(v[0].1, Some(0.0), "uno zero vero resta zero");
        assert!((v[1].1.unwrap() - 16.09344).abs() < 1e-9);
        let temp = di("temp_refrigerante");
        assert!((temp[0].1.unwrap() - 80.0).abs() < 1e-9, "176 °F = 80 °C");
        assert_eq!(
            s.campioni.iter().find(|c| c.key == "map").unwrap().unita,
            "kPa"
        );
        assert!((di("map")[0].1.unwrap() - 99.973_980_750_936).abs() < 1e-6);
        // Celle mancanti nella riga corta: nessun campione inventato.
        assert_eq!(di("torque:turbo_boost_vacuum_gauge").len(), 2);
        assert_eq!(di("farfalla")[2].2, Stato::ErroreComunicazione, "N/A");
        // Nome e unità del parametro libero vengono dall'intestazione.
        let meta = &s.intestazione.parametri["torque:turbo_boost_vacuum_gauge"];
        assert_eq!(
            (meta.nome.as_str(), meta.unita.as_str()),
            ("Turbo Boost & Vacuum Gauge", "psi")
        );
        assert_eq!(
            pids::per_key("torque:gps_speed").unwrap().unita,
            "Meters/second"
        );
        assert!(s.intestazione.creata.starts_with("2026-11-21T19:37:41.512"));
    }

    #[test]
    fn torque_solo_intestazione_e_altri_casi() {
        let solo = "GPS Time,Device Time,Engine RPM(rpm)\n";
        let e = importa_byte(solo.as_bytes(), "x.csv").unwrap_err();
        assert!(e.contains("solo l'intestazione"), "{e}");
        let nessuna_ora = "GPS Time,Device Time,Engine RPM(rpm)\n,ieri,900\n";
        let e = importa_byte(nessuna_ora.as_bytes(), "x.csv").unwrap_err();
        assert!(e.contains("Device Time") && e.contains("1 righe"), "{e}");
        let senza = "GPS Time,Engine RPM(rpm)\nx,900\n";
        let e = importa_byte(senza.as_bytes(), "x.csv").unwrap_err();
        assert!(e.contains("Device Time"), "{e}");
        // LF, separatore punto e virgola e virgola decimale (Excel italiano).
        let it = "GPS Time;Device Time;Engine RPM(rpm);Voltage (Control Module)(V)\n\
                  ;21-nov-2026 19:37:41,512;812,5;14,25\n\
                  ;21/11/2026 19:37:42;-;14\n";
        let imp = importa_byte(it.as_bytes(), "it.csv").unwrap();
        let c = &imp.sessione.campioni;
        assert_eq!(c[0].valore, Some(812.5));
        assert_eq!(c[1].valore, Some(14.25));
        assert_eq!(c[1].key, "tensione_modulo");
        assert_eq!((c[2].valore, c[2].stato), (None, Stato::RispostaMancante));
        assert!((c[3].t - 0.488).abs() < 1e-9);
        // Unità sconosciuta per un parametro noto: conservato come libero.
        let strana =
            "GPS Time,Device Time,Intake Manifold Pressure(inHg)\n,21-Nov-2026 19:37:41.512,29.9\n";
        let imp = importa_byte(strana.as_bytes(), "s.csv").unwrap();
        assert_eq!(
            imp.sessione.campioni[0].key,
            "torque:intake_manifold_pressure"
        );
        assert_eq!(imp.sessione.campioni[0].unita, "inHg");
        // Colonna ripetuta: la seconda resta un parametro libero distinto.
        let doppia =
            "GPS Time,Device Time,Engine RPM(rpm),Engine RPM(rpm)\n,21-Nov-2026 19:37:41.512,1,2\n";
        let imp = importa_byte(doppia.as_bytes(), "d.csv").unwrap();
        let k: Vec<&str> = imp
            .sessione
            .campioni
            .iter()
            .map(|c| c.key.as_str())
            .collect();
        assert_eq!(k, vec!["giri", "torque:engine_rpm"]);
    }

    #[test]
    fn file_illeggibili_non_vanno_in_panico() {
        assert!(importa_byte(b"", "v").unwrap_err().contains("vuoto"));
        assert!(
            importa_byte(b"   \r\n\n", "v")
                .unwrap_err()
                .contains("vuoto")
        );
        let e = importa_byte(b"ciao,mondo\n1,2\n", "x.txt").unwrap_err();
        assert!(e.contains("formato non riconosciuto"), "{e}");
        // Byte non UTF-8 (Latin-1: ° = 0xB0) in un file Torque.
        let mut latin = b"GPS Time,Device Time,Engine Coolant Temperature(\xB0F)\n".to_vec();
        latin.extend_from_slice(b",21-Nov-2026 19:37:41.512,212\n");
        let imp = importa_byte(&latin, "l.csv").unwrap();
        assert!((imp.sessione.campioni[0].valore.unwrap() - 100.0).abs() < 1e-9);
        assert!(imp.riepilogo.avvisi[0].contains("Latin-1"));
        // Byte casuali, virgolette mai chiuse, righe lunghissime: nessun panico.
        let mut caso: u64 = 0x1234_5678_9ABC_DEF1;
        for _ in 0..200 {
            let alfabeto = b"GPS Time,Device Time,\"-;\r\n0123(%)x\xFF";
            let mut b = Vec::new();
            caso = caso.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let n = (caso >> 33) as usize % 300;
            for _ in 0..n {
                caso ^= caso << 13;
                caso ^= caso >> 7;
                caso ^= caso << 17;
                b.push(alfabeto[(caso % alfabeto.len() as u64) as usize]);
            }
            let _ = importa_byte(&b, "caso.csv");
            let mut con_intestazione = b"GPS Time,Device Time,A(b)\n".to_vec();
            con_intestazione.extend_from_slice(&b);
            let _ = importa_byte(&con_intestazione, "caso.csv");
            let mut obdio = recording::INTESTAZIONE_CSV.as_bytes().to_vec();
            obdio.push(b'\n');
            obdio.extend_from_slice(&b);
            let _ = importa_byte(&obdio, "caso.csv");
            let mut jsonl = b"{\"tipo\":\"intestazione\",\"versione\":1".to_vec();
            jsonl.extend_from_slice(&b);
            let _ = importa_byte(&jsonl, "caso.obdio.jsonl");
        }
    }

    #[test]
    fn sessione_nativa_importata_e_troncata() {
        let s = sessione_di_prova();
        let testo = recording::testo_sessione(&s).unwrap();
        let imp = importa_byte(testo.as_bytes(), "a.obdio.jsonl").unwrap();
        assert_eq!(imp.riepilogo.formato, FormatoImportazione::Sessione);
        uguali_circa(&imp.sessione.campioni, &s.campioni);
        assert_eq!(imp.sessione.marcatori, s.marcatori);
        // CRLF e ultima riga troncata.
        let crlf = testo.replace('\n', "\r\n");
        let tronco = &crlf[..crlf.len() - 20];
        let imp = importa_byte(tronco.as_bytes(), "b.obdio.jsonl").unwrap();
        assert_eq!(imp.sessione.campioni.len(), s.campioni.len() - 1);
        assert_eq!(imp.riepilogo.righe_scartate, 1);
        let e = importa_byte(b"{\"non\":\"sessione\"}\n", "c.obdio.jsonl").unwrap_err();
        assert!(e.contains("non è una sessione"), "{e}");
    }

    #[test]
    fn intestazioni_e_ore() {
        assert_eq!(
            dividi_intestazione("Throttle Position(Manifold)(%)"),
            ("Throttle Position(Manifold)".into(), "%".into())
        );
        assert_eq!(
            dividi_intestazione(" Speed (OBD)(km/h) "),
            ("Speed (OBD)".into(), "km/h".into())
        );
        assert_eq!(
            dividi_intestazione("Longitude"),
            ("Longitude".into(), "".into())
        );
        assert_eq!(dividi_intestazione("(%)"), ("(%)".into(), "".into()));
        assert_eq!(dividi_intestazione("A((b)"), ("A(".into(), "b".into()));
        assert_eq!(chiave_libera("GPS Speed"), "torque:gps_speed");
        assert_eq!(chiave_libera("  --  "), "torque:colonna");
        let d = leggi_ora_torque("21-Nov-2026 19:37:41.512").unwrap();
        assert_eq!(ora_torque_testo(&d), "21-Nov-2026 19:37:41.512");
        let d = leggi_ora_torque("01-Jan-2027 00:00:00").unwrap();
        assert_eq!(ora_torque_testo(&d), "01-Jan-2027 00:00:00.000");
        assert!(leggi_ora_torque("21-Nov-2026 19:3").is_none());
        assert!(leggi_ora_torque("").is_none());
    }

    #[test]
    fn diagnosi_senza_vin_ed_esportazione_su_file() {
        let dir = std::env::temp_dir().join(format!("obdio_scambio_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = sessione_di_prova();
        assert!(diagnosi_json(&s).is_err());
        s.diagnosi = Some(DiagnosisReport {
            letto_il: "2026-11-21T19:37:41+01:00".into(),
            vin: Some("VF3ABCDEFGHIJKLMN".into()),
            ..Default::default()
        });
        let j = diagnosi_json(&s).unwrap();
        assert!(!j.contains("VF3ABCDEFGHIJKLMN") && j.contains("letto_il"));

        // Sessione in memoria: riscritta. Poi copiata dal file.
        let p = esporta(&s, FormatoEsportazione::Sessione, &dir.join("a")).unwrap();
        assert!(p.to_string_lossy().ends_with("a.obdio.jsonl"));
        let riletta = recording::carica(&p).unwrap();
        uguali_circa(&riletta.campioni, &s.campioni);
        s.percorso = p.clone();
        let copia = esporta(
            &s,
            FormatoEsportazione::Sessione,
            &dir.join("b.obdio.jsonl"),
        )
        .unwrap();
        assert_eq!(std::fs::read(&copia).unwrap(), std::fs::read(&p).unwrap());
        assert!(
            esporta(&s, FormatoEsportazione::Sessione, &p).is_err(),
            "mai su sé stessa"
        );
        let altrove = dir.join("esportati");
        std::fs::create_dir_all(&altrove).unwrap();
        for f in FormatoEsportazione::TUTTI {
            let dest = altrove.join(nome_proposto(&s, f));
            let scritto = esporta(&s, f, &dest).unwrap();
            assert!(scritto.is_file(), "{f:?}");
            assert!(
                !std::fs::read_to_string(&scritto)
                    .unwrap()
                    .contains("VF3ABCDEFGHIJKLMN")
            );
        }
        assert_eq!(
            nome_proposto(&s, FormatoEsportazione::CsvTorque),
            "a_torque.csv"
        );
        let e = esporta(
            &s,
            FormatoEsportazione::CsvObdio,
            &dir.join("manca").join("x.csv"),
        );
        assert!(e.unwrap_err().contains("non esiste"));

        // Importazione salvata come sessione nativa, nome con _IMPORTATA.
        let imp = importa_byte(TORQUE_A_MANO.as_bytes(), "trackLog.csv").unwrap();
        let salvata = salva_importata(&dir, &imp).unwrap();
        let nome = salvata
            .percorso
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(
            nome.starts_with("2026-11-21_19-37-41_IMPORTATA_trackLog"),
            "{nome}"
        );
        assert!(nome.ends_with(recording::ESTENSIONE));
        let riaperta = recording::carica(&salvata.percorso).unwrap();
        uguali_circa(&riaperta.campioni, &imp.sessione.campioni);
        let i = riaperta.intestazione.importata.unwrap();
        assert_eq!(
            (i.file.as_str(), i.formato.as_str()),
            ("trackLog.csv", "CSV in stile Torque")
        );
        assert!(
            riaperta
                .intestazione
                .parametri
                .contains_key("torque:gps_speed")
        );
        // Una seconda importazione dello stesso file non sovrascrive.
        let altra = salva_importata(&dir, &imp).unwrap();
        assert_ne!(altra.percorso, salvata.percorso);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
