//! Canale di byte verso l'adattatore. Non conosce né ELM327 né OBD.

use std::time::Duration;

pub mod ble;
pub mod copione;
pub mod demo;
pub mod profili;
pub mod serial;

/// Il collegamento fisico con l'adattatore è caduto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scollegato(pub String);

pub trait Transport: Send {
    /// Invia i byte all'adattatore.
    fn write(&mut self, data: &[u8]) -> Result<(), Scollegato>;

    /// Restituisce i byte arrivati, attendendo al massimo `timeout`.
    /// Un vettore vuoto significa che nel frattempo non è arrivato nulla.
    /// I dati possono arrivare spezzati in qualunque punto.
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Scollegato>;

    /// Descrizione leggibile del collegamento.
    fn descrizione(&self) -> String;

    /// Vero se i dati sono simulati.
    fn is_demo(&self) -> bool {
        false
    }

    /// Chiude il collegamento. Chiamate successive non devono fallire.
    fn close(&mut self) {}
}
