//! Differenze tra piattaforme in un solo posto: il resto del codice chiede
//! qui che cosa è disponibile invece di spargere `cfg`.

use std::path::{Path, PathBuf};

/// Il collegamento seriale (COM, USB, Bluetooth Classic/SPP) è disponibile.
/// Su Android `serialport` non elenca le porte e il Bluetooth classico passa
/// dall'API Java.
pub const SERIALE_DISPONIBILE: bool = cfg!(not(target_os = "android"));

/// È possibile aprire una cartella nel gestore dei file di sistema.
pub const APRI_CARTELLA_DISPONIBILE: bool = cfg!(windows);

/// Forza la finestra di scelta file interna (campo del percorso ed elenco)
/// anche sul desktop, al posto della finestra di sistema. Serve per provare
/// quella interna; è sempre interna su Android, dove `rfd` non esiste, e con
/// la funzione `collaudo`, perché il collaudo automatico non deve aprire
/// finestre di sistema.
pub const FORZA_SCELTA_INTERNA: bool = false;

/// Per scegliere i file si usa la finestra di sistema (`rfd`).
pub const DIALOGO_DI_SISTEMA: bool =
    cfg!(not(target_os = "android")) && !cfg!(feature = "collaudo") && !FORZA_SCELTA_INTERNA;

/// Testo che sostituisce la sezione seriale dove non è disponibile.
pub const NOTA_SERIALE_ASSENTE: &str =
    "Su Android il collegamento seriale non è disponibile: usa il Bluetooth LE";

/// Apre `percorso` in Esplora risorse. Altrove non fa nulla.
pub fn apri_cartella(percorso: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(percorso).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = percorso;
    }
}

/// Cartella dei documenti dell'utente, se nota (Windows, Linux, macOS).
pub fn cartella_documenti_utente() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("USERPROFILE").map(|u| PathBuf::from(u).join("Documents"))
    } else if cfg!(target_os = "android") {
        None
    } else {
        std::env::var_os("HOME").map(|u| PathBuf::from(u).join("Documents"))
    }
}

/// `Documenti\OBDio`, oppure la cartella temporanea se i Documenti non sono
/// noti: ripiego per i dati quando la cartella dell'eseguibile non è scrivibile.
pub fn cartella_documenti_obdio() -> PathBuf {
    match std::env::var_os("USERPROFILE").filter(|_| cfg!(windows)) {
        Some(u) => PathBuf::from(u).join("Documents").join("OBDio"),
        None => std::env::temp_dir().join("Documents").join("OBDio"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn costanti_coerenti_su_desktop() {
        #[cfg(not(target_os = "android"))]
        {
            const { assert!(SERIALE_DISPONIBILE) };
            assert!(cartella_documenti_obdio().ends_with("Documents/OBDio"));
        }
        #[cfg(windows)]
        {
            const { assert!(APRI_CARTELLA_DISPONIBILE) };
            assert!(
                cartella_documenti_utente().is_some() || std::env::var_os("USERPROFILE").is_none()
            );
        }
        assert!(NOTA_SERIALE_ASSENTE.contains("Bluetooth LE"));
        // Il collaudo automatico non deve mai aprire una finestra di sistema.
        #[cfg(feature = "collaudo")]
        const {
            assert!(!DIALOGO_DI_SISTEMA)
        };
        #[cfg(all(not(feature = "collaudo"), not(target_os = "android")))]
        const {
            assert!(DIALOGO_DI_SISTEMA != FORZA_SCELTA_INTERNA)
        };
        #[cfg(target_os = "android")]
        const {
            assert!(!DIALOGO_DI_SISTEMA)
        };
    }

    #[test]
    fn cartella_documenti_utente_coerente_con_la_piattaforma() {
        assert_eq!(
            cartella_documenti_utente().is_some(),
            if cfg!(windows) {
                std::env::var_os("USERPROFILE").is_some()
            } else {
                cfg!(not(target_os = "android")) && std::env::var_os("HOME").is_some()
            }
        );
    }
}
