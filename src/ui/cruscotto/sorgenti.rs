//! Da dove prende il dato ogni strumento: risoluzione delle chiavi speciali,
//! supporto della centralina, stato del valore e chiavi da interrogare.
//! Solo logica pura, verificata dai test.

use std::collections::{BTreeMap, BTreeSet};

use super::modello::{SONDA_A_MONTE, SONDA_A_VALLE, SPIA_MIL, SPIA_REGISTRAZIONE, TipoStrumento};
use crate::model::Stato;
use crate::obd::pids::{self, PresetP0420};
use crate::ui::dati::Punto;
use crate::ui::sensori;

/// Oltre questo tempo senza campioni il valore è «vecchio».
pub const SOGLIA_VECCHIO_S: f64 = 3.0;

/// Chiave del contachilometri della centralina (PID A6).
pub const CHIAVE_CONTACHILOMETRI: &str = "contachilometri";

/// Sorgente di una spia che non legge un parametro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SorgenteSpia {
    Mil,
    Registrazione,
}

/// Esito della risoluzione della chiave di uno strumento.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Risolta {
    /// Parametro del catalogo; `parziale` se il contachilometri è sostituito
    /// dalla distanza parziale calcolata.
    Parametro {
        chiave: String,
        parziale: bool,
    },
    Spia(SorgenteSpia),
    /// Sorgente speciale che la centralina collegata non offre (per esempio
    /// nessuna sonda a valle).
    Assente,
    /// Chiave che l'app non conosce (file di un'altra versione).
    Sconosciuta,
}

impl Risolta {
    pub fn chiave(&self) -> Option<&str> {
        match self {
            Risolta::Parametro { chiave, .. } => Some(chiave),
            _ => None,
        }
    }
}

/// Supporto di un parametro: `None` se non si sa (nessuna centralina
/// collegata), altrimenti se la centralina lo dichiara. I parametri calcolati
/// sono supportati se lo è la loro sorgente.
pub fn supportato(chiave: &str, unione: Option<&BTreeSet<u8>>) -> Option<bool> {
    let Some(p) = pids::per_key(chiave) else {
        return Some(false);
    };
    let unione = unione?;
    let pid = if p.derivato {
        match pids::sorgente_derivato(chiave).and_then(pids::per_key) {
            Some(s) => s.pid,
            None => return Some(false),
        }
    } else {
        p.pid
    };
    Some(unione.contains(&pid))
}

/// Risolve la chiave di uno strumento sulla centralina corrente.
///
/// `ha_dati` dice se l'archivio dal vivo contiene la chiave: serve quando non
/// c'è una centralina collegata per decidere fra contachilometri e parziale.
pub fn risolvi(
    chiave: &str,
    tipo: TipoStrumento,
    unione: Option<&BTreeSet<u8>>,
    preset: &PresetP0420,
    ha_dati: impl Fn(&str) -> bool,
) -> Risolta {
    let parametro = |k: &str, parziale: bool| Risolta::Parametro {
        chiave: k.to_string(),
        parziale,
    };
    match chiave {
        SPIA_MIL => Risolta::Spia(SorgenteSpia::Mil),
        SPIA_REGISTRAZIONE => Risolta::Spia(SorgenteSpia::Registrazione),
        SONDA_A_MONTE => preset
            .b1s1
            .as_deref()
            .map_or(Risolta::Assente, |k| parametro(k, false)),
        SONDA_A_VALLE => preset
            .b1s2
            .as_deref()
            .map_or(Risolta::Assente, |k| parametro(k, false)),
        CHIAVE_CONTACHILOMETRI if tipo == TipoStrumento::Contachilometri => {
            let reale = match unione {
                Some(_) => supportato(CHIAVE_CONTACHILOMETRI, unione) == Some(true),
                None => ha_dati(CHIAVE_CONTACHILOMETRI),
            };
            if reale {
                parametro(CHIAVE_CONTACHILOMETRI, false)
            } else {
                parametro(pids::CHIAVE_DISTANZA_PARZIALE, true)
            }
        }
        pids::CHIAVE_DISTANZA_PARZIALE => parametro(chiave, true),
        _ if pids::per_key(chiave).is_some() => parametro(chiave, false),
        _ => Risolta::Sconosciuta,
    }
}

/// Stato del valore mostrato da uno strumento.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StatoValore {
    /// La centralina non espone il parametro (o l'app non lo conosce).
    NonSupportato,
    /// Ancora nessun campione.
    InAttesa,
    Valido(f64),
    /// Ultimo valore valido arrivato più di [`SOGLIA_VECCHIO_S`] fa.
    Vecchio {
        valore: f64,
        secondi: f64,
    },
    /// L'ultima lettura non ha dato un valore (nessuna risposta, errore).
    Mancante(Stato),
}

impl StatoValore {
    /// Valore da mostrare (anche se vecchio); `None` se il dato manca.
    pub fn valore(&self) -> Option<f64> {
        match *self {
            StatoValore::Valido(v) => Some(v),
            StatoValore::Vecchio { valore, .. } => Some(valore),
            _ => None,
        }
    }
}

/// Classifica lo stato del valore. `eta_s` è il tempo (orologio del PC)
/// dall'arrivo dell'ultimo campione: così un flusso fermo diventa «vecchio».
pub fn classifica(
    supporto: Option<bool>,
    ultimo: Option<&Punto>,
    eta_s: Option<f64>,
) -> StatoValore {
    if supporto == Some(false) {
        return StatoValore::NonSupportato;
    }
    let Some(p) = ultimo else {
        return StatoValore::InAttesa;
    };
    match p.v {
        Some(v) if !v.is_finite() => StatoValore::Mancante(Stato::ErroreComunicazione),
        Some(v) => match eta_s {
            Some(e) if e > SOGLIA_VECCHIO_S => StatoValore::Vecchio {
                valore: v,
                secondi: e,
            },
            _ => StatoValore::Valido(v),
        },
        None if p.stato == Stato::NonSupportato => StatoValore::NonSupportato,
        None if p.stato == Stato::Ok => StatoValore::Mancante(Stato::RispostaMancante),
        None => StatoValore::Mancante(p.stato),
    }
}

/// Chiavi da interrogare con il cruscotto attivo: l'unione della selezione
/// della pagina Sensori e dei parametri (già risolti) della pagina visibile.
///
/// Come nella pagina Sensori, un parametro calcolato fa leggere la sua
/// sorgente e non viene mai richiesto. Le prioritarie sono le sonde del
/// preset, ma solo se compaiono sulla pagina: altrimenti nessuna.
pub fn chiavi_cruscotto(
    selezione: &BTreeMap<String, bool>,
    preset: &PresetP0420,
    chiavi_pagina: &[String],
) -> (Vec<String>, Vec<String>) {
    let (mut chiavi, _) = sensori::chiavi_da_selezione(selezione, preset);
    for k in chiavi_pagina {
        let da_leggere = match pids::per_key(k) {
            Some(p) if p.derivato => match pids::sorgente_derivato(k) {
                Some(s) => s,
                None => continue,
            },
            Some(_) => k.as_str(),
            None => continue,
        };
        if !chiavi.iter().any(|c| c == da_leggere) {
            chiavi.push(da_leggere.to_string());
        }
    }
    let prioritarie = preset
        .sonde()
        .into_iter()
        .filter(|s| chiavi_pagina.iter().any(|k| k == s))
        .collect();
    (chiavi, prioritarie)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset_demo() -> PresetP0420 {
        let s: BTreeSet<u8> = [0x04, 0x05, 0x06, 0x07, 0x0B, 0x0C, 0x0D, 0x15, 0x24]
            .into_iter()
            .collect();
        PresetP0420::calcola(&s)
    }

    fn punto(v: Option<f64>, stato: Stato) -> Punto {
        Punto { t: 1.0, v, stato }
    }

    #[test]
    fn stato_del_valore() {
        assert_eq!(
            classifica(Some(false), None, None),
            StatoValore::NonSupportato
        );
        assert_eq!(classifica(Some(true), None, None), StatoValore::InAttesa);
        assert_eq!(classifica(None, None, None), StatoValore::InAttesa);
        let ok = punto(Some(2128.0), Stato::Ok);
        assert_eq!(
            classifica(Some(true), Some(&ok), Some(0.2)),
            StatoValore::Valido(2128.0)
        );
        assert_eq!(
            classifica(Some(true), Some(&ok), Some(4.5)),
            StatoValore::Vecchio {
                valore: 2128.0,
                secondi: 4.5
            }
        );
        // Esattamente 3 s: ancora valido.
        assert_eq!(
            classifica(Some(true), Some(&ok), Some(3.0)),
            StatoValore::Valido(2128.0)
        );
        let manca = punto(None, Stato::RispostaMancante);
        assert_eq!(
            classifica(Some(true), Some(&manca), Some(0.1)),
            StatoValore::Mancante(Stato::RispostaMancante)
        );
        let errore = punto(None, Stato::ErroreComunicazione);
        assert_eq!(
            classifica(None, Some(&errore), Some(9.0)),
            StatoValore::Mancante(Stato::ErroreComunicazione)
        );
        let ns = punto(None, Stato::NonSupportato);
        assert_eq!(
            classifica(Some(true), Some(&ns), None),
            StatoValore::NonSupportato
        );
        // Mai un valore per un dato mancante.
        assert_eq!(
            StatoValore::Mancante(Stato::RispostaMancante).valore(),
            None
        );
        assert_eq!(StatoValore::InAttesa.valore(), None);
        assert_eq!(StatoValore::NonSupportato.valore(), None);
    }

    #[test]
    fn supporto_e_risoluzione() {
        let unione: BTreeSet<u8> = [0x0C, 0x0D, 0x15, 0x24].into_iter().collect();
        assert_eq!(supportato("giri", Some(&unione)), Some(true));
        assert_eq!(supportato("temp_olio", Some(&unione)), Some(false));
        assert_eq!(supportato("giri", None), None);
        assert_eq!(supportato("ignoto", None), Some(false));
        // La distanza parziale è supportata se lo è la velocità.
        assert_eq!(
            supportato(pids::CHIAVE_DISTANZA_PARZIALE, Some(&unione)),
            Some(true)
        );

        let preset = PresetP0420::calcola(&unione);
        let mai = |_: &str| false;
        let r = |k: &str, t| risolvi(k, t, Some(&unione), &preset, mai);
        assert_eq!(
            r(SONDA_A_MONTE, TipoStrumento::Grafico).chiave(),
            Some("o2s1_lambda")
        );
        assert_eq!(
            r(SONDA_A_VALLE, TipoStrumento::Numerico).chiave(),
            Some("o2s2_tensione")
        );
        assert_eq!(
            r(SPIA_MIL, TipoStrumento::Spia),
            Risolta::Spia(SorgenteSpia::Mil)
        );
        assert_eq!(r("xyz", TipoStrumento::Quadrante), Risolta::Sconosciuta);
        // Senza contachilometri della centralina: parziale calcolato.
        assert_eq!(
            r(CHIAVE_CONTACHILOMETRI, TipoStrumento::Contachilometri),
            Risolta::Parametro {
                chiave: pids::CHIAVE_DISTANZA_PARZIALE.into(),
                parziale: true
            }
        );
        // Con il PID A6 si usa quello vero.
        let mut con_a6 = unione.clone();
        con_a6.insert(0xA6);
        assert_eq!(
            risolvi(
                CHIAVE_CONTACHILOMETRI,
                TipoStrumento::Contachilometri,
                Some(&con_a6),
                &preset,
                mai
            )
            .chiave(),
            Some(CHIAVE_CONTACHILOMETRI)
        );
        // Un numerico sul contachilometri resta sul contachilometri.
        assert_eq!(
            r(CHIAVE_CONTACHILOMETRI, TipoStrumento::Numerico).chiave(),
            Some(CHIAVE_CONTACHILOMETRI)
        );
        // Scollegati: decide l'archivio.
        let vuoto = PresetP0420::default();
        assert_eq!(
            risolvi(SONDA_A_VALLE, TipoStrumento::Grafico, None, &vuoto, mai),
            Risolta::Assente
        );
        assert_eq!(
            risolvi(
                CHIAVE_CONTACHILOMETRI,
                TipoStrumento::Contachilometri,
                None,
                &vuoto,
                |k| k == CHIAVE_CONTACHILOMETRI
            )
            .chiave(),
            Some(CHIAVE_CONTACHILOMETRI)
        );
    }

    #[test]
    fn unione_delle_chiavi_e_priorita() {
        let preset = preset_demo();
        let selezione = BTreeMap::from([
            ("map".to_string(), true),
            ("carico".to_string(), false),
            ("o2s1_lambda".to_string(), true),
        ]);
        // Pagina «Guida»: nessuna sonda, quindi nessuna priorità, anche se la
        // selezione dei Sensori contiene una sonda.
        let guida: Vec<String> = ["giri", "velocita", "giri", pids::CHIAVE_DISTANZA_PARZIALE]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (chiavi, prioritarie) = chiavi_cruscotto(&selezione, &preset, &guida);
        assert_eq!(chiavi, vec!["map", "o2s1_lambda", "giri", "velocita"]);
        assert!(prioritarie.is_empty());

        // Pagina con le sonde: diventano prioritarie.
        let p0420: Vec<String> = ["o2s1_lambda", "o2s2_tensione", "stft_b1", "ignota"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (chiavi, prioritarie) = chiavi_cruscotto(&BTreeMap::new(), &preset, &p0420);
        assert_eq!(chiavi, vec!["o2s1_lambda", "o2s2_tensione", "stft_b1"]);
        assert_eq!(prioritarie, vec!["o2s1_lambda", "o2s2_tensione"]);

        // Pagina vuota e selezione vuota: niente da leggere.
        let (chiavi, prioritarie) = chiavi_cruscotto(&BTreeMap::new(), &preset, &[]);
        assert!(chiavi.is_empty() && prioritarie.is_empty());
    }
}
