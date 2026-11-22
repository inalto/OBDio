//! La cartella base dei dati è globale e si fissa una volta sola: la prova sta
//! in un test a parte (processo proprio) per non disturbare gli altri.

use std::path::PathBuf;

use obdio::config;

#[test]
fn predefinita_invariata_poi_sostituita_dalla_base() {
    // Prima: nessuna base, la cartella resta quella di sempre (accanto
    // all'eseguibile, o in Documenti\OBDio).
    assert!(config::cartella_base().is_none());
    let prima = config::cartella_dati("prova_base");
    let exe = std::env::current_exe().unwrap();
    assert_eq!(prima, exe.parent().unwrap().join("prova_base"));
    let _ = std::fs::remove_dir_all(&prima);

    // Dopo: la base fissata vince, per i dati e per le sessioni.
    let base: PathBuf = std::env::temp_dir().join(format!("obdio_android_{}", std::process::id()));
    assert!(config::imposta_cartella_base(base.clone()));
    assert!(!config::imposta_cartella_base(base.join("altra")));
    assert_eq!(config::cartella_base(), Some(base.as_path()));
    assert_eq!(config::cartella_dati("prova_base"), base.join("prova_base"));
    assert_eq!(
        config::cartella_configurazione(),
        base.join("configurazione")
    );
    assert_eq!(obdio::recording::cartella_sessioni(), base.join("sessioni"));
    assert!(base.join("sessioni").is_dir());
    let _ = std::fs::remove_dir_all(&base);
}
