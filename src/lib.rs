//! OBDio — lettore diagnostico OBD-II/EOBD. Tutto in lettura, salvo la
//! cancellazione confermata dei codici guasto (servizio 04).
//!
//! Strati, dal basso verso l'alto:
//! - [`transport`]: canale di byte verso l'adattatore (BLE, seriale, demo);
//! - [`elm`]: dialogo ELM327 con elenco chiuso dei comandi ammessi;
//! - [`obd`]: decodifica delle risposte OBD-II;
//! - [`diagnosis`]: lettura di DTC, prontezza, fermo immagine, Mode 06;
//! - [`recording`]: sessioni su disco ed esportazione CSV;
//! - [`worker`]: thread di lavoro con stati espliciti;
//! - [`ui`]: interfaccia egui.

pub mod config;
pub mod diagnosis;
pub mod elm;
pub mod model;
pub mod obd;
pub mod recording;
pub mod summary;
pub mod transport;
pub mod ui;
pub mod worker;
