//! Modalità Land Rover Td5 (Defender e Discovery 2 con motore Td5,
//! 1998–2006), in SOLA LETTURA, tramite un adattatore OBDLink CX (Bluetooth
//! LE) o LX (Bluetooth classico, porta COM).
//!
//! Il Td5 parla un dialetto proprio di KWP2000 con messaggi senza
//! intestazione: la lettura OBD-II generica non funziona (docs/RICERCA_TD5.md).
//! Strati, dal basso:
//! - [`frame`]: lunghezza, checksum, estrazione tollerante delle risposte;
//! - [`chiave`]: algoritmo seme → chiave (td5keygen, licenza BSD a 2 clausole);
//! - [`richieste`]: elenco chiuso delle richieste, tutte di lettura;
//! - [`decodifica`] e [`guasti`]: valori in tempo reale e mappa dei guasti;
//! - [`stn`]: confine [`stn::CanaleTd5`] fra messaggi e filo, e la sua
//!   realizzazione per gli OBDLink con tre strategie di ripiego;
//! - [`sessione`]: apertura, mantenimento, ripristino, letture, diagnosi;
//! - [`simulato`]: adattatore e centralina simulati per verifiche e demo.
//!
//! Mai provato su un adattatore o un veicolo veri.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::diagnosis::Lettura;

pub mod chiave;
pub mod decodifica;
pub mod frame;
pub mod guasti;
pub mod richieste;
pub mod sessione;
pub mod simulato;
pub mod stn;

pub use sessione::SessioneTd5;
pub use stn::{CanaleTd5, ErroreTd5, Stn, Strategia};

/// Parte della diagnosi propria del Td5 (assente per l'OBD-II).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosiTd5 {
    /// Esito della lettura della mappa dei guasti (`21 3B`).
    pub lettura_guasti: Lettura,
    /// Risposta a `1A 9A`, in testo (formato non verificato).
    pub tipo_centralina: Option<String>,
    /// Risposta a `21 32`, in testo (formato non verificato).
    pub variante_mappa: Option<String>,
    /// Tensione della batteria letta dalla centralina (`21 10`), in volt.
    pub tensione_batteria: Option<f64>,
    /// Modo con cui l'adattatore ha aperto la sessione.
    pub strategia: Option<String>,
}

/// Tipo di veicolo scelto nella pagina Connessione.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoVeicolo {
    /// Lettura OBD-II/EOBD standard con un adattatore ELM327.
    #[default]
    ObdStandard,
    /// Centralina Land Rover Td5 con un OBDLink CX o LX.
    LandRoverTd5,
}

/// File della scelta nella cartella di configurazione.
pub const FILE_CONFIGURAZIONE: &str = "veicolo.json";

#[derive(Default, Serialize, Deserialize)]
struct ConfigVeicolo {
    #[serde(default)]
    tipo: TipoVeicolo,
}

impl TipoVeicolo {
    pub const TUTTI: [TipoVeicolo; 2] = [TipoVeicolo::ObdStandard, TipoVeicolo::LandRoverTd5];

    pub fn nome(self) -> &'static str {
        match self {
            TipoVeicolo::ObdStandard => "OBD-II standard",
            TipoVeicolo::LandRoverTd5 => "Land Rover Td5 (OBDLink CX/LX)",
        }
    }

    /// Legge la scelta salvata; senza file vale l'OBD-II standard.
    pub fn carica(cartella: &Path) -> Result<Self, String> {
        Ok(
            crate::config::carica::<ConfigVeicolo>(cartella, FILE_CONFIGURAZIONE)?
                .unwrap_or_default()
                .tipo,
        )
    }

    pub fn salva(self, cartella: &Path) -> Result<(), String> {
        crate::config::salva(cartella, FILE_CONFIGURAZIONE, &ConfigVeicolo { tipo: self })
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scelta_del_veicolo_salvata_e_riletta() {
        let dir = std::env::temp_dir().join(format!("obdio_veicolo_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(TipoVeicolo::carica(&dir).unwrap(), TipoVeicolo::ObdStandard);
        TipoVeicolo::LandRoverTd5.salva(&dir).unwrap();
        assert_eq!(
            TipoVeicolo::carica(&dir).unwrap(),
            TipoVeicolo::LandRoverTd5
        );
        let testo = std::fs::read_to_string(dir.join(FILE_CONFIGURAZIONE)).unwrap();
        assert!(testo.contains("land_rover_td5"), "{testo}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
