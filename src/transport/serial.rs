//! Porta seriale (COM), usata dagli adattatori Bluetooth Classic/SPP e USB.

use std::io::{Read, Write};
use std::time::Duration;

use super::{Scollegato, Transport};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortaSeriale {
    pub nome: String,
    /// Tipo riportato dal sistema (Bluetooth, USB, ...).
    pub tipo: String,
}

/// Elenca le porte COM presenti.
pub fn elenca_porte() -> Result<Vec<PortaSeriale>, String> {
    let porte = serialport::available_ports().map_err(|e| e.to_string())?;
    let mut out: Vec<PortaSeriale> = porte
        .into_iter()
        .map(|p| PortaSeriale {
            tipo: match p.port_type {
                serialport::SerialPortType::BluetoothPort => "Bluetooth".to_string(),
                serialport::SerialPortType::UsbPort(_) => "USB".to_string(),
                serialport::SerialPortType::PciPort => "PCI".to_string(),
                serialport::SerialPortType::Unknown => "Tipo non indicato".to_string(),
            },
            nome: p.port_name,
        })
        .collect();
    out.sort_by(|a, b| a.nome.cmp(&b.nome));
    Ok(out)
}

pub struct SerialTransport {
    porta: Option<Box<dyn serialport::SerialPort>>,
    nome: String,
    baud: u32,
}

impl SerialTransport {
    /// Apre la porta. Con le porte Bluetooth l'apertura attende il collegamento
    /// radio e può richiedere alcuni secondi.
    pub fn apri(nome: &str, baud: u32) -> Result<Self, String> {
        let porta = serialport::new(nome, baud)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| e.to_string())?;
        Ok(SerialTransport {
            porta: Some(porta),
            nome: nome.to_string(),
            baud,
        })
    }
}

impl Transport for SerialTransport {
    fn write(&mut self, data: &[u8]) -> Result<(), Scollegato> {
        let Some(p) = self.porta.as_mut() else {
            return Err(Scollegato("porta seriale chiusa".into()));
        };
        p.write_all(data)
            .and_then(|_| p.flush())
            .map_err(|e| Scollegato(format!("scrittura su {} fallita: {e}", self.nome)))
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Scollegato> {
        let Some(p) = self.porta.as_mut() else {
            return Err(Scollegato("porta seriale chiusa".into()));
        };
        let disponibili = p
            .bytes_to_read()
            .map_err(|e| Scollegato(format!("porta {} non più disponibile: {e}", self.nome)))?;
        if disponibili == 0 && timeout.is_zero() {
            return Ok(Vec::new());
        }
        let _ = p.set_timeout(timeout.max(Duration::from_millis(1)));
        let mut buf = [0u8; 512];
        match p.read(&mut buf) {
            Ok(n) => Ok(buf[..n].to_vec()),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(Vec::new()),
            Err(e) => Err(Scollegato(format!("lettura da {} fallita: {e}", self.nome))),
        }
    }

    fn descrizione(&self) -> String {
        format!("Seriale {} ({} baud)", self.nome, self.baud)
    }

    fn close(&mut self) {
        self.porta = None;
    }
}
