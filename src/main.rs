// Nelle build di rilascio non apre la finestra della console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    // Prima di tutto: da qui in poi ogni panic finisce nel registro degli
    // errori, anche quello che sta per abortire il processo.
    obdio::registri::installa_gestore_panic();
    obdio::registri::inoltra_log_btleplug();
    let mut argomenti = std::env::args().skip(1);
    if argomenti.next().as_deref() == Some("--diagnosi-ble") {
        // Diagnosi Bluetooth da terminale (docs/DIAGNOSI_PROBLEMI.md).
        std::process::exit(obdio::diagnosi_ble::da_riga_di_comando(argomenti.next()));
    }
    obdio::ui::esegui()
}
