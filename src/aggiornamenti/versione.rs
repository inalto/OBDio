//! Numeri di versione nel formato `MAGGIORE.MINORE.CORREZIONE` con eventuale
//! pre-rilascio (`-rc1`, `-beta.2`), confrontati secondo Semantic Versioning
//! 2.0. Un'etichetta che non si legge è «sconosciuta»: non è mai più nuova.

use std::cmp::Ordering;
use std::fmt;

/// Lunghezza massima accettata per un'etichetta: oltre non è una versione.
const LUNGHEZZA_MASSIMA: usize = 64;

/// Una parte del pre-rilascio: numerica o testuale (le numeriche vengono
/// prima, come vuole Semantic Versioning).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Parte {
    Numero(u64),
    Testo(String),
}

impl Ord for Parte {
    fn cmp(&self, altra: &Self) -> Ordering {
        match (self, altra) {
            (Parte::Numero(a), Parte::Numero(b)) => a.cmp(b),
            (Parte::Numero(_), Parte::Testo(_)) => Ordering::Less,
            (Parte::Testo(_), Parte::Numero(_)) => Ordering::Greater,
            (Parte::Testo(a), Parte::Testo(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Parte {
    fn partial_cmp(&self, altra: &Self) -> Option<Ordering> {
        Some(self.cmp(altra))
    }
}

/// Versione letta da un'etichetta come `v0.4.0` o `1.0.0-rc1`. I metadati di
/// compilazione (`+…`) sono accettati e ignorati nel confronto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Versione {
    pub maggiore: u64,
    pub minore: u64,
    pub correzione: u64,
    pre: Vec<Parte>,
}

impl Versione {
    /// Legge un'etichetta, con o senza la `v` iniziale. `None` se non è una
    /// versione valida.
    pub fn leggi(etichetta: &str) -> Option<Self> {
        let t = etichetta.trim();
        if t.is_empty() || t.len() > LUNGHEZZA_MASSIMA {
            return None;
        }
        let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
        // I metadati di compilazione non contano nel confronto, ma devono
        // essere ben formati.
        let (t, metadati) = match t.split_once('+') {
            Some((a, b)) => (a, Some(b)),
            None => (t, None),
        };
        if let Some(m) = metadati
            && !identificatori_validi(m)
        {
            return None;
        }
        let (nucleo, pre) = match t.split_once('-') {
            Some((a, b)) => (a, Some(b)),
            None => (t, None),
        };
        let mut numeri = nucleo.split('.');
        let maggiore = numero(numeri.next()?)?;
        let minore = numero(numeri.next()?)?;
        let correzione = numero(numeri.next()?)?;
        if numeri.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(p) => {
                if !identificatori_validi(p) {
                    return None;
                }
                let mut parti = Vec::new();
                for id in p.split('.') {
                    if id.bytes().all(|b| b.is_ascii_digit()) {
                        parti.push(Parte::Numero(numero(id)?));
                    } else {
                        parti.push(Parte::Testo(id.to_string()));
                    }
                }
                parti
            }
        };
        Some(Versione {
            maggiore,
            minore,
            correzione,
            pre,
        })
    }

    /// Versione di questa build (da `Cargo.toml`).
    pub fn corrente() -> Self {
        Versione::leggi(env!("CARGO_PKG_VERSION")).expect("versione del pacchetto non valida")
    }

    /// Vero per un pre-rilascio (`1.0.0-rc1`): non viene mai proposto.
    pub fn pre_rilascio(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Versione {
    fn cmp(&self, altra: &Self) -> Ordering {
        (self.maggiore, self.minore, self.correzione)
            .cmp(&(altra.maggiore, altra.minore, altra.correzione))
            .then_with(|| match (self.pre.is_empty(), altra.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // Una versione finale viene dopo i suoi pre-rilasci.
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&altra.pre),
            })
    }
}

impl PartialOrd for Versione {
    fn partial_cmp(&self, altra: &Self) -> Option<Ordering> {
        Some(self.cmp(altra))
    }
}

impl fmt::Display for Versione {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.maggiore, self.minore, self.correzione)?;
        for (i, p) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            match p {
                Parte::Numero(n) => write!(f, "{n}")?,
                Parte::Testo(t) => f.write_str(t)?,
            }
        }
        Ok(())
    }
}

/// Numero senza zeri iniziali (salvo «0»), solo cifre.
fn numero(t: &str) -> Option<u64> {
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) || (t.len() > 1 && t.starts_with('0'))
    {
        return None;
    }
    t.parse().ok()
}

/// Identificatori separati da punti, non vuoti, fatti di `[0-9A-Za-z-]`.
fn identificatori_validi(t: &str) -> bool {
    !t.is_empty()
        && t.split('.')
            .all(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

/// Esito del confronto fra la versione installata e quella offerta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confronto {
    /// L'offerta è una versione finale strettamente più nuova.
    PiuNuova,
    /// Uguale o più vecchia: nulla da fare.
    NonPiuNuova,
    /// L'offerta è un pre-rilascio: ignorata.
    PreRilascio,
    /// L'etichetta offerta non si legge: mai trattata come più nuova.
    Sconosciuta,
}

/// Confronta la versione installata con l'etichetta di un rilascio.
pub fn confronta(installata: &Versione, offerta: &str) -> Confronto {
    match Versione::leggi(offerta) {
        None => Confronto::Sconosciuta,
        Some(v) if v.pre_rilascio() => Confronto::PreRilascio,
        Some(v) if v > *installata => Confronto::PiuNuova,
        Some(_) => Confronto::NonPiuNuova,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(t: &str) -> Versione {
        Versione::leggi(t).unwrap_or_else(|| panic!("{t} non letta"))
    }

    #[test]
    fn lettura_con_e_senza_v() {
        assert_eq!(v("v0.4.0"), v("0.4.0"));
        assert_eq!(v("V1.2.3").to_string(), "1.2.3");
        assert_eq!(v(" 1.2.3 ").to_string(), "1.2.3");
        assert_eq!(v("1.0.0-rc1").to_string(), "1.0.0-rc1");
        assert_eq!(v("1.0.0-beta.2").to_string(), "1.0.0-beta.2");
        assert_eq!(v("1.0.0+build.7").to_string(), "1.0.0");
        assert!(Versione::corrente().to_string() == env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn etichette_illeggibili() {
        for t in [
            "",
            "v",
            "latest",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.x",
            "-1.2.3",
            "1.2.3-",
            "1.2.3-rc..1",
            "1.2.3-rc_1",
            "1.2.3+",
            "1.2.3-01",
            "vv1.2.3",
            "1.2.3 beta",
            "99999999999999999999.0.0",
            "１.２.３",
        ] {
            assert!(
                Versione::leggi(t).is_none(),
                "«{t}» non dovrebbe essere letta"
            );
        }
        assert!(Versione::leggi(&format!("1.0.0-{}", "a".repeat(80))).is_none());
    }

    #[test]
    fn ordine_semantico() {
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.3.1") > v("0.3.0"));
        assert!(v("1.0.0") > v("1.0.0-rc1"));
        assert!(v("1.0.0-rc2") > v("1.0.0-rc1"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.9"));
        assert!(v("1.0.0-alpha") < v("1.0.0-alpha.1"));
        assert!(v("1.0.0-alpha.1") < v("1.0.0-alpha.beta"));
        assert!(v("1.0.0-beta.11") < v("1.0.0-rc.1"));
        assert_eq!(v("1.0.0+a").cmp(&v("1.0.0+b")), Ordering::Equal);
    }

    #[test]
    fn confronto_con_l_installata() {
        let installata = v("0.3.0");
        assert_eq!(confronta(&installata, "v0.4.0"), Confronto::PiuNuova);
        assert_eq!(confronta(&installata, "v0.10.0"), Confronto::PiuNuova);
        assert_eq!(confronta(&installata, "v0.3.0"), Confronto::NonPiuNuova);
        assert_eq!(confronta(&installata, "v0.2.9"), Confronto::NonPiuNuova);
        assert_eq!(confronta(&installata, "v0.4.0-rc1"), Confronto::PreRilascio);
        assert_eq!(confronta(&installata, "nightly"), Confronto::Sconosciuta);
        assert_eq!(confronta(&installata, "v9"), Confronto::Sconosciuta);
        // Da un pre-rilascio installato, la versione finale è più nuova.
        assert_eq!(confronta(&v("1.0.0-rc1"), "1.0.0"), Confronto::PiuNuova);
    }
}
