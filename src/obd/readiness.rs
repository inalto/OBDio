//! PID 01: spia MIL, numero di DTC e monitor di prontezza.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monitor {
    pub nome: String,
    pub disponibile: bool,
    /// Significativo solo se `disponibile`.
    pub completato: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    pub ecu: String,
    pub mil_accesa: bool,
    pub numero_dtc: u8,
    /// Vero per motori ad accensione spontanea (diesel).
    pub diesel: bool,
    pub monitor: Vec<Monitor>,
}

const BENZINA: [&str; 8] = [
    "Catalizzatore",
    "Catalizzatore riscaldato",
    "Sistema evaporativo",
    "Aria secondaria",
    "Riservato (bit 4)",
    "Sonde lambda",
    "Riscaldatore sonde lambda",
    "EGR / variatore di fase",
];

const DIESEL: [&str; 8] = [
    "Catalizzatore NMHC",
    "Post-trattamento NOx / SCR",
    "Riservato (bit 2)",
    "Pressione di sovralimentazione",
    "Riservato (bit 4)",
    "Sensore gas di scarico",
    "Filtro antiparticolato",
    "EGR / variatore di fase",
];

/// Decodifica i quattro byte A–D del PID 01.
pub fn decode(ecu: &str, d: &[u8]) -> Option<Readiness> {
    if d.len() < 4 {
        return None;
    }
    let (a, b, c, dd) = (d[0], d[1], d[2], d[3]);
    let diesel = b & 0x08 != 0;
    let mut monitor = Vec::new();
    for (i, nome) in ["Mancate accensioni", "Sistema carburante", "Componenti"]
        .iter()
        .enumerate()
    {
        monitor.push(Monitor {
            nome: nome.to_string(),
            disponibile: b & (1 << i) != 0,
            completato: b & (1 << (i + 4)) == 0,
        });
    }
    let nomi = if diesel { DIESEL } else { BENZINA };
    for (i, nome) in nomi.iter().enumerate() {
        if nome.starts_with("Riservato") {
            continue;
        }
        monitor.push(Monitor {
            nome: nome.to_string(),
            disponibile: c & (1 << i) != 0,
            completato: dd & (1 << i) == 0,
        });
    }
    Some(Readiness {
        ecu: ecu.to_string(),
        mil_accesa: a & 0x80 != 0,
        numero_dtc: a & 0x7F,
        diesel,
        monitor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mil_e_monitor() {
        // MIL accesa, 1 DTC; benzina; catalizzatore disponibile e NON completato.
        let r = decode("7E8", &[0x81, 0x07, 0x65, 0x01]).unwrap();
        assert!(r.mil_accesa);
        assert_eq!(r.numero_dtc, 1);
        assert!(!r.diesel);
        let cat = r
            .monitor
            .iter()
            .find(|m| m.nome == "Catalizzatore")
            .unwrap();
        assert!(cat.disponibile && !cat.completato);
        let o2 = r.monitor.iter().find(|m| m.nome == "Sonde lambda").unwrap();
        assert!(o2.disponibile && o2.completato);
        let aria = r
            .monitor
            .iter()
            .find(|m| m.nome == "Aria secondaria")
            .unwrap();
        assert!(!aria.disponibile);
    }

    #[test]
    fn mil_spenta() {
        let r = decode("7E8", &[0x00, 0x07, 0x00, 0x00]).unwrap();
        assert!(!r.mil_accesa);
        assert_eq!(r.numero_dtc, 0);
        assert!(decode("7E8", &[0x00, 0x07]).is_none());
    }
}
