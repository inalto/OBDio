//! Sessioni su disco ed esportazione CSV.
//!
//! Una sessione è un file di testo `.obdio.jsonl`: una riga JSON per record,
//! scritta in coda man mano. Se l'app o il collegamento si interrompono, tutto
//! ciò che è già stato scritto resta leggibile.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::diagnosis::DiagnosisReport;
use crate::model::{Marker, Sample, Stato};
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
enum Record {
    Intestazione(Intestazione),
    Campione(Sample),
    Marcatore(Marker),
    Diagnosi(Box<DiagnosisReport>),
}

/// Cartella delle sessioni: accanto all'eseguibile, oppure in Documenti se
/// la cartella dell'eseguibile non è scrivibile.
pub fn cartella_sessioni() -> PathBuf {
    let accanto = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("sessioni")));
    if let Some(dir) = accanto
        && fs::create_dir_all(&dir).is_ok()
        && File::create(dir.join(".prova_scrittura")).is_ok()
    {
        let _ = fs::remove_file(dir.join(".prova_scrittura"));
        return dir;
    }
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("Documents").join("OBDio").join("sessioni");
    let _ = fs::create_dir_all(&dir);
    dir
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
    let file = File::open(percorso).map_err(|e| format!("impossibile aprire il file: {e}"))?;
    let mut s = SessionData {
        percorso: percorso.to_path_buf(),
        ..Default::default()
    };
    let mut intestazione_vista = false;
    for riga in BufReader::new(file).lines() {
        let riga = riga.map_err(|e| format!("errore di lettura: {e}"))?;
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
    Ok(s)
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

fn campo(s: &str) -> String {
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

    let origine = if s.intestazione.demo {
        "demo_sintetico"
    } else {
        "veicolo"
    };
    // BOM UTF-8: serve a Excel per riconoscere la codifica.
    let mut out = String::from("\u{feff}");
    out.push_str(INTESTAZIONE_CSV);
    out.push_str("\r\n");
    for (_, riga) in righe {
        let colonne: [String; 9] = match riga {
            Riga::C(c) => {
                let descrizione = pids::per_key(&c.key)
                    .map(|p| p.nome.clone())
                    .unwrap_or_default();
                let valore = match (c.stato, c.valore) {
                    (Stato::Ok, Some(v)) => {
                        let decimali = pids::per_key(&c.key).map(|p| p.decimali + 1).unwrap_or(4);
                        format!("{v:.decimali$}")
                    }
                    _ => String::new(),
                };
                [
                    c.ts.clone(),
                    format!("{:.3}", c.t),
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
                format!("{:.3}", m.t),
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
}
