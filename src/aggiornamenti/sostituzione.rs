//! Sostituzione dell'eseguibile in uso su Windows, dove un programma in
//! esecuzione non si può sovrascrivere ma si può rinominare:
//!
//! 1. il file nuovo è già accanto all'eseguibile con un nome temporaneo
//!    (`OBDio.exe.nuovo`), scritto e sincronizzato su disco;
//! 2. si elimina un eventuale `OBDio.vecchio.exe` rimasto da prima;
//! 3. `OBDio.exe` → `OBDio.vecchio.exe`;
//! 4. `OBDio.exe.nuovo` → `OBDio.exe`.
//!
//! Se il passo 4 fallisce si torna indietro (`OBDio.vecchio.exe` →
//! `OBDio.exe`, oppure una copia se anche la rinomina fallisce): con il nome
//! originale resta sempre un eseguibile funzionante. Il vecchio file si
//! elimina al successivo avvio riuscito ([`pulisci_residui`]).
//!
//! Le operazioni sui file passano dal tratto [`OperazioniFile`], così le
//! prove possono far fallire ogni passo.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::ErroreAggiornamento;

/// Variabile d'ambiente che consente l'installazione anche da una build di
/// sviluppo o da una cartella `target`. Solo per le prove automatiche.
pub const DEROGA_SVILUPPO: &str = "OBDIO_AGGIORNAMENTI_PROVA_INSTALLAZIONE";

/// Operazioni sui file usate dalla sostituzione.
pub trait OperazioniFile {
    fn rinomina(&self, da: &Path, a: &Path) -> io::Result<()>;
    fn rimuovi(&self, percorso: &Path) -> io::Result<()>;
    fn copia(&self, da: &Path, a: &Path) -> io::Result<()>;
    fn esiste(&self, percorso: &Path) -> bool;
}

/// Il file system vero.
pub struct FileReali;

impl OperazioniFile for FileReali {
    fn rinomina(&self, da: &Path, a: &Path) -> io::Result<()> {
        fs::rename(da, a)
    }
    fn rimuovi(&self, percorso: &Path) -> io::Result<()> {
        fs::remove_file(percorso)
    }
    fn copia(&self, da: &Path, a: &Path) -> io::Result<()> {
        fs::copy(da, a).map(|_| ())
    }
    fn esiste(&self, percorso: &Path) -> bool {
        percorso.exists()
    }
}

/// Nome con un'aggiunta prima dell'estensione: `OBDio.exe` → `OBDio.<x>.exe`.
fn con_aggiunta(exe: &Path, aggiunta: &str) -> PathBuf {
    let base = exe
        .file_stem()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| OsString::from("OBDio"));
    let mut nome = base;
    nome.push(".");
    nome.push(aggiunta);
    if let Some(e) = exe.extension() {
        nome.push(".");
        nome.push(e);
    }
    exe.with_file_name(nome)
}

/// `OBDio.exe` → `OBDio.vecchio.exe`, nella stessa cartella.
pub fn percorso_vecchio(exe: &Path) -> PathBuf {
    con_aggiunta(exe, "vecchio")
}

/// `OBDio.exe` → `OBDio.exe.nuovo`: il file scaricato, finché non è verificato
/// e messo al suo posto. Senza l'estensione `.exe` non si avvia per errore.
pub fn percorso_nuovo(exe: &Path) -> PathBuf {
    let mut nome = exe
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| OsString::from("OBDio.exe"));
    nome.push(".nuovo");
    exe.with_file_name(nome)
}

/// Esito di una sostituzione riuscita.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sostituito {
    /// L'eseguibile, ora nella versione nuova.
    pub eseguibile: PathBuf,
    /// La versione precedente, da eliminare al prossimo avvio.
    pub vecchio: PathBuf,
}

/// Mette `nuovo` al posto di `exe`. In caso di errore `exe` è ancora quello
/// di prima (o, se anche il ritorno indietro fallisce, l'errore dice dove
/// sta la copia della versione precedente) e `nuovo` è stato eliminato.
pub fn sostituisci(
    fs: &dyn OperazioniFile,
    exe: &Path,
    nuovo: &Path,
) -> Result<Sostituito, ErroreAggiornamento> {
    let vecchio = percorso_vecchio(exe);
    if !fs.esiste(nuovo) {
        return Err(ErroreAggiornamento::Sostituzione(format!(
            "il file scaricato {} non c'è più",
            nuovo.display()
        )));
    }
    // 2. Residuo di un aggiornamento precedente.
    if fs.esiste(&vecchio)
        && let Err(e) = fs.rimuovi(&vecchio)
    {
        let _ = fs.rimuovi(nuovo);
        return Err(ErroreAggiornamento::Sostituzione(format!(
            "la copia precedente {} non si può eliminare ({e}): forse è ancora aperta. \
             Chiudi le altre finestre di OBDio e riprova",
            vecchio.display()
        )));
    }
    // 3. L'eseguibile in uso si sposta da parte.
    if let Err(e) = fs.rinomina(exe, &vecchio) {
        let _ = fs.rimuovi(nuovo);
        return Err(ErroreAggiornamento::Sostituzione(format!(
            "{} non si può rinominare ({e}); nulla è stato cambiato",
            exe.display()
        )));
    }
    // 4. Il nuovo prende il suo nome.
    if let Err(e) = fs.rinomina(nuovo, exe) {
        // Ritorno indietro: prima con una rinomina, poi con una copia.
        let ripristino = fs
            .rinomina(&vecchio, exe)
            .or_else(|_| fs.copia(&vecchio, exe));
        let _ = fs.rimuovi(nuovo);
        return match ripristino {
            Ok(()) => Err(ErroreAggiornamento::Sostituzione(format!(
                "il file nuovo non si può mettere al posto di {} ({e}); è stata ripristinata \
                 la versione in uso",
                exe.display()
            ))),
            Err(e2) => Err(ErroreAggiornamento::SostituzioneCritica(format!(
                "il file nuovo non si può mettere al suo posto ({e}) e la versione in uso non si \
                 può ripristinare ({e2}). La versione in uso è in {}: rinominala in {} a mano",
                vecchio.display(),
                exe.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ))),
        };
    }
    Ok(Sostituito {
        eseguibile: exe.to_path_buf(),
        vecchio,
    })
}

/// Elimina `OBDio.vecchio.exe` e un eventuale `OBDio.exe.nuovo` rimasto a
/// metà. `Ok(true)` se non resta nulla; `Ok(false)` se un file è ancora
/// bloccato (appena dopo il riavvio la versione precedente può essere ancora
/// in chiusura): si riprova più tardi.
pub fn pulisci_residui(fs: &dyn OperazioniFile, exe: &Path) -> bool {
    let mut pulito = true;
    for p in [percorso_vecchio(exe), percorso_nuovo(exe)] {
        if fs.esiste(&p) && fs.rimuovi(&p).is_err() {
            pulito = false;
        }
    }
    pulito
}

/// Controlla che nella cartella dell'eseguibile si possa scrivere, creando
/// ed eliminando un file di prova.
pub fn cartella_scrivibile(exe: &Path) -> Result<(), ErroreAggiornamento> {
    let cartella = exe.parent().unwrap_or(Path::new("."));
    let prova = cartella.join(".obdio_prova_aggiornamento");
    match fs::File::create(&prova) {
        Ok(_) => {
            let _ = fs::remove_file(&prova);
            Ok(())
        }
        Err(e) => Err(ErroreAggiornamento::CartellaNonScrivibile(
            cartella.to_path_buf(),
            e.to_string(),
        )),
    }
}

/// Motivo per cui questa copia non deve sostituirsi da sola: una build di
/// sviluppo o un eseguibile dentro una cartella `target` (prodotto di
/// `cargo build`). `deroga` è la variabile [`DEROGA_SVILUPPO`] impostata a `1`.
pub fn blocco_sviluppo(exe: &Path, debug: bool, deroga: bool) -> Option<String> {
    if deroga {
        return None;
    }
    if debug {
        return Some(
            "Questa è una build di sviluppo (debug): non si aggiorna da sola, per non \
             sovrascrivere i file di compilazione."
                .into(),
        );
    }
    let in_target = exe
        .parent()
        .into_iter()
        .flat_map(|p| p.components())
        .any(|c| c.as_os_str().eq_ignore_ascii_case("target"));
    in_target.then(|| {
        "Questo eseguibile sta in una cartella «target» di compilazione: non si aggiorna da \
         solo, per non sovrascrivere i file di compilazione."
            .into()
    })
}

/// [`blocco_sviluppo`] per questa build, con la deroga letta dall'ambiente.
pub fn blocco_sviluppo_corrente(exe: &Path) -> Option<String> {
    let deroga = std::env::var_os(DEROGA_SVILUPPO).is_some_and(|v| v == "1");
    blocco_sviluppo(exe, cfg!(debug_assertions), deroga)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// File system vero che fallisce l'operazione numero `guasto` (dalla
    /// prima, contando da 1) e le successive del tipo indicato in `anche`.
    struct ConGuasti {
        guasti: Vec<usize>,
        n: Cell<usize>,
    }

    impl ConGuasti {
        fn new(guasti: &[usize]) -> Self {
            ConGuasti {
                guasti: guasti.to_vec(),
                n: Cell::new(0),
            }
        }
        fn passo(&self) -> io::Result<()> {
            let n = self.n.get() + 1;
            self.n.set(n);
            if self.guasti.contains(&n) {
                Err(io::Error::other(format!("guasto simulato al passo {n}")))
            } else {
                Ok(())
            }
        }
    }

    impl OperazioniFile for ConGuasti {
        fn rinomina(&self, da: &Path, a: &Path) -> io::Result<()> {
            self.passo()?;
            fs::rename(da, a)
        }
        fn rimuovi(&self, p: &Path) -> io::Result<()> {
            self.passo()?;
            fs::remove_file(p)
        }
        fn copia(&self, da: &Path, a: &Path) -> io::Result<()> {
            self.passo()?;
            fs::copy(da, a).map(|_| ())
        }
        fn esiste(&self, p: &Path) -> bool {
            p.exists()
        }
    }

    fn cartella(nome: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("obdio_sost_{nome}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Cartella con `OBDio.exe` («vecchio») e `OBDio.exe.nuovo` («nuovo»).
    fn prepara(nome: &str, con_residuo: bool) -> (PathBuf, PathBuf, PathBuf) {
        let d = cartella(nome);
        let exe = d.join("OBDio.exe");
        let nuovo = percorso_nuovo(&exe);
        fs::write(&exe, b"versione 0.3.0").unwrap();
        fs::write(&nuovo, b"versione 0.4.0").unwrap();
        if con_residuo {
            fs::write(percorso_vecchio(&exe), b"versione 0.2.0").unwrap();
        }
        (d, exe, nuovo)
    }

    #[test]
    fn nomi_dei_file() {
        let exe = Path::new(r"C:\OBDio\OBDio.exe");
        assert_eq!(
            percorso_vecchio(exe),
            Path::new(r"C:\OBDio\OBDio.vecchio.exe")
        );
        assert_eq!(percorso_nuovo(exe), Path::new(r"C:\OBDio\OBDio.exe.nuovo"));
        let senza = Path::new("/opt/obdio/obdio");
        assert_eq!(
            percorso_vecchio(senza),
            Path::new("/opt/obdio/obdio.vecchio")
        );
    }

    #[test]
    fn sostituzione_riuscita() {
        let (d, exe, nuovo) = prepara("ok", true);
        let s = sostituisci(&FileReali, &exe, &nuovo).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"versione 0.4.0");
        assert_eq!(fs::read(&s.vecchio).unwrap(), b"versione 0.3.0");
        assert!(!nuovo.exists());
        // Al prossimo avvio il vecchio sparisce.
        assert!(pulisci_residui(&FileReali, &exe));
        assert!(!s.vecchio.exists());
        assert_eq!(fs::read(&exe).unwrap(), b"versione 0.4.0");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn guasto_a_ogni_passo_lascia_l_eseguibile_originale() {
        // Operazioni con il residuo: 1 rimuovi vecchio, 2 rinomina exe,
        // 3 rinomina nuovo (poi 4 ripristino, 5 rimuovi nuovo).
        for guasto in 1..=3 {
            let (d, exe, nuovo) = prepara(&format!("guasto{guasto}"), true);
            let esito = sostituisci(&ConGuasti::new(&[guasto]), &exe, &nuovo);
            assert!(
                matches!(esito, Err(ErroreAggiornamento::Sostituzione(_))),
                "passo {guasto}: {esito:?}"
            );
            assert_eq!(
                fs::read(&exe).unwrap(),
                b"versione 0.3.0",
                "passo {guasto}: l'eseguibile originale deve restare al suo posto"
            );
            assert!(
                !nuovo.exists(),
                "passo {guasto}: il file nuovo va eliminato"
            );
            let _ = fs::remove_dir_all(d);
        }
    }

    #[test]
    fn ripristino_con_copia_se_la_rinomina_fallisce() {
        // Senza residuo: 1 rinomina exe, 2 rinomina nuovo (guasto),
        // 3 rinomina vecchio→exe (guasto), 4 copia (riesce).
        let (d, exe, nuovo) = prepara("copia", false);
        let esito = sostituisci(&ConGuasti::new(&[2, 3]), &exe, &nuovo);
        assert!(matches!(esito, Err(ErroreAggiornamento::Sostituzione(_))));
        assert_eq!(fs::read(&exe).unwrap(), b"versione 0.3.0");
        assert!(!nuovo.exists());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn ripristino_impossibile_lo_dice_e_conserva_la_copia() {
        let (d, exe, nuovo) = prepara("critico", false);
        let esito = sostituisci(&ConGuasti::new(&[2, 3, 4]), &exe, &nuovo);
        match esito {
            Err(ErroreAggiornamento::SostituzioneCritica(m)) => {
                assert!(m.contains("OBDio.vecchio.exe"), "{m}")
            }
            altro => panic!("{altro:?}"),
        }
        // Mai zero eseguibili: la versione in uso è ancora su disco, intera.
        assert_eq!(fs::read(percorso_vecchio(&exe)).unwrap(), b"versione 0.3.0");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn file_nuovo_mancante() {
        let d = cartella("mancante");
        let exe = d.join("OBDio.exe");
        fs::write(&exe, b"x").unwrap();
        assert!(sostituisci(&FileReali, &exe, &percorso_nuovo(&exe)).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"x");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn pulizia_rimandata_se_il_file_e_bloccato() {
        let (d, exe, _nuovo) = prepara("pulizia", true);
        // Primo tentativo: la rimozione del vecchio fallisce (bloccato).
        assert!(!pulisci_residui(&ConGuasti::new(&[1]), &exe));
        assert!(percorso_vecchio(&exe).exists());
        // Più tardi riesce; anche il file nuovo a metà sparisce.
        assert!(pulisci_residui(&FileReali, &exe));
        assert!(!percorso_vecchio(&exe).exists());
        assert!(!percorso_nuovo(&exe).exists());
        assert!(exe.exists());
        // Niente da pulire: va bene lo stesso.
        assert!(pulisci_residui(&FileReali, &exe));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn cartella_non_scrivibile() {
        let d = cartella("scrivibile");
        assert!(cartella_scrivibile(&d.join("OBDio.exe")).is_ok());
        let assente = d.join("non_esiste").join("OBDio.exe");
        assert!(matches!(
            cartella_scrivibile(&assente),
            Err(ErroreAggiornamento::CartellaNonScrivibile(..))
        ));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn build_di_sviluppo_non_si_sostituiscono() {
        let rilascio = Path::new("C:/Programmi portatili/OBDio/OBDio.exe");
        assert!(blocco_sviluppo(rilascio, false, false).is_none());
        assert!(blocco_sviluppo(rilascio, true, false).is_some());
        let target = Path::new("F:/Apps/OBDio/target/release/obdio-desktop.exe");
        assert!(blocco_sviluppo(target, false, false).is_some());
        let target_maiuscolo = Path::new("F:/Apps/OBDio/TARGET/release/OBDio.exe");
        assert!(blocco_sviluppo(target_maiuscolo, false, false).is_some());
        // «target» solo come parte di un nome non conta.
        let simile = Path::new("C:/targets/OBDio/OBDio.exe");
        assert!(blocco_sviluppo(simile, false, false).is_none());
        // La deroga esplicita (solo prove) sblocca entrambi.
        assert!(blocco_sviluppo(target, true, true).is_none());
        // In questa build di test (debug) senza deroga è sempre bloccata.
        if cfg!(debug_assertions) && std::env::var_os(DEROGA_SVILUPPO).is_none() {
            assert!(blocco_sviluppo_corrente(rilascio).is_some());
        }
    }
}
