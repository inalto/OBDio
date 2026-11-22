//! OBDio — lettore diagnostico OBD-II/EOBD. Tutto in lettura, salvo la
//! cancellazione confermata dei codici guasto (servizio 04).
//!
//! Strati, dal basso verso l'alto:
//! - [`transport`]: canale di byte verso l'adattatore (BLE, seriale, demo);
//! - [`elm`]: dialogo ELM327 con elenco chiuso dei comandi ammessi;
//! - [`obd`]: decodifica delle risposte OBD-II;
//! - [`diagnosis`]: lettura di DTC, prontezza, fermo immagine, Mode 06;
//! - [`recording`]: sessioni su disco ed esportazione CSV;
//! - [`registri`]: registro degli errori fatali e dei passi di collegamento;
//! - [`scambio`]: importazione ed esportazione (CSV OBDio, CSV in stile Torque, JSON);
//! - [`riproduzione`]: riproduzione in tempo reale di una sessione registrata;
//! - [`mqtt`]: pubblicazione in tempo reale su un broker MQTT;
//! - [`mcp`]: server MCP locale per assistenti AI (spento finché non lo si attiva);
//! - `aggiornamenti`: controllo e installazione delle versioni nuove da GitHub
//!   (solo desktop, in un thread a parte);
//! - [`worker`]: thread di lavoro con stati espliciti;
//! - [`ui`]: interfaccia egui.

#[cfg(not(target_os = "android"))]
pub mod aggiornamenti;
pub mod config;
pub mod diagnosi_ble;
pub mod diagnosis;
pub mod elm;
pub mod mcp;
pub mod model;
pub mod mqtt;
pub mod obd;
pub mod piattaforma;
pub mod recording;
pub mod registri;
pub mod riproduzione;
pub mod scambio;
pub mod summary;
pub mod transport;
pub mod ui;
pub mod worker;

/// Punto d'ingresso su Android (GameActivity). Condivide con la versione per
/// desktop la costruzione dell'app, [`ui::crea_app`].
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    // Prima di tutto: un panic da qui in poi finisce nel registro degli errori.
    registri::installa_gestore_panic();
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    // Dati privati dell'app: unica cartella scrivibile senza permessi.
    if let Some(percorso) = app.internal_data_path() {
        config::imposta_cartella_base(percorso);
    }
    let opzioni = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    if let Err(e) = eframe::run_native("OBDio", opzioni, Box::new(|cc| Ok(ui::crea_app(cc)))) {
        log::error!("OBDio terminata con errore: {e}");
    }
}
