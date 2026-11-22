//! Sessioni su disco ed esportazione CSV.
//!
//! Una sessione è un file di testo `.obdio.jsonl`: una riga JSON per record,
//! scritta in coda man mano. Se l'app o il collegamento si interrompono, tutto
//! ciò che è già stato scritto resta leggibile.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::diagnosis::DiagnosisReport;
use crate::model::{Marker, Origine, Sample, Stato};
use crate::obd::pids;

pub const ESTENSIONE: &str = ".obdio.jsonl";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Intestazione {
    pub versione: u32,
    pub creata: String,
    /// Vero se i dati sono simulati.
    pub demo: bool,
    pub adattatore: String,
    pub collegamento: String,
    pub protocollo: String,
    pub ecu: Vec<String>,
    /// Presente solo nelle sessioni create da un'importazione.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importata: Option<Importata>,
    /// Nome e unità dei parametri liberi (chiavi fuori dal catalogo, per
    /// esempio le colonne sconosciute di un CSV di Torque).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parametri: BTreeMap<String, MetaParametro>,
}

impl Intestazione {
    /// Origine dei dati registrati (una sessione non è mai una riproduzione).
    pub fn origine(&self) -> Origine {
        if self.demo {
            Origine::Demo
        } else {
            Origine::Veicolo
        }
    }
}

/// Da quale file e formato è stata importata una sessione.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Importata {
    /// Nome del file d'origine (senza cartella).
    pub file: String,
    /// Formato riconosciuto, in chiaro (per esempio «CSV in stile Torque»).
    pub formato: String,
    /// Data e ora dell'importazione, ISO 8601.
    pub il: String,
}

/// Descrizione di un parametro libero.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MetaParametro {
    pub nome: String,
    #[serde(default)]
    pub unita: String,
    /// Decimali da mostrare; `None`: due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimali: Option<usize>,
}

/// Rende noti all'app i parametri liberi dell'intestazione (vedi
/// [`pids::registra_libero`]).
pub fn registra_parametri(i: &Intestazione) {
    for (k, m) in &i.parametri {
        pids::registra_libero(k, &m.nome, &m.unita, m.decimali.unwrap_or(2));
    }
}

/// Nome leggibile di un parametro della sessione: dal catalogo, altrimenti
/// dall'intestazione, altrimenti la chiave stessa.
pub fn nome_parametro(s: &SessionData, key: &str) -> String {
    if pids::nel_catalogo(key) {
        return pids::per_key(key)
            .map(|p| p.nome.clone())
            .unwrap_or_default();
    }
    match s.intestazione.parametri.get(key) {
        Some(m) if !m.nome.is_empty() => m.nome.clone(),
        _ => pids::per_key(key)
            .map(|p| p.nome.clone())
            .unwrap_or_else(|| key.to_string()),
    }
}

/// Decimali usati nei file per un parametro (uno in più di quelli mostrati).
pub(crate) fn decimali_file(s: &SessionData, key: &str) -> usize {
    if pids::nel_catalogo(key) {
        return pids::per_key(key).map(|p| p.decimali + 1).unwrap_or(4);
    }
    s.intestazione
        .parametri
        .get(key)
        .and_then(|m| m.decimali)
        .map(|d| d + 1)
        .unwrap_or(4)
}

/// Numero per i file CSV: con `decimali` cifre se così si rilegge il valore
/// esatto, altrimenti con tutte le cifre necessarie (mai in notazione
/// esponenziale). Così l'esportazione non perde precisione.
pub fn numero_csv(v: f64, decimali: usize) -> String {
    let fisso = format!("{v:.decimali$}");
    if fisso.parse::<f64>().ok() == Some(v) {
        fisso
    } else {
        format!("{v}")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
enum Record {
    Intestazione(Intestazione),
    Campione(Sample),
    Marcatore(Marker),
    Diagnosi(Box<DiagnosisReport>),
}

/// Cartella delle sessioni: nella cartella base se fissata (Android), altrimenti
/// accanto all'eseguibile oppure in Documenti se questa non è scrivibile.
pub fn cartella_sessioni() -> PathBuf {
    crate::config::cartella_dati("sessioni")
}

/// Scrive una sessione mentre i campioni arrivano.
pub struct Recorder {
    file: BufWriter<File>,
    percorso: PathBuf,
    /// Tempo (asse dal vivo) corrispondente allo zero della registrazione.
    t_inizio: f64,
    ultimo_flush: Instant,
    pub campioni: usize,
}

impl Recorder {
    /// Crea il file e scrive intestazione ed eventuale diagnosi già letta.
    pub fn avvia(
        cartella: &Path,
        intestazione: &Intestazione,
        diagnosi: Option<&DiagnosisReport>,
        t_inizio: f64,
    ) -> std::io::Result<Self> {
        fs::create_dir_all(cartella)?;
        let base = format!(
            "{}{}",
            chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"),
            if intestazione.demo { "_DEMO" } else { "" }
        );
        let mut percorso = cartella.join(format!("{base}{ESTENSIONE}"));
        let mut n = 2;
        while percorso.exists() {
            percorso = cartella.join(format!("{base}_{n}{ESTENSIONE}"));
            n += 1;
        }
        let mut rec = Recorder {
            file: BufWriter::new(File::create(&percorso)?),
            percorso,
            t_inizio,
            ultimo_flush: Instant::now(),
            campioni: 0,
        };
        rec.scrivi(&Record::Intestazione(intestazione.clone()))?;
        if let Some(d) = diagnosi {
            rec.scrivi(&Record::Diagnosi(Box::new(d.clone())))?;
        }
        rec.file.flush()?;
        Ok(rec)
    }

    pub fn percorso(&self) -> &Path {
        &self.percorso
    }

    /// Secondi dall'inizio della registrazione per un tempo dell'asse dal vivo.
    pub fn relativo(&self, t: f64) -> f64 {
        (t - self.t_inizio).max(0.0)
    }

    fn scrivi(&mut self, r: &Record) -> std::io::Result<()> {
        serde_json::to_writer(&mut self.file, r)?;
        self.file.write_all(b"\n")
    }

    fn flush_periodico(&mut self) -> std::io::Result<()> {
        if self.ultimo_flush.elapsed().as_millis() >= 1000 {
            self.file.flush()?;
            self.ultimo_flush = Instant::now();
        }
        Ok(())
    }

    /// Aggiunge un campione; `s.t` è sull'asse dal vivo e viene reso relativo.
    pub fn campione(&mut self, s: &Sample) -> std::io::Result<()> {
        let mut s = s.clone();
        s.t = self.relativo(s.t);
        self.scrivi(&Record::Campione(s))?;
        self.campioni += 1;
        self.flush_periodico()
    }

    pub fn marcatore(&mut self, m: &Marker) -> std::io::Result<()> {
        let mut m = m.clone();
        m.t = self.relativo(m.t);
        self.scrivi(&Record::Marcatore(m))?;
        self.file.flush()
    }

    pub fn diagnosi(&mut self, d: &DiagnosisReport) -> std::io::Result<()> {
        self.scrivi(&Record::Diagnosi(Box::new(d.clone())))?;
        self.file.flush()
    }

    /// Chiude il file e restituisce il percorso.
    pub fn chiudi(mut self) -> std::io::Result<PathBuf> {
        self.file.flush()?;
        Ok(self.percorso.clone())
    }
}

/// Prefisso del file con la copia della diagnosi salvata prima di cancellare
/// i codici guasto.
pub const PREFISSO_COPIA_DIAGNOSI: &str = "diagnosi_prima_cancellazione_";

/// Salva in `cartella` una copia JSON della diagnosi
/// (`diagnosi_prima_cancellazione_AAAA-MM-GG_HH-MM-SS[_DEMO].json`) e ne
/// restituisce il percorso. Non sovrascrive mai un file esistente. Il VIN
/// non viene scritto.
pub fn salva_copia_diagnosi(cartella: &Path, d: &DiagnosisReport) -> std::io::Result<PathBuf> {
    fs::create_dir_all(cartella)?;
    let base = format!(
        "{PREFISSO_COPIA_DIAGNOSI}{}{}",
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"),
        if d.demo { "_DEMO" } else { "" }
    );
    let testo = serde_json::to_string_pretty(d).map_err(std::io::Error::other)?;
    let mut n = 1;
    loop {
        let nome = if n == 1 {
            format!("{base}.json")
        } else {
            format!("{base}_{n}.json")
        };
        let percorso = cartella.join(nome);
        match File::create_new(&percorso) {
            Ok(mut f) => {
                f.write_all(testo.as_bytes())?;
                f.sync_all()?;
                return Ok(percorso);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 100 => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Sessione riletta da disco.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionData {
    pub percorso: PathBuf,
    pub intestazione: Intestazione,
    pub campioni: Vec<Sample>,
    pub marcatori: Vec<Marker>,
    /// Ultima diagnosi registrata nella sessione.
    pub diagnosi: Option<DiagnosisReport>,
    /// Righe non interpretabili (per esempio l'ultima, se troncata).
    pub righe_scartate: usize,
}

pub fn carica(percorso: &Path) -> Result<SessionData, String> {
    let byte = fs::read(percorso).map_err(|e| format!("impossibile aprire il file: {e}"))?;
    let mut s = leggi(&String::from_utf8_lossy(&byte))?;
    s.percorso = percorso.to_path_buf();
    Ok(s)
}

/// Interpreta il testo di una sessione `.obdio.jsonl`. Le righe illeggibili
/// (per esempio l'ultima, troncata) sono contate e saltate. I parametri liberi
/// dell'intestazione vengono registrati.
pub fn leggi(testo: &str) -> Result<SessionData, String> {
    let mut s = SessionData::default();
    let mut intestazione_vista = false;
    for riga in testo.lines() {
        let riga = riga.trim_start_matches('\u{feff}').trim();
        if riga.is_empty() {
            continue;
        }
        match serde_json::from_str::<Record>(riga) {
            Ok(Record::Intestazione(i)) => {
                s.intestazione = i;
                intestazione_vista = true;
            }
            Ok(Record::Campione(c)) => s.campioni.push(c),
            Ok(Record::Marcatore(m)) => s.marcatori.push(m),
            Ok(Record::Diagnosi(d)) => s.diagnosi = Some(*d),
            Err(_) => s.righe_scartate += 1,
        }
    }
    if !intestazione_vista {
        return Err("il file non è una sessione OBDio (intestazione mancante)".into());
    }
    registra_parametri(&s.intestazione);
    Ok(s)
}

/// Testo `.obdio.jsonl` della sessione: intestazione, diagnosi, poi campioni
/// e marcatori in ordine di tempo.
pub fn testo_sessione(s: &SessionData) -> Result<String, String> {
    enum Voce<'a> {
        C(&'a Sample),
        M(&'a Marker),
    }
    let mut voci: Vec<(f64, Voce)> = s
        .campioni
        .iter()
        .map(|c| (c.t, Voce::C(c)))
        .chain(s.marcatori.iter().map(|m| (m.t, Voce::M(m))))
        .collect();
    voci.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = String::new();
    let mut riga = |r: &Record| -> Result<(), String> {
        out.push_str(&serde_json::to_string(r).map_err(|e| e.to_string())?);
        out.push('\n');
        Ok(())
    };
    riga(&Record::Intestazione(s.intestazione.clone()))?;
    if let Some(d) = &s.diagnosi {
        riga(&Record::Diagnosi(Box::new(d.clone())))?;
    }
    for (_, v) in voci {
        match v {
            Voce::C(c) => riga(&Record::Campione(c.clone()))?,
            Voce::M(m) => riga(&Record::Marcatore(m.clone()))?,
        }
    }
    Ok(out)
}

/// Salva la sessione in un file nuovo nella cartella, con il nome
/// `<base>.obdio.jsonl` (o `<base>_2…` se esiste già), e ne restituisce il
/// percorso. Non sovrascrive mai.
pub fn salva_nuova(cartella: &Path, base: &str, s: &SessionData) -> Result<PathBuf, String> {
    fs::create_dir_all(cartella)
        .map_err(|e| format!("impossibile creare la cartella {}: {e}", cartella.display()))?;
    let testo = testo_sessione(s)?;
    let mut n = 1;
    loop {
        let nome = if n == 1 {
            format!("{base}{ESTENSIONE}")
        } else {
            format!("{base}_{n}{ESTENSIONE}")
        };
        let percorso = cartella.join(nome);
        match File::create_new(&percorso) {
            Ok(mut f) => {
                f.write_all(testo.as_bytes())
                    .and_then(|_| f.sync_all())
                    .map_err(|e| format!("impossibile scrivere {}: {e}", percorso.display()))?;
                return Ok(percorso);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 1000 => n += 1,
            Err(e) => return Err(format!("impossibile creare {}: {e}", percorso.display())),
        }
    }
}

/// Elenco delle sessioni salvate, dalla più recente.
pub fn elenca(cartella: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(cartella)
        .map(|it| {
            it.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(ESTENSIONE))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// Percorso del CSV associato a una sessione.
pub fn percorso_csv(sessione: &Path) -> PathBuf {
    let nome = sessione
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("sessione");
    let base = nome.strip_suffix(ESTENSIONE).unwrap_or(nome);
    sessione.with_file_name(format!("{base}.csv"))
}

pub(crate) fn campo(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub const INTESTAZIONE_CSV: &str =
    "timestamp,tempo_s,ecu,parametro,descrizione,valore,unita,stato,origine";

/// Testo CSV della sessione. Vedi `docs/FORMATO_CSV.md`.
pub fn csv(s: &SessionData) -> String {
    enum Riga<'a> {
        C(&'a Sample),
        M(&'a Marker),
    }
    let mut righe: Vec<(f64, Riga)> = s
        .campioni
        .iter()
        .map(|c| (c.t, Riga::C(c)))
        .chain(s.marcatori.iter().map(|m| (m.t, Riga::M(m))))
        .collect();
    righe.sort_by(|a, b| a.0.total_cmp(&b.0));

    let origine = s.intestazione.origine().codice();
    // BOM UTF-8: serve a Excel per riconoscere la codifica.
    let mut out = String::from("\u{feff}");
    out.push_str(INTESTAZIONE_CSV);
    out.push_str("\r\n");
    for (_, riga) in righe {
        let colonne: [String; 9] = match riga {
            Riga::C(c) => {
                let descrizione = nome_parametro(s, &c.key);
                // Mai uno zero al posto di un dato assente: valore vuoto.
                let valore = match (c.stato, c.valore) {
                    (Stato::Ok, Some(v)) if v.is_finite() => {
                        numero_csv(v, decimali_file(s, &c.key))
                    }
                    _ => String::new(),
                };
                [
                    c.ts.clone(),
                    numero_csv(c.t, 3),
                    c.ecu.clone(),
                    c.key.clone(),
                    descrizione,
                    valore,
                    c.unita.clone(),
                    c.stato.codice().to_string(),
                    origine.to_string(),
                ]
            }
            Riga::M(m) => [
                m.ts.clone(),
                numero_csv(m.t, 3),
                String::new(),
                "marcatore".to_string(),
                m.etichetta.clone(),
                String::new(),
                String::new(),
                "marcatore".to_string(),
                origine.to_string(),
            ],
        };
        let riga: Vec<String> = colonne.iter().map(|c| campo(c)).collect();
        out.push_str(&riga.join(","));
        out.push_str("\r\n");
    }
    out
}

/// Scrive il CSV accanto alla sessione e ne restituisce il percorso.
pub fn esporta_csv(s: &SessionData) -> Result<PathBuf, String> {
    let percorso = percorso_csv(&s.percorso);
    fs::write(&percorso, csv(s)).map_err(|e| format!("impossibile scrivere il CSV: {e}"))?;
    Ok(percorso)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn campione(t: f64, key: &str, valore: Option<f64>, stato: Stato) -> Sample {
        Sample {
            t,
            ts: "2026-11-21T10:00:00.000+01:00".into(),
            ecu: if valore.is_some() {
                "7E8".into()
            } else {
                String::new()
            },
            key: key.into(),
            pid: pids::per_key(key).unwrap().pid,
            valore,
            unita: pids::per_key(key).unwrap().unita.into(),
            stato,
        }
    }

    fn cartella(nome: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("obdio_test_{nome}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn scrittura_rilettura_e_csv() {
        let dir = cartella("rec");
        let int = Intestazione {
            versione: 1,
            creata: "2026-11-21T10:00:00.000+01:00".into(),
            demo: true,
            adattatore: "ELM327 v1.5".into(),
            collegamento: "demo".into(),
            protocollo: "CAN".into(),
            ecu: vec!["7E8".into()],
            ..Default::default()
        };
        let mut rec = Recorder::avvia(&dir, &int, None, 100.0).unwrap();
        rec.campione(&campione(100.5, "giri", Some(2128.0), Stato::Ok))
            .unwrap();
        rec.campione(&campione(
            101.0,
            "o2s2_tensione",
            None,
            Stato::RispostaMancante,
        ))
        .unwrap();
        rec.marcatore(&Marker {
            t: 100.75,
            ts: "2026-11-21T10:00:00.750+01:00".into(),
            etichetta: "Minimo, a caldo".into(),
        })
        .unwrap();
        let percorso = rec.chiudi().unwrap();
        assert!(
            percorso
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("_DEMO")
        );

        let s = carica(&percorso).unwrap();
        assert_eq!(s.intestazione, int);
        assert_eq!(s.campioni.len(), 2);
        assert!((s.campioni[0].t - 0.5).abs() < 1e-9);
        assert_eq!(s.marcatori[0].etichetta, "Minimo, a caldo");

        let testo = csv(&s);
        let righe: Vec<&str> = testo.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(righe[0], INTESTAZIONE_CSV);
        assert_eq!(
            righe[1],
            "2026-11-21T10:00:00.000+01:00,0.500,7E8,giri,Giri motore,2128.0,giri/min,ok,demo_sintetico"
        );
        // Il marcatore è in ordine di tempo e il testo con virgola è tra virgolette.
        assert!(
            righe[2].contains(",marcatore,\"Minimo, a caldo\",,,marcatore,"),
            "{}",
            righe[2]
        );
        // Lettura mancante: valore vuoto e stato esplicito, mai zero.
        assert!(
            righe[3]
                .contains(",o2s2_tensione,Sonda B1S2 (a valle) — tensione,,V,risposta_mancante,"),
            "{}",
            righe[3]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_troncato_resta_leggibile() {
        let dir = cartella("tronco");
        let int = Intestazione {
            versione: 1,
            ..Default::default()
        };
        let mut rec = Recorder::avvia(&dir, &int, None, 0.0).unwrap();
        for i in 0..5 {
            rec.campione(&campione(i as f64, "giri", Some(900.0), Stato::Ok))
                .unwrap();
        }
        let percorso = rec.chiudi().unwrap();
        // Simula un'interruzione a metà dell'ultima riga.
        let testo = fs::read_to_string(&percorso).unwrap();
        fs::write(&percorso, &testo[..testo.len() - 25]).unwrap();
        let s = carica(&percorso).unwrap();
        assert_eq!(s.campioni.len(), 4);
        assert_eq!(s.righe_scartate, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_estraneo_rifiutato() {
        let dir = cartella("estraneo");
        let p = dir.join(format!("x{ESTENSIONE}"));
        fs::write(&p, "ciao\n").unwrap();
        assert!(carica(&p).is_err());
        assert_eq!(elenca(&dir), vec![p.clone()]);
        assert_eq!(percorso_csv(&p), dir.join("x.csv"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn numeri_csv_senza_perdita() {
        assert_eq!(numero_csv(2128.0, 1), "2128.0");
        assert_eq!(numero_csv(0.5, 3), "0.500");
        // Con i decimali fissi non si rileggerebbe lo stesso valore.
        assert_eq!(numero_csv(2128.25, 1), "2128.25");
        assert_eq!(numero_csv(1.0 / 3.0, 3), "0.3333333333333333");
        assert_eq!(numero_csv(1e-7, 2), "0.0000001");
        for v in [0.1 + 0.2, 33.333_333_333_333_336, -40.0, 1e15 + 0.5] {
            assert_eq!(numero_csv(v, 2).parse::<f64>().unwrap(), v);
        }
    }

    #[test]
    fn sessione_nuova_mai_sovrascritta_e_parametri_liberi() {
        let dir = cartella("nuova");
        let mut s = SessionData {
            intestazione: Intestazione {
                versione: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        s.intestazione.parametri.insert(
            "prova:recording_libero".into(),
            MetaParametro {
                nome: "Pressione turbo".into(),
                unita: "bar".into(),
                decimali: Some(2),
            },
        );
        s.campioni
            .push(campione(1.0, "giri", Some(900.0), Stato::Ok));
        s.marcatori.push(Marker {
            t: 0.5,
            ts: String::new(),
            etichetta: "m".into(),
        });
        let a = salva_nuova(&dir, "base", &s).unwrap();
        let b = salva_nuova(&dir, "base", &s).unwrap();
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with("base_2.obdio.jsonl"));
        let r = carica(&a).unwrap();
        assert_eq!(r.intestazione.parametri, s.intestazione.parametri);
        assert_eq!(r.campioni, s.campioni);
        // Caricare la sessione rende noto il parametro libero.
        let p = pids::per_key("prova:recording_libero").unwrap();
        assert_eq!((p.nome.as_str(), p.unita), ("Pressione turbo", "bar"));
        assert_eq!(
            nome_parametro(&r, "prova:recording_libero"),
            "Pressione turbo"
        );
        assert_eq!(nome_parametro(&r, "giri"), "Giri motore");
        // Un'intestazione senza i campi nuovi si legge come prima.
        let vecchia = r#"{"tipo":"intestazione","versione":1,"creata":"x","demo":false,"adattatore":"a","collegamento":"c","protocollo":"p","ecu":[]}"#;
        let v = leggi(vecchia).unwrap();
        assert!(v.intestazione.importata.is_none() && v.intestazione.parametri.is_empty());
        // E una nuova senza parametri liberi non scrive i campi nuovi.
        let t = testo_sessione(&v).unwrap();
        assert!(!t.contains("parametri") && !t.contains("importata"));
        let _ = fs::remove_dir_all(&dir);
    }
}
