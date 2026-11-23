//! Decodifica dei dati in tempo reale (servizio 21h). Tutti i valori sono
//! big-endian (ricerca/td5/proto.md, correzione del 2026-09).
//!
//! Le chiavi sono quelle del catalogo dei parametri di OBDio
//! ([`crate::obd::pids`]): dove il significato coincide si usa la chiave OBD-II
//! già esistente (giri, velocità, temperature, pressioni, tensione), così i
//! valori passano per cruscotto, grafici, registrazione, MQTT e MCP come
//! quelli di una centralina OBD-II. I valori solo del Td5 hanno chiavi
//! `td5_…`.
//!
//! Scelte sulle fonti, che non sono d'accordo fra loro (docs/RICERCA_TD5.md):
//! - 1A: temperatura del liquido ai byte 0–1 (tutte le fonti), dell'aria
//!   aspirata ai byte 4–5 e del gasolio ai byte 12–13, in kelvin × 10, come
//!   in Td5OpenDiag (ricerca/td5/ConnectFragment.java). I due valori
//!   coincidono con quelli dello strumento citati in proto.md (23,2 °C e
//!   64,0 °C nella stessa trama). La «pressione del collettore» che proto.md
//!   legge ai byte 2–3 vale 6,98 kPa a motore spento, fisicamente impossibile:
//!   non viene mostrata. La temperatura dei byte 8–9 (aria esterna, solo su
//!   alcune versioni secondo Td5OpenDiag) e gli altri byte restano fuori.
//! - 1C: pressione del collettore ai byte 0–1, /100 kPa, come in Td5OpenDiag
//!   (102,17 kPa a motore spento nell'esempio di proto.md: plausibile). La
//!   portata d'aria dei byte 4–5 non è confermata (proto.md la dice ignota) e
//!   resta fuori, come gli altri byte.
//! - 1B: con 10 byte di dati quattro tracce e alimentazione (proto.md), con 8
//!   byte tre tracce e alimentazione (Td5OpenDiag).
//! - 0D: un solo byte, km/h (proto.md e Td5OpenDiag; pid.txt dice altro).

use super::richieste::IdLocale;
use crate::model::Stato;

pub const CHIAVE_TEMP_GASOLIO: &str = "td5_temp_gasolio";
pub const CHIAVI_PEDALE: [&str; 4] = [
    "td5_pedale_1",
    "td5_pedale_2",
    "td5_pedale_3",
    "td5_pedale_4",
];
pub const CHIAVE_ALIMENTAZIONE: &str = "td5_alimentazione_sensori";
pub const CHIAVE_EGR: &str = "td5_egr";
pub const CHIAVE_WASTEGATE: &str = "td5_wastegate";
pub const CHIAVI_INIETTORI: [&str; 5] = [
    "td5_iniettore_1",
    "td5_iniettore_2",
    "td5_iniettore_3",
    "td5_iniettore_4",
    "td5_iniettore_5",
];
/// Interruttori della 1E: (chiave, byte, bit, attivo basso).
pub const INTERRUTTORI: [(&str, usize, u8, bool); 9] = [
    ("td5_freno_secondario", 0, 0, true),
    ("td5_frizione", 0, 1, true),
    ("td5_cruise_principale", 0, 2, true),
    ("td5_cruise_set", 0, 3, true),
    ("td5_cruise_resume", 0, 4, true),
    ("td5_ac_ventola", 1, 2, true),
    ("td5_ac_compressore", 1, 3, true),
    // 1 = marce ridotte (LOW), non attivo basso.
    ("td5_riduttore_ridotte", 1, 6, false),
    ("td5_freno_principale", 1, 7, true),
];

/// Un valore decodificato. Se `stato` non è `Ok`, `valore` è `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct Valore {
    pub chiave: &'static str,
    pub valore: Option<f64>,
    pub stato: Stato,
}

fn ok(chiave: &'static str, v: f64) -> Valore {
    Valore {
        chiave,
        valore: Some(v),
        stato: Stato::Ok,
    }
}

fn senza(chiave: &'static str, stato: Stato) -> Valore {
    Valore {
        chiave,
        valore: None,
        stato,
    }
}

/// Chiavi prodotte da un identificativo.
pub fn chiavi(id: IdLocale) -> Vec<&'static str> {
    match id {
        IdLocale::Giri => vec!["giri"],
        IdLocale::Velocita => vec!["velocita"],
        IdLocale::Batteria => vec!["tensione_modulo"],
        IdLocale::Temperature => vec!["temp_refrigerante", "temp_aspirazione", CHIAVE_TEMP_GASOLIO],
        IdLocale::Pedale => {
            let mut v = CHIAVI_PEDALE.to_vec();
            v.push(CHIAVE_ALIMENTAZIONE);
            v
        }
        IdLocale::Collettore => vec!["map"],
        IdLocale::Interruttori => INTERRUTTORI.iter().map(|i| i.0).collect(),
        IdLocale::PressioneAmbiente => vec!["pressione_barometrica"],
        IdLocale::Egr => vec![CHIAVE_EGR],
        IdLocale::Wastegate => vec![CHIAVE_WASTEGATE],
        IdLocale::Iniettori => CHIAVI_INIETTORI.to_vec(),
    }
}

/// Identificativo che produce la chiave, se è una chiave del Td5.
pub fn id_di(chiave: &str) -> Option<IdLocale> {
    IdLocale::TUTTI
        .into_iter()
        .find(|id| chiavi(*id).contains(&chiave))
}

/// Tutte le chiavi dell'identificativo con lo stesso stato (risposta
/// mancante, rifiuto, errore).
pub fn con_stato(id: IdLocale, stato: Stato) -> Vec<Valore> {
    chiavi(id).into_iter().map(|k| senza(k, stato)).collect()
}

fn parola(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]))
}

fn kelvin(d: &[u8], i: usize) -> Option<f64> {
    parola(d, i).map(|w| (w as f64 - 2732.0) / 10.0)
}

/// Decodifica il corpo di una risposta positiva (`61 <id> <dati…>`). Una
/// risposta non pertinente o troppo corta dà «errore di comunicazione» per
/// ogni chiave che non si può leggere: mai uno zero.
pub fn decodifica(id: IdLocale, corpo: &[u8]) -> Vec<Valore> {
    let d = match corpo {
        [0x61, x, dati @ ..] if *x == id.codice() => dati,
        _ => return con_stato(id, Stato::ErroreComunicazione),
    };
    let campo = |chiave: &'static str, v: Option<f64>| match v {
        Some(v) => ok(chiave, v),
        None => senza(chiave, Stato::ErroreComunicazione),
    };
    match id {
        IdLocale::Giri => vec![campo("giri", parola(d, 0).map(f64::from))],
        IdLocale::Velocita => vec![campo("velocita", d.first().map(|b| *b as f64))],
        IdLocale::Batteria => vec![campo(
            "tensione_modulo",
            parola(d, 0).map(|w| w as f64 / 1000.0),
        )],
        IdLocale::Temperature => vec![
            campo("temp_refrigerante", kelvin(d, 0)),
            campo("temp_aspirazione", kelvin(d, 4)),
            campo(CHIAVE_TEMP_GASOLIO, kelvin(d, 12)),
        ],
        IdLocale::Pedale => {
            let volt = |i| parola(d, i).map(|w| w as f64 / 1000.0);
            match d.len() {
                n if n >= 10 => {
                    let mut v: Vec<Valore> = CHIAVI_PEDALE
                        .iter()
                        .enumerate()
                        .map(|(i, k)| campo(k, volt(2 * i)))
                        .collect();
                    v.push(campo(CHIAVE_ALIMENTAZIONE, volt(8)));
                    v
                }
                8 | 9 => {
                    let mut v: Vec<Valore> = CHIAVI_PEDALE[..3]
                        .iter()
                        .enumerate()
                        .map(|(i, k)| campo(k, volt(2 * i)))
                        .collect();
                    // Questa versione non ha la quarta traccia.
                    v.push(senza(CHIAVI_PEDALE[3], Stato::NonSupportato));
                    v.push(campo(CHIAVE_ALIMENTAZIONE, volt(6)));
                    v
                }
                _ => con_stato(id, Stato::ErroreComunicazione),
            }
        }
        IdLocale::Collettore => vec![campo("map", parola(d, 0).map(|w| w as f64 / 100.0))],
        IdLocale::Interruttori => INTERRUTTORI
            .iter()
            .map(|(k, byte, bit, attivo_basso)| {
                let v = d.get(*byte).map(|b| {
                    let alto = b >> bit & 1 == 1;
                    if alto != *attivo_basso { 1.0 } else { 0.0 }
                });
                campo(k, v)
            })
            .collect(),
        IdLocale::PressioneAmbiente => vec![campo(
            "pressione_barometrica",
            parola(d, 0).map(|w| w as f64 / 100.0),
        )],
        IdLocale::Egr => vec![campo(CHIAVE_EGR, parola(d, 0).map(|w| w as f64 / 100.0))],
        IdLocale::Wastegate => vec![campo(
            CHIAVE_WASTEGATE,
            parola(d, 0).map(|w| w as f64 / 100.0),
        )],
        IdLocale::Iniettori => CHIAVI_INIETTORI
            .iter()
            .enumerate()
            .map(|(i, k)| campo(k, parola(d, 2 * i).map(|w| w as i16 as f64)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::td5::frame::{da_esadecimale, verifica};

    /// Corpo di una trama d'esempio di proto.md (checksum compreso).
    fn corpo(trama: &str) -> Vec<u8> {
        let t = da_esadecimale(trama).unwrap();
        verifica(&t).unwrap().to_vec()
    }

    fn valore(v: &[Valore], k: &str) -> f64 {
        let x = v.iter().find(|x| x.chiave == k).unwrap();
        assert_eq!(x.stato, Stato::Ok, "{k}");
        x.valore.unwrap()
    }

    fn vicino(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn giri_dall_esempio() {
        // proto.md: 04 61 09 02 E8 (checksum corretto 58): 744 giri/min.
        let v = decodifica(IdLocale::Giri, &corpo("04 61 09 02 E8 58"));
        assert_eq!(valore(&v, "giri"), 744.0);
    }

    #[test]
    fn temperature_dall_esempio() {
        let v = decodifica(
            IdLocale::Temperature,
            &corpo("12 61 1A 0D 6E 02 BA 0B 94 0A B5 0B 74 0B 61 0D 2C 04 69 B3"),
        );
        // 0D6E = 3438 → 70,6 °C; 0B94 → 23,2 °C; 0D2C → 64,0 °C.
        assert!(vicino(valore(&v, "temp_refrigerante"), 70.6));
        assert!(vicino(valore(&v, "temp_aspirazione"), 23.2));
        assert!(vicino(valore(&v, CHIAVE_TEMP_GASOLIO), 64.0));
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn pedale_a_quattro_e_tre_tracce() {
        // proto.md con il checksum corretto (6B).
        let v = decodifica(
            IdLocale::Pedale,
            &corpo("0C 61 1B 02 81 11 44 11 5F 00 00 13 88 6B"),
        );
        assert!(vicino(valore(&v, "td5_pedale_1"), 0.641));
        assert!(vicino(valore(&v, "td5_pedale_2"), 4.420));
        assert!(vicino(valore(&v, "td5_pedale_3"), 4.447));
        assert!(vicino(valore(&v, "td5_pedale_4"), 0.0));
        assert!(vicino(valore(&v, CHIAVE_ALIMENTAZIONE), 5.0));
        // Td5OpenDiag: 8 byte, tre tracce. La quarta è «non supportata», non zero.
        let v = decodifica(
            IdLocale::Pedale,
            &[0x61, 0x1B, 0x02, 0x81, 0x11, 0x44, 0x11, 0x5F, 0x13, 0x88],
        );
        assert!(vicino(valore(&v, CHIAVE_ALIMENTAZIONE), 5.0));
        let quarta = v.iter().find(|x| x.chiave == "td5_pedale_4").unwrap();
        assert_eq!((quarta.valore, quarta.stato), (None, Stato::NonSupportato));
    }

    #[test]
    fn collettore_dall_esempio() {
        // proto.md con il checksum corretto (11): 27E9 = 10217 → 102,17 kPa.
        let v = decodifica(
            IdLocale::Collettore,
            &corpo("0A 61 1C 27 E9 28 06 00 2C 00 20 11"),
        );
        assert!(vicino(valore(&v, "map"), 102.17));
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn batteria_pressione_iniettori() {
        let v = decodifica(IdLocale::Batteria, &corpo("06 61 10 36 43 36 39 5F"));
        assert!(vicino(valore(&v, "tensione_modulo"), 13.891));
        let v = decodifica(IdLocale::PressioneAmbiente, &[0x61, 0x23, 0x26, 0xDB]);
        assert!(vicino(valore(&v, "pressione_barometrica"), 99.47));
        let v = decodifica(
            IdLocale::Iniettori,
            &corpo("0C 61 40 00 02 00 00 FF FD 00 04 FF FD AB"),
        );
        let trim: Vec<f64> = CHIAVI_INIETTORI.iter().map(|k| valore(&v, k)).collect();
        assert_eq!(trim, [2.0, 0.0, -3.0, 4.0, -3.0]);
        let v = decodifica(IdLocale::Egr, &[0x61, 0x37, 0x13, 0x88]);
        assert!(vicino(valore(&v, CHIAVE_EGR), 50.0));
        let v = decodifica(IdLocale::Velocita, &[0x61, 0x0D, 0x45]);
        assert_eq!(valore(&v, "velocita"), 69.0);
    }

    #[test]
    fn interruttori_attivi_bassi() {
        // A riposo tutti alti: niente premuto, riduttore in marce normali (0).
        let v = decodifica(IdLocale::Interruttori, &[0x61, 0x1E, 0xFF, 0xBF]);
        for (k, ..) in INTERRUTTORI {
            assert_eq!(valore(&v, k), 0.0, "{k}");
        }
        // Freno principale (DB2 bit 7) e frizione (DB1 bit 1) premuti, ridotte.
        let v = decodifica(IdLocale::Interruttori, &[0x61, 0x1E, 0xFD, 0x7F]);
        assert_eq!(valore(&v, "td5_freno_principale"), 1.0);
        assert_eq!(valore(&v, "td5_frizione"), 1.0);
        assert_eq!(valore(&v, "td5_riduttore_ridotte"), 1.0);
        assert_eq!(valore(&v, "td5_freno_secondario"), 0.0);
    }

    #[test]
    fn risposte_corte_o_estranee_mai_zero() {
        let v = decodifica(IdLocale::Temperature, &[0x61, 0x1A, 0x0D, 0x6E, 0x02]);
        assert!(vicino(valore(&v, "temp_refrigerante"), 70.6));
        for k in ["temp_aspirazione", CHIAVE_TEMP_GASOLIO] {
            let x = v.iter().find(|x| x.chiave == k).unwrap();
            assert_eq!((x.valore, x.stato), (None, Stato::ErroreComunicazione));
        }
        // Risposta a un altro identificativo.
        let v = decodifica(IdLocale::Giri, &[0x61, 0x0D, 0x00]);
        assert_eq!(v[0].stato, Stato::ErroreComunicazione);
        assert!(v.iter().all(|x| x.valore.is_none()));
        let v = decodifica(IdLocale::Pedale, &[0x61, 0x1B, 0x00, 0x01]);
        assert!(v.iter().all(|x| x.valore.is_none()));
    }

    #[test]
    fn chiavi_e_identificativi() {
        assert_eq!(id_di("giri"), Some(IdLocale::Giri));
        assert_eq!(id_di("td5_frizione"), Some(IdLocale::Interruttori));
        assert_eq!(id_di("carico"), None);
        for id in IdLocale::TUTTI {
            for k in chiavi(id) {
                assert!(crate::obd::pids::per_key(k).is_some(), "{k} fuori catalogo");
            }
        }
    }
}
