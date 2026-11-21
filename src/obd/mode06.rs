//! Servizio 06 su CAN: risultati dei monitor di bordo.
//!
//! Struttura della risposta (dopo il byte 0x46): blocchi di 9 byte
//! `MID, TID, UASID, valore(2), minimo(2), massimo(2)`.
//!
//! La tabella di scala qui sotto è un sottoinsieme della tabella «Unit and
//! Scaling ID» come riportata dal progetto python-OBD (obd/UnitsAndScaling.py).
//! Non è stata confrontata con il testo SAE J1979: per questo l'interfaccia
//! mostra sempre anche i valori grezzi, e per gli identificativi fuori tabella
//! mostra solo quelli.

use serde::{Deserialize, Serialize};

/// MID del monitor del catalizzatore, bancata 1.
pub const MID_CATALIZZATORE_B1: u8 = 0x21;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scalato {
    pub valore: f64,
    pub minimo: f64,
    pub massimo: f64,
    pub unita: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TestMonitor {
    pub ecu: String,
    pub mid: u8,
    pub tid: u8,
    pub uas_id: u8,
    pub grezzo_valore: u16,
    pub grezzo_minimo: u16,
    pub grezzo_massimo: u16,
    /// Presente solo se l'identificativo di scala è in tabella.
    pub scalato: Option<Scalato>,
}

/// (scala per bit, scostamento, unità) per gli identificativi in tabella.
/// Gli identificativi da 0x80 in su sono con segno (complemento a due).
fn scala(uas: u8) -> Option<(f64, f64, &'static str)> {
    Some(match uas {
        0x01 | 0x24 | 0x2B | 0x81 => (1.0, 0.0, "conteggio"),
        0x02 | 0x82 => (0.1, 0.0, "conteggio"),
        0x03 | 0x83 => (0.01, 0.0, "conteggio"),
        0x04 | 0x84 => (0.001, 0.0, "conteggio"),
        0x05 | 0x85 => (0.0000305, 0.0, "conteggio"),
        0x06 | 0x86 => (0.000305, 0.0, "conteggio"),
        0x07 => (0.25, 0.0, "giri/min"),
        0x08 => (0.01, 0.0, "km/h"),
        0x09 => (1.0, 0.0, "km/h"),
        0x0A | 0x8A => (0.122, 0.0, "mV"),
        0x0B | 0x8B => (0.001, 0.0, "V"),
        0x0C | 0x8C => (0.01, 0.0, "V"),
        0x0D | 0x8D => (0.00390625, 0.0, "mA"),
        0x0E | 0x8E => (0.001, 0.0, "A"),
        0x0F => (0.01, 0.0, "A"),
        0x10 | 0x90 => (1.0, 0.0, "ms"),
        0x11 => (100.0, 0.0, "ms"),
        0x12 => (1.0, 0.0, "s"),
        0x13 => (1.0, 0.0, "mΩ"),
        0x14 => (1.0, 0.0, "Ω"),
        0x15 => (1.0, 0.0, "kΩ"),
        0x16 => (0.1, -40.0, "°C"),
        0x96 => (0.1, 0.0, "°C"),
        0x17 => (0.01, 0.0, "kPa"),
        0x1A => (1.0, 0.0, "kPa"),
        0x1B => (10.0, 0.0, "kPa"),
        0x1C | 0x9C => (0.01, 0.0, "°"),
        0x1D | 0x9D => (0.5, 0.0, "°"),
        0x1E => (0.0000305, 0.0, "rapporto"),
        0x1F => (0.05, 0.0, "rapporto"),
        0x20 => (0.00390625, 0.0, "rapporto"),
        0x21 => (1.0, 0.0, "mHz"),
        0x22 => (1.0, 0.0, "Hz"),
        0x23 => (1.0, 0.0, "kHz"),
        0x25 => (1.0, 0.0, "km"),
        0x2F | 0xAF => (0.01, 0.0, "%"),
        0x34 => (1.0, 0.0, "min"),
        0x35 => (10.0, 0.0, "ms"),
        _ => return None,
    })
}

fn applica(uas: u8, grezzo: u16) -> Option<(f64, &'static str)> {
    let (k, off, unita) = scala(uas)?;
    let n = if uas >= 0x80 {
        grezzo as i16 as f64
    } else {
        grezzo as f64
    };
    Some((n * k + off, unita))
}

/// Decodifica un messaggio completo del servizio 06 (`data[0] == 0x46`).
pub fn parse(ecu: &str, data: &[u8]) -> Vec<TestMonitor> {
    let mut out = Vec::new();
    if data.first() != Some(&0x46) {
        return out;
    }
    for b in data[1..].chunks_exact(9) {
        let w = |i: usize| ((b[i] as u16) << 8) | b[i + 1] as u16;
        let (uas, v, mn, mx) = (b[2], w(3), w(5), w(7));
        let scalato = match (applica(uas, v), applica(uas, mn), applica(uas, mx)) {
            (Some((valore, unita)), Some((minimo, _)), Some((massimo, _))) => Some(Scalato {
                valore,
                minimo,
                massimo,
                unita: unita.to_string(),
            }),
            _ => None,
        };
        out.push(TestMonitor {
            ecu: ecu.to_string(),
            mid: b[0],
            tid: b[1],
            uas_id: uas,
            grezzo_valore: v,
            grezzo_minimo: mn,
            grezzo_massimo: mx,
            scalato,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_con_scala_nota() {
        let d = [0x46, 0x21, 0x80, 0x20, 0x00, 0x90, 0x00, 0x00, 0x00, 0x80];
        let t = parse("7E8", &d);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].mid, t[0].tid, t[0].uas_id), (0x21, 0x80, 0x20));
        assert_eq!(t[0].grezzo_valore, 0x0090);
        let s = t[0].scalato.as_ref().unwrap();
        assert!((s.valore - 0.5625).abs() < 1e-9);
        assert!((s.massimo - 0.5).abs() < 1e-9);
        assert_eq!(s.unita, "rapporto");
    }

    #[test]
    fn scala_sconosciuta_resta_grezza() {
        let d = [0x46, 0x21, 0x81, 0x7E, 0x12, 0x34, 0x00, 0x00, 0xFF, 0xFF];
        let t = parse("7E8", &d);
        assert_eq!(t[0].grezzo_valore, 0x1234);
        assert!(t[0].scalato.is_none());
    }

    #[test]
    fn con_segno_e_blocchi_multipli() {
        let d = [
            0x46, 0x21, 0x01, 0x8B, 0xFF, 0x38, 0xFC, 0x18, 0x03, 0xE8, 0x21, 0x02, 0x10, 0x00,
            0x64, 0x00, 0x00, 0x01, 0xF4, 0xAA,
        ];
        let t = parse("7E8", &d);
        assert_eq!(t.len(), 2);
        assert!((t[0].scalato.as_ref().unwrap().valore + 0.2).abs() < 1e-9);
        assert_eq!(t[1].scalato.as_ref().unwrap().unita, "ms");
        assert_eq!(t[1].scalato.as_ref().unwrap().valore, 100.0);
    }
}
