//! Strumento di sviluppo: elenca porte COM e dispositivi BLE visibili, senza
//! collegarsi a nulla. `cargo run --example scansione_ble`

use std::sync::Arc;
use std::time::Duration;

use obdio::transport::ble::BleManager;
use obdio::transport::serial::elenca_porte;

fn main() {
    println!("Porte COM:");
    match elenca_porte() {
        Ok(p) if p.is_empty() => println!("  (nessuna)"),
        Ok(p) => p.iter().for_each(|x| println!("  {} - {}", x.nome, x.tipo)),
        Err(e) => println!("  errore: {e}"),
    }
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime"),
    );
    let ble = match BleManager::new(rt) {
        Ok(b) => b,
        Err(e) => {
            println!("Bluetooth non utilizzabile: {e:?}");
            return;
        }
    };
    if let Err(e) = ble.avvia_scansione() {
        println!("Scansione non avviata: {e:?}");
        return;
    }
    println!("Scansione BLE per 12 secondi...");
    std::thread::sleep(Duration::from_secs(12));
    let dispositivi = ble.dispositivi();
    ble.ferma_scansione();
    println!("Dispositivi BLE visti: {}", dispositivi.len());
    for d in dispositivi
        .iter()
        .filter(|d| !d.nome.is_empty() || d.probabile_obd)
    {
        println!(
            "  {}{} [{}] {:?} dBm",
            if d.probabile_obd { "[OBD] " } else { "" },
            d.nome,
            d.indirizzo,
            d.rssi
        );
    }
}
