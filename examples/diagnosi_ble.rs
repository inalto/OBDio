//! Diagnosi Bluetooth LE da console, con lo stesso codice dell'app e della
//! modalità `OBDio.exe --diagnosi-ble`: scansione di 10 secondi, connessione,
//! elenco dei servizi GATT, scollegamento. Non invia alcun comando ELM327 e
//! non scrive nulla sull'adattatore. Stampa i passi e salva un resoconto.
//!
//! ```text
//! cargo run --example diagnosi_ble -- [nome o indirizzo]
//! ```

fn main() {
    obdio::registri::installa_gestore_panic();
    obdio::registri::inoltra_log_btleplug();
    let codice = obdio::diagnosi_ble::da_riga_di_comando(std::env::args().nth(1));
    std::process::exit(codice);
}
