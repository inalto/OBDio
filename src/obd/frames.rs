//! Dalle righe ELM327 (con intestazioni attive) ai messaggi per centralina.

use serde::{Deserialize, Serialize};

/// Tipo di bus, da cui dipende il formato delle righe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusKind {
    /// ISO 15765-4 CAN con identificatori a 11 bit.
    Can11,
    /// ISO 15765-4 CAN con identificatori a 29 bit.
    Can29,
    /// J1850, ISO 9141-2, ISO 14230-4: tre byte di intestazione e checksum.
    Legacy,
}

impl BusKind {
    pub fn is_can(self) -> bool {
        !matches!(self, BusKind::Legacy)
    }
}

/// Messaggio completo di una centralina; `data[0]` è il byte di servizio della risposta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EcuMessage {
    pub ecu: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Riga con caratteri non esadecimali.
    NonEsadecimale(String),
    /// Riga troppo corta per contenere intestazione e dati.
    TroppoCorta(String),
    /// Risposta su più trame non completa.
    Incompleta { ecu: String },
    /// Trame consecutive fuori sequenza.
    FuoriSequenza { ecu: String },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::NonEsadecimale(r) => write!(f, "riga non esadecimale: «{r}»"),
            FrameError::TroppoCorta(r) => write!(f, "riga troppo corta: «{r}»"),
            FrameError::Incompleta { ecu } => {
                write!(f, "risposta multi-trama incompleta da {ecu}")
            }
            FrameError::FuoriSequenza { ecu } => write!(f, "trame fuori sequenza da {ecu}"),
        }
    }
}

fn hex_bytes(s: &str, riga: &str) -> Result<Vec<u8>, FrameError> {
    if !s.len().is_multiple_of(2) {
        return Err(FrameError::TroppoCorta(riga.to_string()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| FrameError::NonEsadecimale(riga.to_string()))
        })
        .collect()
}

struct Parziale {
    ecu: String,
    data: Vec<u8>,
    atteso: usize,
    prossimo: u8,
}

/// Ricompone le righe in messaggi, uno per centralina sui bus CAN
/// (riassemblando ISO-TP) e uno per riga sugli altri protocolli.
pub fn assemble(lines: &[String], kind: BusKind) -> Result<Vec<EcuMessage>, FrameError> {
    let mut out: Vec<EcuMessage> = Vec::new();
    let mut parziali: Vec<Parziale> = Vec::new();

    for riga in lines {
        let compatta: String = riga.chars().filter(|c| !c.is_whitespace()).collect();
        if compatta.is_empty() {
            continue;
        }
        if !compatta.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(FrameError::NonEsadecimale(riga.clone()));
        }
        match kind {
            BusKind::Legacy => {
                let b = hex_bytes(&compatta, riga)?;
                if b.len() < 5 {
                    return Err(FrameError::TroppoCorta(riga.clone()));
                }
                out.push(EcuMessage {
                    ecu: format!("{:02X}", b[2]),
                    data: b[3..b.len() - 1].to_vec(),
                });
            }
            BusKind::Can11 | BusKind::Can29 => {
                let h = if kind == BusKind::Can11 { 3 } else { 8 };
                if compatta.len() < h + 4 {
                    return Err(FrameError::TroppoCorta(riga.clone()));
                }
                let ecu = compatta[..h].to_uppercase();
                let b = hex_bytes(&compatta[h..], riga)?;
                match b[0] >> 4 {
                    0 => {
                        let n = (b[0] & 0x0F) as usize;
                        if n == 0 || b.len() < 1 + n {
                            return Err(FrameError::TroppoCorta(riga.clone()));
                        }
                        out.push(EcuMessage {
                            ecu,
                            data: b[1..1 + n].to_vec(),
                        });
                    }
                    1 => {
                        let atteso = (((b[0] & 0x0F) as usize) << 8) | b[1] as usize;
                        parziali.retain(|p| p.ecu != ecu);
                        parziali.push(Parziale {
                            ecu,
                            data: b[2..].to_vec(),
                            atteso,
                            prossimo: 1,
                        });
                    }
                    2 => {
                        let Some(p) = parziali.iter_mut().find(|p| p.ecu == ecu) else {
                            return Err(FrameError::FuoriSequenza { ecu });
                        };
                        if b[0] & 0x0F != p.prossimo {
                            return Err(FrameError::FuoriSequenza { ecu });
                        }
                        p.prossimo = (p.prossimo + 1) & 0x0F;
                        p.data.extend_from_slice(&b[1..]);
                    }
                    // Controllo di flusso o trama sconosciuta: non porta dati.
                    _ => {}
                }
            }
        }
    }

    for mut p in parziali {
        if p.data.len() < p.atteso {
            return Err(FrameError::Incompleta { ecu: p.ecu });
        }
        p.data.truncate(p.atteso);
        out.push(EcuMessage {
            ecu: p.ecu,
            data: p.data,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn righe(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn trama_singola_con_riempimento() {
        let m = assemble(&righe(&["7E8 06 41 00 BE 3F A8 13 AA"]), BusKind::Can11).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].ecu, "7E8");
        assert_eq!(m[0].data, vec![0x41, 0x00, 0xBE, 0x3F, 0xA8, 0x13]);
    }

    #[test]
    fn senza_spazi() {
        let m = assemble(&righe(&["7E804410C2140"]), BusKind::Can11).unwrap();
        assert_eq!(m[0].data, vec![0x41, 0x0C, 0x21, 0x40]);
    }

    #[test]
    fn piu_centraline() {
        let m = assemble(
            &righe(&["7E8 06 41 00 BE 3F A8 13", "7E9 06 41 00 98 18 80 01"]),
            BusKind::Can11,
        )
        .unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].ecu, "7E9");
    }

    #[test]
    fn multi_trama_interlacciata() {
        // Due centraline rispondono al servizio 03 con trame alternate.
        let m = assemble(
            &righe(&[
                "7E8 10 08 43 03 04 20 01 71",
                "7E9 04 43 01 07 00",
                "7E8 21 03 00 AA AA AA AA AA",
            ]),
            BusKind::Can11,
        )
        .unwrap();
        assert_eq!(m.len(), 2);
        let e8 = m.iter().find(|x| x.ecu == "7E8").unwrap();
        assert_eq!(
            e8.data,
            vec![0x43, 0x03, 0x04, 0x20, 0x01, 0x71, 0x03, 0x00]
        );
    }

    #[test]
    fn multi_trama_incompleta() {
        let e = assemble(&righe(&["7E8 10 14 49 02 01 57 50 30"]), BusKind::Can11).unwrap_err();
        assert_eq!(e, FrameError::Incompleta { ecu: "7E8".into() });
    }

    #[test]
    fn fuori_sequenza() {
        let e = assemble(
            &righe(&["7E8 10 14 49 02 01 57 50 30", "7E8 22 00 00 00 00 00 00 00"]),
            BusKind::Can11,
        )
        .unwrap_err();
        assert_eq!(e, FrameError::FuoriSequenza { ecu: "7E8".into() });
    }

    #[test]
    fn can_29_bit() {
        let m = assemble(&righe(&["18 DA F1 10 03 41 0D 45"]), BusKind::Can29).unwrap();
        assert_eq!(m[0].ecu, "18DAF110");
        assert_eq!(m[0].data, vec![0x41, 0x0D, 0x45]);
    }

    #[test]
    fn protocolli_non_can() {
        let m = assemble(&righe(&["48 6B 10 41 0C 21 40 C1"]), BusKind::Legacy).unwrap();
        assert_eq!(m[0].ecu, "10");
        assert_eq!(m[0].data, vec![0x41, 0x0C, 0x21, 0x40]);
    }

    #[test]
    fn riga_sporca() {
        assert!(matches!(
            assemble(&righe(&["7E8 06 41 ZZ"]), BusKind::Can11),
            Err(FrameError::NonEsadecimale(_))
        ));
    }
}
