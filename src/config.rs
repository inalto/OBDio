//! Cartelle dei dati e file di configurazione in JSON.
//!
//! La configurazione sta in file leggibili accanto all'eseguibile, così il
//! pacchetto resta portabile e gli stessi file servono per importare ed
//! esportare. Su Android la cartella base è quella privata dell'app, fissata
//! all'avvio con [`imposta_cartella_base`] (vedi docs/ANDROID.md).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Serialize;
use serde::de::DeserializeOwned;

static CARTELLA_BASE: OnceLock<PathBuf> = OnceLock::new();

/// Fissa una volta per tutte la cartella base dei dati (su Android, quella
/// privata dell'app). Va chiamata all'avvio, prima di ogni altra funzione di
/// questo modulo. Restituisce `false` se era già stata fissata.
pub fn imposta_cartella_base(base: PathBuf) -> bool {
    CARTELLA_BASE.set(base).is_ok()
}

/// Cartella base fissata con [`imposta_cartella_base`], se presente.
pub fn cartella_base() -> Option<&'static Path> {
    CARTELLA_BASE.get().map(PathBuf::as_path)
}

/// `base/nome`, creata se manca.
fn cartella_in_base(base: &Path, nome: &str) -> PathBuf {
    let dir = base.join(nome);
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Cartella `nome` nella cartella base, se fissata; altrimenti accanto
/// all'eseguibile, oppure in `Documenti\OBDio` se quella dell'eseguibile non
/// è scrivibile. Viene creata se manca.
pub fn cartella_dati(nome: &str) -> PathBuf {
    if let Some(base) = cartella_base() {
        return cartella_in_base(base, nome);
    }
    let accanto = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(nome)));
    if let Some(dir) = accanto
        && fs::create_dir_all(&dir).is_ok()
        && fs::File::create(dir.join(".prova_scrittura")).is_ok()
    {
        let _ = fs::remove_file(dir.join(".prova_scrittura"));
        return dir;
    }
    let dir = crate::piattaforma::cartella_documenti_obdio().join(nome);
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Cartella dei file di configurazione (cruscotti, MQTT, preferenze).
pub fn cartella_configurazione() -> PathBuf {
    cartella_dati("configurazione")
}

/// Legge `cartella/nome`. `Ok(None)` se il file non esiste.
pub fn carica<T: DeserializeOwned>(cartella: &Path, nome: &str) -> Result<Option<T>, String> {
    let percorso = cartella.join(nome);
    let testo = match fs::read_to_string(&percorso) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("impossibile leggere {}: {e}", percorso.display())),
    };
    serde_json::from_str(testo.trim_start_matches('\u{feff}'))
        .map(Some)
        .map_err(|e| format!("{} non è valido: {e}", percorso.display()))
}

/// Scrive `cartella/nome` in JSON leggibile. Scrive prima un file temporaneo
/// e poi lo rinomina, per non lasciare un file a metà.
pub fn salva<T: Serialize>(cartella: &Path, nome: &str, valore: &T) -> Result<PathBuf, String> {
    fs::create_dir_all(cartella)
        .map_err(|e| format!("impossibile creare {}: {e}", cartella.display()))?;
    let percorso = cartella.join(nome);
    let temporaneo = cartella.join(format!("{nome}.tmp"));
    let testo = serde_json::to_string_pretty(valore).map_err(|e| e.to_string())?;
    fs::write(&temporaneo, testo)
        .map_err(|e| format!("impossibile scrivere {}: {e}", temporaneo.display()))?;
    fs::rename(&temporaneo, &percorso)
        .map_err(|e| format!("impossibile scrivere {}: {e}", percorso.display()))?;
    Ok(percorso)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Prova {
        nome: String,
        valore: u32,
    }

    #[test]
    fn cartella_in_base_crea_la_sottocartella() {
        let base = std::env::temp_dir().join(format!("obdio_base_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = cartella_in_base(&base, "sessioni");
        assert_eq!(dir, base.join("sessioni"));
        assert!(dir.is_dir());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn scrittura_e_rilettura() {
        let dir = std::env::temp_dir().join(format!("obdio_config_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(carica::<Prova>(&dir, "prova.json").unwrap(), None);
        let p = Prova {
            nome: "cruscotto".into(),
            valore: 7,
        };
        let percorso = salva(&dir, "prova.json", &p).unwrap();
        assert!(percorso.exists() && !dir.join("prova.json.tmp").exists());
        assert_eq!(carica::<Prova>(&dir, "prova.json").unwrap(), Some(p));
        fs::write(dir.join("rotto.json"), "{").unwrap();
        assert!(carica::<Prova>(&dir, "rotto.json").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
