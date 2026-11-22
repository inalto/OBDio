//! Segnaposto della porta seriale su Android: stessa interfaccia di
//! `serial.rs`, ma nessuna porta. Il Bluetooth classico richiederebbe l'API
//! Java (vedi docs/ANDROID.md).

use std::time::Duration;

use super::{Scollegato, Transport};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortaSeriale {
    pub nome: String,
    pub tipo: String,
}

/// Nessuna porta elencabile su Android.
pub fn elenca_porte() -> Result<Vec<PortaSeriale>, String> {
    Ok(Vec::new())
}

pub struct SerialTransport;

impl SerialTransport {
    pub fn apri(_nome: &str, _baud: u32) -> Result<Self, String> {
        Err("il collegamento seriale non è disponibile su Android".into())
    }
}

impl Transport for SerialTransport {
    fn write(&mut self, _data: &[u8]) -> Result<(), Scollegato> {
        Err(Scollegato("porta seriale non disponibile".into()))
    }

    fn read(&mut self, _timeout: Duration) -> Result<Vec<u8>, Scollegato> {
        Err(Scollegato("porta seriale non disponibile".into()))
    }

    fn descrizione(&self) -> String {
        "Seriale non disponibile".into()
    }
}
