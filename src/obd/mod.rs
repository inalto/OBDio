//! Decodifica OBD-II: trame, PID, codici guasto, prontezza, Mode 06.

pub mod dtc;
pub mod frames;
pub mod mode06;
pub mod pids;
pub mod readiness;

pub use frames::{BusKind, EcuMessage};

/// Richieste OBD di sola lettura. L'elenco è chiuso: da qui non si possono
/// inviare servizi di scrittura o cancellazione (04, 08, 2E, 2F, 31, 3B...).
/// La cancellazione dei codici guasto (04) ha un percorso separato che
/// richiede la conferma dell'utente: `Elm::cancella_dtc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObdRequest {
    /// Servizio 01: dato corrente.
    CurrentData(u8),
    /// Servizio 02: dato del fermo immagine (fotogramma 0).
    FreezeFrame(u8),
    /// Servizio 03: DTC confermati.
    StoredDtcs,
    /// Servizio 07: DTC in sospeso.
    PendingDtcs,
    /// Servizio 0A: DTC permanenti.
    PermanentDtcs,
    /// Servizio 06: risultati dei monitor (solo CAN).
    MonitorResults(u8),
    /// Servizio 09 PID 02: VIN.
    Vin,
}

impl ObdRequest {
    /// Numero del servizio OBD.
    pub fn service(self) -> u8 {
        match self {
            ObdRequest::CurrentData(_) => 0x01,
            ObdRequest::FreezeFrame(_) => 0x02,
            ObdRequest::StoredDtcs => 0x03,
            ObdRequest::MonitorResults(_) => 0x06,
            ObdRequest::PendingDtcs => 0x07,
            ObdRequest::Vin => 0x09,
            ObdRequest::PermanentDtcs => 0x0A,
        }
    }

    /// Testo inviato all'adattatore (senza ritorno a capo).
    pub fn command(self) -> String {
        match self {
            ObdRequest::CurrentData(p) => format!("01{p:02X}"),
            ObdRequest::FreezeFrame(p) => format!("02{p:02X}00"),
            ObdRequest::StoredDtcs => "03".into(),
            ObdRequest::MonitorResults(m) => format!("06{m:02X}"),
            ObdRequest::PendingDtcs => "07".into(),
            ObdRequest::Vin => "0902".into(),
            ObdRequest::PermanentDtcs => "0A".into(),
        }
    }

    /// Vero se `data` è una risposta positiva a questa richiesta.
    pub fn matches(self, data: &[u8]) -> bool {
        if data.first() != Some(&(self.service() + 0x40)) {
            return false;
        }
        match self {
            ObdRequest::CurrentData(p) => data.get(1) == Some(&p),
            ObdRequest::FreezeFrame(p) => data.get(1) == Some(&p),
            ObdRequest::MonitorResults(m) => data.get(1) == Some(&m),
            ObdRequest::Vin => data.get(1) == Some(&0x02),
            _ => true,
        }
    }
}

/// Decodifica una maschera «PID supportati» di 4 byte: bit più alto = `base + 1`.
pub fn supported_from_mask(base: u8, mask: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, byte) in mask.iter().take(4).enumerate() {
        for bit in 0..8u8 {
            if byte & (0x80 >> bit) != 0 {
                out.push(base.wrapping_add((i as u8) * 8 + bit + 1));
            }
        }
    }
    out
}

/// Costruisce la maschera di 4 byte per l'intervallo che parte da `base`.
pub fn mask_from_supported(base: u8, pids: &[u8]) -> [u8; 4] {
    let mut m = [0u8; 4];
    for &p in pids {
        if p > base && (p - base) <= 32 {
            let i = (p - base - 1) as usize;
            m[i / 8] |= 0x80 >> (i % 8);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maschera_pid() {
        let pids = supported_from_mask(0x00, &[0xBE, 0x3F, 0xA8, 0x13]);
        assert!(pids.contains(&0x01) && pids.contains(&0x0C) && pids.contains(&0x20));
        assert!(!pids.contains(&0x02) && !pids.contains(&0x08));
        assert_eq!(
            supported_from_mask(0x20, &[0x80, 0, 0, 0x01]),
            vec![0x21, 0x40]
        );
        let m = mask_from_supported(0x00, &pids);
        assert_eq!(m, [0xBE, 0x3F, 0xA8, 0x13]);
    }

    #[test]
    fn solo_servizi_di_lettura() {
        let tutte = [
            ObdRequest::CurrentData(0x0C),
            ObdRequest::FreezeFrame(0x02),
            ObdRequest::StoredDtcs,
            ObdRequest::PendingDtcs,
            ObdRequest::PermanentDtcs,
            ObdRequest::MonitorResults(0x21),
            ObdRequest::Vin,
        ];
        for r in tutte {
            let c = r.command();
            assert!(c.chars().all(|ch| ch.is_ascii_hexdigit()), "{c}");
            let servizio = u8::from_str_radix(&c[..2], 16).unwrap();
            assert!(
                matches!(servizio, 0x01 | 0x02 | 0x03 | 0x06 | 0x07 | 0x09 | 0x0A),
                "servizio non ammesso: {c}"
            );
        }
    }

    #[test]
    fn nessuna_lettura_produce_il_servizio_04() {
        for p in 0..=u8::MAX {
            for r in [
                ObdRequest::CurrentData(p),
                ObdRequest::FreezeFrame(p),
                ObdRequest::MonitorResults(p),
                ObdRequest::StoredDtcs,
                ObdRequest::PendingDtcs,
                ObdRequest::PermanentDtcs,
                ObdRequest::Vin,
            ] {
                let c = r.command();
                assert_ne!(r.service(), 0x04, "{c}");
                assert!(!c.starts_with("04"), "{c}");
            }
        }
    }
}
