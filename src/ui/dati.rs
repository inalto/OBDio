//! Serie temporali mostrate nei grafici, dal vivo o da una sessione riaperta.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Marker, Sample, Stato};
use crate::recording::SessionData;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Punto {
    pub t: f64,
    pub v: Option<f64>,
    pub stato: Stato,
}

#[derive(Clone, Debug, Default)]
pub struct Serie {
    /// Centralina di riferimento della serie (la prima che ha risposto).
    pub ecu: String,
    pub unita: String,
    pub punti: Vec<Punto>,
    /// Statistiche correnti dei soli valori validi (mai influenzate dai buchi).
    pub stat: Statistiche,
}

/// Minimo, massimo e media corrente di una serie; tutti assenti finché non
/// arriva un valore valido.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Statistiche {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub conteggio: u64,
    pub somma: f64,
}

impl Statistiche {
    pub fn aggiungi(&mut self, v: f64) {
        self.min = Some(self.min.map_or(v, |m| m.min(v)));
        self.max = Some(self.max.map_or(v, |m| m.max(v)));
        self.conteggio += 1;
        self.somma += v;
    }

    pub fn media(&self) -> Option<f64> {
        (self.conteggio > 0).then(|| self.somma / self.conteggio as f64)
    }
}

/// Oltre questo intervallo senza campioni validi la linea viene spezzata.
pub const SOGLIA_BUCO_S: f64 = 2.5;
const MAX_PUNTI: usize = 400_000;

#[derive(Clone, Debug, Default)]
pub struct Archivio {
    pub serie: BTreeMap<String, Serie>,
    pub marcatori: Vec<Marker>,
    /// Tempo dell'ultimo campione ricevuto.
    pub t_max: f64,
}

/// Tratti continui di una serie e intervalli in cui il dato manca.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracciato {
    pub tratti: Vec<Vec<[f64; 2]>>,
    pub buchi: Vec<(f64, f64)>,
}

impl Archivio {
    pub fn aggiungi(&mut self, c: &Sample) {
        let s = self.serie.entry(c.key.clone()).or_default();
        if !c.ecu.is_empty() {
            if s.ecu.is_empty() {
                s.ecu = c.ecu.clone();
            } else if s.ecu != c.ecu {
                // Un'altra centralina risponde allo stesso PID: resta nel file,
                // ma non viene mescolata nella stessa curva.
                return;
            }
        }
        if s.unita.is_empty() {
            s.unita = c.unita.clone();
        }
        s.punti.push(Punto {
            t: c.t,
            v: c.valore,
            stato: c.stato,
        });
        if let Some(v) = c.valore {
            s.stat.aggiungi(v);
        }
        if s.punti.len() > MAX_PUNTI {
            s.punti.drain(..MAX_PUNTI / 4);
        }
        self.t_max = self.t_max.max(c.t);
    }

    /// Riparte da zero con minimo, massimo e media di ogni serie.
    pub fn azzera_statistiche(&mut self) {
        for s in self.serie.values_mut() {
            s.stat = Statistiche::default();
        }
    }

    pub fn da_sessione(s: &SessionData) -> Self {
        let mut a = Archivio::default();
        for c in &s.campioni {
            a.aggiungi(c);
        }
        a.marcatori = s.marcatori.clone();
        a
    }

    pub fn ultimo(&self, key: &str) -> Option<&Punto> {
        self.serie.get(key)?.punti.last()
    }

    /// Frequenza effettiva (Hz) dei campioni validi negli ultimi 5 secondi.
    pub fn frequenza(&self, key: &str) -> Option<f64> {
        let punti = &self.serie.get(key)?.punti;
        let da = self.t_max - 5.0;
        let inizio = punti.partition_point(|p| p.t < da);
        let validi: Vec<f64> = punti[inizio..]
            .iter()
            .filter(|p| p.v.is_some())
            .map(|p| p.t)
            .collect();
        let (primo, ultimo) = (*validi.first()?, *validi.last()?);
        if validi.len() < 2 || ultimo <= primo {
            return None;
        }
        Some((validi.len() - 1) as f64 / (ultimo - primo))
    }

    /// PID presenti nell'archivio (per scegliere i segnali di una sessione riaperta).
    pub fn pid_presenti(&self) -> BTreeSet<u8> {
        self.serie
            .keys()
            .filter_map(|k| crate::obd::pids::per_key(k))
            .filter(|p| !p.derivato)
            .map(|p| p.pid)
            .collect()
    }

    /// Spezza la serie in tratti continui a partire dal tempo `da`.
    pub fn tracciato(&self, key: &str, da: f64) -> Tracciato {
        let mut out = Tracciato::default();
        let Some(s) = self.serie.get(key) else {
            return out;
        };
        let inizio = s.punti.partition_point(|p| p.t < da);
        // Un punto prima della finestra, per non far iniziare la linea a metà.
        let inizio = inizio.saturating_sub(1);
        let mut tratto: Vec<[f64; 2]> = Vec::new();
        let mut ultimo_valido: Option<f64> = None;
        let mut buco_da: Option<f64> = None;
        let mut ultimo_t = da;
        for p in &s.punti[inizio..] {
            ultimo_t = p.t;
            match p.v {
                Some(v) => {
                    if let Some(a) = buco_da.take() {
                        out.buchi.push((a, p.t));
                    } else if let Some(prima) = ultimo_valido
                        && p.t - prima > SOGLIA_BUCO_S
                    {
                        out.buchi.push((prima, p.t));
                        if !tratto.is_empty() {
                            out.tratti.push(std::mem::take(&mut tratto));
                        }
                    }
                    tratto.push([p.t, v]);
                    ultimo_valido = Some(p.t);
                }
                None => {
                    if buco_da.is_none() {
                        buco_da = Some(ultimo_valido.unwrap_or(p.t));
                        if !tratto.is_empty() {
                            out.tratti.push(std::mem::take(&mut tratto));
                        }
                    }
                }
            }
        }
        if let Some(a) = buco_da {
            out.buchi.push((a, ultimo_t));
        }
        if !tratto.is_empty() {
            out.tratti.push(tratto);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(t: f64, key: &str, ecu: &str, v: Option<f64>) -> Sample {
        Sample {
            t,
            ts: String::new(),
            ecu: ecu.into(),
            key: key.into(),
            pid: 0x0C,
            valore: v,
            unita: "giri/min".into(),
            stato: if v.is_some() {
                Stato::Ok
            } else {
                Stato::RispostaMancante
            },
        }
    }

    #[test]
    fn buchi_per_letture_mancanti_e_per_silenzio() {
        let mut a = Archivio::default();
        for (t, v) in [
            (0.0, Some(1.0)),
            (0.5, Some(2.0)),
            (1.0, None),
            (1.5, None),
            (2.0, Some(3.0)),
            (2.5, Some(4.0)),
            // Silenzio di 5 secondi senza alcun campione.
            (7.5, Some(5.0)),
        ] {
            a.aggiungi(&c(t, "giri", if v.is_some() { "7E8" } else { "" }, v));
        }
        let tr = a.tracciato("giri", 0.0);
        assert_eq!(tr.tratti.len(), 3);
        assert_eq!(tr.tratti[0], vec![[0.0, 1.0], [0.5, 2.0]]);
        assert_eq!(tr.tratti[1], vec![[2.0, 3.0], [2.5, 4.0]]);
        assert_eq!(tr.buchi, vec![(0.5, 2.0), (2.5, 7.5)]);
    }

    #[test]
    fn statistiche_solo_sui_valori_validi() {
        let mut a = Archivio::default();
        assert_eq!(a.serie.get("giri").map(|s| s.stat.media()), None);
        for (t, v) in [
            (0.0, Some(10.0)),
            (0.5, None),
            (1.0, Some(30.0)),
            (1.5, Some(20.0)),
        ] {
            a.aggiungi(&c(t, "giri", if v.is_some() { "7E8" } else { "" }, v));
        }
        let st = a.serie["giri"].stat;
        assert_eq!((st.min, st.max, st.conteggio), (Some(10.0), Some(30.0), 3));
        assert_eq!(st.media(), Some(20.0));
        // Una serie con soli errori non ha statistiche (mai zero).
        a.aggiungi(&c(0.0, "map", "", None));
        let vuota = a.serie["map"].stat;
        assert_eq!((vuota.min, vuota.max, vuota.media()), (None, None, None));

        a.azzera_statistiche();
        assert_eq!(a.serie["giri"].stat, Statistiche::default());
        assert_eq!(a.serie["giri"].punti.len(), 4);
        a.aggiungi(&c(2.0, "giri", "7E8", Some(5.0)));
        assert_eq!(a.serie["giri"].stat.min, Some(5.0));
        assert_eq!(a.serie["giri"].stat.media(), Some(5.0));
    }

    #[test]
    fn buco_ancora_aperto_alla_fine() {
        let mut a = Archivio::default();
        a.aggiungi(&c(0.0, "giri", "7E8", Some(1.0)));
        a.aggiungi(&c(1.0, "giri", "", None));
        a.aggiungi(&c(2.0, "giri", "", None));
        let tr = a.tracciato("giri", 0.0);
        assert_eq!(tr.buchi, vec![(0.0, 2.0)]);
        assert_eq!(a.ultimo("giri").unwrap().v, None);
    }

    #[test]
    fn seconda_centralina_non_mescolata_e_frequenza() {
        let mut a = Archivio::default();
        for i in 0..11 {
            a.aggiungi(&c(i as f64 * 0.1, "giri", "7E8", Some(900.0)));
            a.aggiungi(&c(i as f64 * 0.1, "giri", "7E9", Some(0.0)));
        }
        assert_eq!(a.serie["giri"].punti.len(), 11);
        assert!((a.frequenza("giri").unwrap() - 10.0).abs() < 1e-6);
        assert_eq!(a.frequenza("velocita"), None);
    }
}
