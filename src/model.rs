//! Tipi condivisi fra thread di lavoro, registrazione e interfaccia.

use serde::{Deserialize, Serialize};

/// Esito di una singola lettura. Un dato assente non viene mai sostituito da zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stato {
    /// Valore letto e decodificato.
    Ok,
    /// La centralina dichiara di non supportare il parametro.
    NonSupportato,
    /// Parametro dichiarato ma la centralina non ha risposto (NO DATA).
    RispostaMancante,
    /// Timeout, errore di bus o risposta non interpretabile.
    ErroreComunicazione,
}

impl Stato {
    /// Codice stabile usato nei file (CSV e sessioni).
    pub fn codice(self) -> &'static str {
        match self {
            Stato::Ok => "ok",
            Stato::NonSupportato => "non_supportato",
            Stato::RispostaMancante => "risposta_mancante",
            Stato::ErroreComunicazione => "errore_comunicazione",
        }
    }

    /// Testo mostrato all'utente.
    pub fn etichetta(self) -> &'static str {
        match self {
            Stato::Ok => "OK",
            Stato::NonSupportato => "Non supportato",
            Stato::RispostaMancante => "Risposta mancante",
            Stato::ErroreComunicazione => "Errore di comunicazione",
        }
    }
}

/// Un campione di un parametro in tempo reale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Secondi dall'inizio (della connessione dal vivo, della registrazione nei file).
    pub t: f64,
    /// Data e ora locali del campione, ISO 8601 con millisecondi.
    pub ts: String,
    /// Centralina che ha risposto; vuoto se nessuna ha risposto.
    pub ecu: String,
    /// Chiave stabile del parametro (vedi `obd::pids`).
    pub key: String,
    pub pid: u8,
    /// `None` quando lo stato non è `Ok`.
    pub valore: Option<f64>,
    pub unita: String,
    pub stato: Stato,
}

/// Marcatore inserito dall'utente durante la registrazione.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub t: f64,
    pub ts: String,
    pub etichetta: String,
}

/// Data e ora locali in ISO 8601 con millisecondi.
pub fn adesso_iso() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

/// Formatta un numero all'italiana: virgola decimale e punto per le migliaia.
pub fn num_it(v: f64, decimali: usize) -> String {
    let s = format!("{:.*}", decimali, v.abs());
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    if let Some(f) = frac {
        out.push(',');
        out.push_str(f);
    }
    let zero = out.chars().all(|c| matches!(c, '0' | ',' | '.'));
    if v < 0.0 && !zero {
        out.insert(0, '−');
    }
    out
}

/// Come [`num_it`] ma con il segno sempre esplicito (correzioni carburante).
pub fn num_it_segno(v: f64, decimali: usize) -> String {
    let s = num_it(v, decimali);
    if v > 0.0 && !s.chars().all(|c| matches!(c, '0' | ',' | '.')) {
        format!("+{s}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeri_italiani() {
        assert_eq!(num_it(2128.0, 0), "2.128");
        assert_eq!(num_it(0.78125, 2), "0,78");
        assert_eq!(num_it(-3.5, 1), "−3,5");
        assert_eq!(num_it(-0.001, 2), "0,00");
        assert_eq!(num_it_segno(8.59375, 2), "+8,59");
        assert_eq!(num_it_segno(-1.0, 1), "−1,0");
        assert_eq!(num_it(1234567.5, 1), "1.234.567,5");
    }
}
