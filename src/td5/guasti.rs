//! Codici guasto del Td5: una mappa di bit di 35 byte nella risposta a
//! `21 3B` (ricerca/td5/ConnectFragment.java, `read_faults`). Il bit `j` del
//! byte `i` è il guasto «(i+1)-(j+1)», per esempio «02-4».
//!
//! I nomi inglesi vengono dalla tabella `FaultCodes.java` del progetto
//! Td5OpenDiag (<https://github.com/Td5OpenDiag/Td5OpenDiag-android>, licenza
//! Apache 2.0; copia della licenza in LICENSE-THIRD-PARTY.md) e sono riportati
//! come sono, refusi compresi. «(L)» e «(C)» restano come nell'originale: il
//! loro significato non è documentato. La traduzione italiana c'è solo dove il
//! significato è chiaro; altrove resta il solo originale.
//!
//! Da verificare sul veicolo: l'ordine dei bit (qui il bit 0 è il meno
//! significativo, come nella funzione `get_bit` usata da Td5OpenDiag, di cui
//! però non abbiamo il sorgente) e la lunghezza della risposta.

use crate::diagnosis::RigaDtc;

/// Byte della mappa dei guasti dopo `61 3B`.
pub const BYTE_MAPPA: usize = 35;

/// Un guasto acceso nella mappa.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuastoTd5 {
    /// Numerazione del Td5, «02-4».
    pub codice: String,
    /// Nome originale (inglese), o `None` se la posizione non è in tabella.
    pub originale: Option<&'static str>,
    /// Traduzione italiana, solo dove il significato è chiaro.
    pub italiano: Option<&'static str>,
}

impl GuastoTd5 {
    /// Descrizione per l'utente: italiano con l'originale accanto, oppure il
    /// solo originale.
    pub fn descrizione(&self) -> String {
        match (self.italiano, self.originale) {
            (Some(it), Some(en)) => format!("{it} (originale: {en})"),
            (None, Some(en)) => en.to_string(),
            _ => "Guasto non presente nella tabella di Td5OpenDiag".to_string(),
        }
    }

    pub fn riga(&self) -> RigaDtc {
        RigaDtc {
            codice: self.codice.clone(),
            descrizione: self.descrizione(),
            ecu: "Td5".into(),
            // La mappa dei guasti non distingue confermati e in sospeso.
            stati: Vec::new(),
        }
    }
}

/// Voce della tabella: byte (da 0), bit (da 0), originale, italiano.
type Voce = (u8, u8, &'static str, Option<&'static str>);

/// Nome di una posizione della mappa.
pub fn voce(byte: u8, bit: u8) -> Option<(&'static str, Option<&'static str>)> {
    TABELLA
        .iter()
        .find(|v| v.0 == byte && v.1 == bit)
        .map(|v| (v.2, v.3))
}

/// Decodifica il corpo della risposta (`61 3B` e la mappa). `Err` se la
/// risposta non è quella attesa o è più corta di 35 byte di mappa.
pub fn decodifica(corpo: &[u8]) -> Result<Vec<GuastoTd5>, String> {
    let mappa = match corpo {
        [0x61, 0x3B, mappa @ ..] => mappa,
        _ => return Err("risposta non pertinente alla lettura dei codici guasto".into()),
    };
    if mappa.len() < BYTE_MAPPA {
        return Err(format!(
            "mappa dei codici guasto di {} byte invece di {BYTE_MAPPA}",
            mappa.len()
        ));
    }
    let mut v = Vec::new();
    for (i, b) in mappa.iter().take(BYTE_MAPPA).enumerate() {
        for j in 0..8u8 {
            if b >> j & 1 == 1 {
                let (originale, italiano) = match voce(i as u8, j) {
                    Some((o, it)) => (Some(o), it),
                    None => (None, None),
                };
                v.push(GuastoTd5 {
                    codice: format!("{:02}-{}", i + 1, j + 1),
                    originale,
                    italiano,
                });
            }
        }
    }
    Ok(v)
}

#[rustfmt::skip]
const TABELLA: &[Voce] = &[
    (0, 0, "egr inlet throttle diagnostics (L)", Some("diagnosi farfalla di aspirazione EGR (L)")),
    (0, 1, "turbocharger wastegate diagnostics (L)", Some("diagnosi wastegate del turbocompressore (L)")),
    (0, 2, "egr vacuum diagnostics (L)", Some("diagnosi depressione EGR (L)")),
    (0, 3, "temperature gauge diagnostics (L)", Some("diagnosi indicatore di temperatura (L)")),
    (0, 4, "driver demand problem 1 (L)", None),
    (0, 5, "driver demand problem 2 (L)", None),
    (0, 6, "air flow circuit (L)", Some("circuito del flusso d'aria (L)")),
    (0, 7, "manifold pressure circuit (L)", Some("circuito della pressione del collettore (L)")),
    (1, 0, "inlet air temp. circuit (L)", Some("circuito della temperatura dell'aria aspirata (L)")),
    (1, 1, "fuel temp. circuit (L)", Some("circuito della temperatura del gasolio (L)")),
    (1, 2, "coolant temp. circuit (L)", Some("circuito della temperatura del liquido di raffreddamento (L)")),
    (1, 3, "battery volts (L)", Some("tensione della batteria (L)")),
    (1, 4, "reference voltage (L)", Some("tensione di riferimento (L)")),
    (1, 5, "ambient air temp. circuit (L)", Some("circuito della temperatura dell'aria ambiente (L)")),
    (1, 6, "driver demand supply problem (L)", None),
    (1, 7, "ambient pressure circuit (L)", Some("circuito della pressione ambiente (L)")),
    (2, 0, "egr inlet throttle diagnostics (L)", Some("diagnosi farfalla di aspirazione EGR (L)")),
    (2, 1, "turbocharger wastegate diagnostics (L)", Some("diagnosi wastegate del turbocompressore (L)")),
    (2, 2, "egr vacuum diagnostics (L)", Some("diagnosi depressione EGR (L)")),
    (2, 3, "temperature gauge diagnostics (L)", Some("diagnosi indicatore di temperatura (L)")),
    (2, 4, "driver demand problem 1 (L)", None),
    (2, 5, "driver demand problem 2 (L)", None),
    (2, 6, "air flow circuit (L)", Some("circuito del flusso d'aria (L)")),
    (2, 7, "manifold pressure circuit (L)", Some("circuito della pressione del collettore (L)")),
    (3, 0, "inlet air temp. circuit (L)", Some("circuito della temperatura dell'aria aspirata (L)")),
    (3, 1, "fuel temperature circuit (L)", Some("circuito della temperatura del gasolio (L)")),
    (3, 2, "coolant temp. circuit (L)", Some("circuito della temperatura del liquido di raffreddamento (L)")),
    (3, 3, "battery volts (L)", Some("tensione della batteria (L)")),
    (3, 4, "reference voltage (L)", Some("tensione di riferimento (L)")),
    (3, 5, "ambient air temperature circuit (L)", Some("circuito della temperatura dell'aria ambiente (L)")),
    (3, 6, "driver demand supply problem (L)", None),
    (3, 7, "ambient pressure circuit (L)", Some("circuito della pressione ambiente (L)")),
    (4, 0, "egr inlet throttle diagnostics (C)", Some("diagnosi farfalla di aspirazione EGR (C)")),
    (4, 1, "turbocharger wastegate diagnostics (C)", Some("diagnosi wastegate del turbocompressore (C)")),
    (4, 2, "egr vacuum diagnostics (C)", Some("diagnosi depressione EGR (C)")),
    (4, 3, "temperature gauge diagnostics (C)", Some("diagnosi indicatore di temperatura (C)")),
    (4, 4, "driver demand problem 1 (C)", None),
    (4, 5, "driver demand problem 2 (C)", None),
    (4, 6, "air flow circuit (C)", Some("circuito del flusso d'aria (C)")),
    (4, 7, "manifold pressure circuit (C)", Some("circuito della pressione del collettore (C)")),
    (5, 0, "inlet air temp. circuit (C)", Some("circuito della temperatura dell'aria aspirata (C)")),
    (5, 1, "fuel temperature circuit (C)", Some("circuito della temperatura del gasolio (C)")),
    (5, 2, "coolant temp. circuit (C)", Some("circuito della temperatura del liquido di raffreddamento (C)")),
    (5, 3, "battery voltage problem (C)", Some("problema di tensione della batteria (C)")),
    (5, 4, "reference voltage (C)", Some("tensione di riferimento (C)")),
    (5, 6, "driver demand supply problem (C)", None),
    (5, 7, "ambient pressure circuit (C)", Some("circuito della pressione ambiente (C)")),
    (6, 0, "cruise lamp drive over temp. (L)", None),
    (6, 1, "fuel used output drive over temp. (L)", None),
    (6, 2, "radiator fan drive over temp. (L)", None),
    (6, 3, "active engine mounting over temp. (L)", None),
    (6, 4, "turbocharger wastegate short circuit (L)", None),
    (6, 5, "egr inlet throttle short circuit (L)", None),
    (6, 6, "egr vacuum modulator short circuit (L)", None),
    (6, 7, "temperature gauge short circuit (L)", None),
    (7, 0, "air conditioning fan drive over temp. (L)", None),
    (7, 1, "fuel pump drive over temp. (L)", None),
    (7, 2, "tacho drive over temp. (L)", None),
    (7, 3, "gearbox/abs drive over temp. (L)", None),
    (7, 4, "air conditioning clutch over temp. (L)", None),
    (7, 5, "mil lamp drive over temp. (L)", None),
    (7, 6, "glow plug relay drive over temp. (L)", None),
    (7, 7, "glowplug lamp drive over temperature (L)", None),
    (8, 0, "fuel used output drive open load (L)", None),
    (8, 1, "cruise lamp drive open load (L)", None),
    (8, 2, "radiator fan drive open load (L)", None),
    (8, 3, "active engine mounting open load (L)", None),
    (8, 4, "turbocharger wastegate open load (L)", None),
    (8, 5, "egr inlett throttle open load (L)", None),
    (8, 6, "egr vacuum modulator open load (L)", None),
    (8, 7, "temperature gauge open load (L)", None),
    (9, 0, "air conditioning fan drive open load (L)", None),
    (9, 1, "fuel pump drive open load (L)", Some("comando della pompa del gasolio a circuito aperto (L)")),
    (9, 2, "tachometer open load (L)", None),
    (9, 3, "gearbox/abs drive open load (L)", None),
    (9, 4, "air conditioning clutch open load (L)", None),
    (9, 5, "mil lamp drive open load (L)", None),
    (9, 6, "glow plug lamp drive open load (L)", None),
    (9, 7, "glow plug relay drive open load (L)", None),
    (10, 0, "cruise control lamp drive over temperature (C)", None),
    (10, 1, "fuel used output drive over temperature (C)", None),
    (10, 2, "radiator fan drive over temperature (C)", None),
    (10, 3, "active engine mounting over temperature (C)", None),
    (10, 4, "turbocharger wastegate short circuit (C)", None),
    (10, 5, "egr inlet throttle short circuit (C)", None),
    (10, 6, "egr vacuum modulator short circuit (C)", None),
    (10, 7, "temperature gauge short circuit (C)", None),
    (11, 0, "air conditioning fan drive open load (C)", None),
    (11, 1, "fuel pump drive open load (C)", Some("comando della pompa del gasolio a circuito aperto (C)")),
    (11, 2, "tachometer open load (C)", None),
    (11, 3, "gearbox/abs drive open load (C)", None),
    (11, 4, "air conditioning clutch open load (C)", None),
    (11, 5, "mil lamp drive open load (C)", None),
    (11, 6, "glow plug relay drive open load (C)", None),
    (11, 7, "glowplug relay drive open load (C)", None),
    (12, 0, "cruise control lamp drive over temp. (C)", None),
    (12, 1, "fuel used output drive over temp. (C)", None),
    (12, 2, "radiator fan drive over temp. (C)", None),
    (12, 3, "active engine mounting over temp. (C)", None),
    (12, 4, "turbocharger wastegate short circuit (C)", None),
    (12, 5, "egr inlet throttle short circuit (C)", None),
    (12, 6, "egr vacuum modulator short circuit (C)", None),
    (12, 7, "temperature gauge short circuit (C)", None),
    (13, 0, "air conditioning fan drive open load (C)", None),
    (13, 1, "fuel pump drive open load (C)", Some("comando della pompa del gasolio a circuito aperto (C)")),
    (13, 2, "tachometer open load (C)", None),
    (13, 3, "gearbox/abs drive open load (C)", None),
    (13, 4, "air conditioning clutch open load (C)", None),
    (13, 5, "mil lamp drive open load (C)", None),
    (13, 6, "glow plug relay drive open load (C)", None),
    (13, 7, "glowplug relay drive open load (C)", None),
    (14, 1, "high speed crank (L)", None),
    (15, 0, "pump relay open circuit", None),
    (15, 1, "high speed crank (L)", None),
    (16, 1, "high speed crank (C)", None),
    (18, 1, "can rx/tx error (L)", None),
    (18, 2, "can tx/rx error (L)", None),
    (18, 5, "noisy crank signal has been detected (L)", None),
    (18, 7, "can has had reset failure (L)", None),
    (19, 0, "turbocharger under boosting (L)", Some("sovralimentazione del turbocompressore insufficiente (L)")),
    (19, 1, "turbocharger over boosting (L)", Some("sovralimentazione del turbocompressore eccessiva (L)")),
    (19, 3, "egr valve stuck open (L)", Some("valvola EGR bloccata aperta (L)")),
    (19, 4, "egr valve stuck closed (L)", Some("valvola EGR bloccata chiusa (L)")),
    (20, 3, "driver demand 1 out of range (L)", None),
    (20, 4, "driver demand 2 out of range (L)", None),
    (20, 5, "problem detected with driver demand (L)", None),
    (20, 6, "inconsistencies found with driver demand (L)", None),
    (20, 7, "injector trim data corrupted (L)", None),
    (21, 0, "road speed missing (L)", Some("segnale di velocità del veicolo assente (L)")),
    (21, 2, "vehicle accel. outside bounds of cruise control (L)", None),
    (21, 6, "cruise control resume stuck closed (L)", None),
    (21, 7, "cruise control set stuck closed (L)", None),
    (22, 0, "excessive can bus off (C)", None),
    (22, 1, "can rx/tx error (C)", None),
    (22, 2, "can tx/rx error (C)", None),
    (22, 3, "unable to detect remote can mode (C)", None),
    (22, 4, "under boost has occurred on this trip (C)", None),
    (22, 5, "noisy crack signal has been detected (C)", None),
    (23, 0, "turbocharger under boosting (C)", Some("sovralimentazione del turbocompressore insufficiente (C)")),
    (23, 1, "turbocharger over boosting (C)", Some("sovralimentazione del turbocompressore eccessiva (C)")),
    (23, 2, "over boost has occurred this trip (C)", None),
    (23, 3, "egr valve stuck open (C)", Some("valvola EGR bloccata aperta (C)")),
    (23, 4, "egr valve stuck closed (C)", Some("valvola EGR bloccata chiusa (C)")),
    (23, 6, "problem detected with auto gear box (C)", None),
    (24, 3, "driver demand 1 out of range (L)", None),
    (24, 4, "driver demand 2 out of range (L)", None),
    (24, 5, "problem detected with drive demand (C)", None),
    (24, 6, "inconsistencies found with driver demand (C)", None),
    (24, 7, "injector trim data corrupted (C)", None),
    (25, 0, "road speed missing (C)", Some("segnale di velocità del veicolo assente (C)")),
    (25, 1, "cruise control system problem (C)", None),
    (25, 2, "vehicle accel. outside bounds for cruise control (C)", None),
    (25, 6, "cruise control resume stuck closed (C)", None),
    (25, 7, "cruise control set stuck closed (C)", None),
    (26, 0, "inj. 1 peak charge long (L)", None),
    (26, 1, "inj. 2 peak charge long (L)", None),
    (26, 2, "inj. 3 peak charge long (L)", None),
    (26, 3, "inj. 4 peak charge long (L)", None),
    (26, 4, "inj. 5 peak charge long (L)", None),
    (26, 5, "inj. 6 peak charge long (L)", None),
    (26, 6, "topside switch failed post injection (L)", None),
    (27, 0, "inj. 1 peak charge short (L)", None),
    (27, 1, "inj. 2 peak charge short (L)", None),
    (27, 2, "inj. 3 peak charge short (L)", None),
    (27, 3, "inj. 4 peak charge short (L)", None),
    (27, 4, "inj. 5 peak charge short (L)", None),
    (27, 5, "inj. 6 peak charge short (L)", None),
    (27, 6, "topside switch failed pre injection (L)", None),
    (28, 0, "inj. 1 peak charge long (C)", None),
    (28, 1, "inj. 2 peak charge long (C)", None),
    (28, 2, "inj. 3 peak charge long (C)", None),
    (28, 3, "inj. 4 peak charge long (C)", None),
    (28, 4, "inj. 5 peak charge long (C)", None),
    (28, 5, "inj. 6 peak charge long (C)", None),
    (28, 6, "topside switch failed post injection (C)", None),
    (29, 0, "inj. 1 peak charge short (C)", None),
    (29, 1, "inj. 2 peak charge short (C)", None),
    (29, 2, "inj. 3 peak charge short (C)", None),
    (29, 3, "inj. 4 peak charge short (C)", None),
    (29, 4, "inj. 5 peak charge short (C)", None),
    (29, 5, "inj. 6 peak charge short (C)", None),
    (29, 6, "topside switch failed pre injection (C)", None),
    (30, 0, "inj. 1 open circuit (L)", Some("iniettore 1 a circuito aperto (L)")),
    (30, 1, "inj. 2 open circuit (L)", Some("iniettore 2 a circuito aperto (L)")),
    (30, 2, "inj. 3 open circuit (L)", Some("iniettore 3 a circuito aperto (L)")),
    (30, 3, "inj. 4 open circuit (L)", Some("iniettore 4 a circuito aperto (L)")),
    (30, 4, "inj. 5 open circuit (L)", Some("iniettore 5 a circuito aperto (L)")),
    (30, 5, "inj. 6 open circuit (L)", Some("iniettore 6 a circuito aperto (L)")),
    (31, 0, "inj. 1 short circuit (L)", Some("iniettore 1 in cortocircuito (L)")),
    (31, 1, "inj. 2 short circuit (L)", Some("iniettore 2 in cortocircuito (L)")),
    (31, 2, "inj. 3 short circuit (L)", Some("iniettore 3 in cortocircuito (L)")),
    (31, 3, "inj. 4 short circuit (L)", Some("iniettore 4 in cortocircuito (L)")),
    (31, 4, "inj. 5 short circuit (L)", Some("iniettore 5 in cortocircuito (L)")),
    (31, 5, "inj. 6 short circuit (L)", Some("iniettore 6 in cortocircuito (L)")),
    (32, 0, "inj. 1 open circuit (C)", Some("iniettore 1 a circuito aperto (C)")),
    (32, 1, "inj. 2 open circuit (C)", Some("iniettore 2 a circuito aperto (C)")),
    (32, 2, "inj. 3 open circuit (C)", Some("iniettore 3 a circuito aperto (C)")),
    (32, 3, "inj. 4 open circuit (C)", Some("iniettore 4 a circuito aperto (C)")),
    (32, 4, "inj. 5 open circuit (C)", Some("iniettore 5 a circuito aperto (C)")),
    (32, 5, "inj. 6 open circuit (C)", Some("iniettore 6 a circuito aperto (C)")),
    (33, 0, "inj. 1 short circuit (C)", Some("iniettore 1 in cortocircuito (C)")),
    (33, 1, "inj. 2 short circuit (C)", Some("iniettore 2 in cortocircuito (C)")),
    (33, 2, "inj. 3 short circuit (C)", Some("iniettore 3 in cortocircuito (C)")),
    (33, 3, "inj. 4 short circuit (C)", Some("iniettore 4 in cortocircuito (C)")),
    (33, 4, "inj. 5 short circuit (C)", Some("iniettore 5 in cortocircuito (C)")),
    (33, 5, "inj. 6 short circuit (C)", Some("iniettore 6 in cortocircuito (C)")),
    (34, 0, "inj. 1 partial short circuit (L)", None),
    (34, 1, "inj. 2 partial short circuit (L)", None),
    (34, 2, "inj. 3 partial short circuit (L)", None),
    (34, 3, "inj. 4 partial short circuit (L)", None),
    (34, 4, "inj. 5 partial short circuit (L)", None),
    (34, 5, "inj. 6 partial short circuit (L)", None),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabella_coerente() {
        assert_eq!(TABELLA.len(), 212);
        for v in TABELLA {
            assert!((v.0 as usize) < BYTE_MAPPA && v.1 < 8, "{v:?}");
        }
        // Posizioni uniche.
        let mut viste = std::collections::BTreeSet::new();
        for v in TABELLA {
            assert!(viste.insert((v.0, v.1)), "{v:?}");
        }
        assert_eq!(voce(1, 3).map(|v| v.0), Some("battery volts (L)"));
        assert_eq!(
            voce(1, 3).and_then(|v| v.1),
            Some("tensione della batteria (L)")
        );
        // 06-6 e tutta la riga 18 mancano anche nell'originale.
        assert!(voce(5, 5).is_none());
        assert!((0..8).all(|j| voce(17, j).is_none()));
    }

    #[test]
    fn mappa_vuota_e_mappa_con_guasti() {
        let mut corpo = vec![0x61, 0x3B];
        corpo.extend([0u8; BYTE_MAPPA]);
        assert!(decodifica(&corpo).unwrap().is_empty());
        // Byte 2 bit 3 → 02-4 (tensione batteria); byte 31 bit 0 → 32-1;
        // byte 18 bit 0 → 19-1, non in tabella.
        corpo[2 + 1] = 0b0000_1000;
        corpo[2 + 31] = 0b0000_0001;
        corpo[2 + 18] = 0b0000_0001;
        let g = decodifica(&corpo).unwrap();
        let codici: Vec<&str> = g.iter().map(|x| x.codice.as_str()).collect();
        assert_eq!(codici, ["02-4", "19-1", "32-1"]);
        assert!(g[0].descrizione().contains("tensione della batteria"));
        assert!(g[0].descrizione().contains("battery volts (L)"));
        assert_eq!(g[1].originale, None);
        assert!(g[1].descrizione().contains("non presente"));
        assert_eq!(
            g[2].descrizione(),
            "iniettore 1 in cortocircuito (L) (originale: inj. 1 short circuit (L))"
        );
        let r = g[0].riga();
        assert_eq!((r.codice.as_str(), r.ecu.as_str()), ("02-4", "Td5"));
        assert!(r.stati.is_empty());
    }

    #[test]
    fn risposte_non_valide() {
        assert!(decodifica(&[0x61, 0x3B, 0x00]).is_err());
        assert!(decodifica(&[0x61, 0x3C]).is_err());
        assert!(decodifica(&[]).is_err());
    }
}
