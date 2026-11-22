//! Il registro degli errori sopravvive a un abort del processo.
//!
//! Da Rust 1.81 un panic che prova a risalire fuori da una funzione
//! `extern "system"` (come i gestori di eventi WinRT di btleplug) non si
//! propaga: il processo viene abortito. Questa prova avvia sé stessa come
//! processo figlio, che installa il gestore dei panic di OBDio e va in panic
//! dentro un callback `extern "system"` su un thread senza nome (come i thread
//! di WinRT). Il figlio muore di abort; il padre controlla che la voce sia sul
//! disco.

use std::path::PathBuf;
use std::process::Command;

const VARIABILE_FIGLIO: &str = "OBDIO_PROVA_FIGLIO_ABORT";
const MESSAGGIO: &str = "panic di prova dentro un callback di sistema";

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("obdio_errori_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Il callback, chiamato tramite puntatore come fa il sistema.
extern "system" fn callback_di_sistema(_argomento: usize) -> i32 {
    panic!("{MESSAGGIO}");
}

#[test]
#[ignore = "processo figlio di panic_in_un_callback_di_sistema_resta_nel_registro"]
fn figlio_va_in_panic_in_un_callback_di_sistema() {
    if std::env::var_os(VARIABILE_FIGLIO).is_none() {
        // Eseguito a mano con `--ignored`: senza la variabile non fa nulla.
        return;
    }
    obdio::registri::installa_gestore_panic();
    let f: extern "system" fn(usize) -> i32 = std::hint::black_box(callback_di_sistema);
    // Thread senza nome, come quelli di WinRT.
    let _ = std::thread::spawn(move || f(0)).join();
    // Non si arriva qui: il processo è già stato abortito.
    std::process::exit(0);
}

#[test]
fn panic_in_un_callback_di_sistema_resta_nel_registro() {
    let dir = cartella("abort");
    let uscita = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "figlio_va_in_panic_in_un_callback_di_sistema",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(VARIABILE_FIGLIO, "1")
        .env("OBDIO_REGISTRI", &dir)
        .env("OBDIO_SENZA_AVVISO", "1")
        .output()
        .unwrap();
    let errori = String::from_utf8_lossy(&uscita.stderr);
    // Il figlio è morto di abort, non è uscito normalmente.
    assert!(
        !uscita.status.success(),
        "il figlio doveva essere abortito: {:?}\n{errori}",
        uscita.status
    );
    assert!(
        errori.contains("cannot unwind") || errori.contains(MESSAGGIO),
        "stderr del figlio: {errori}"
    );
    #[cfg(windows)]
    assert_eq!(
        uscita.status.code().map(|c| c as u32),
        Some(0xC000_0409),
        "su Windows l'abort è un __fastfail (STATUS_STACK_BUFFER_OVERRUN)"
    );

    // Le voci sono sul disco, complete. Sono due: il panic nel callback e,
    // subito dopo, quello del runtime («panic in a function that cannot
    // unwind») che precede l'abort.
    let registro = dir.join(obdio::registri::FILE_ERRORI);
    let testo = std::fs::read_to_string(&registro).expect("registro degli errori mancante");
    assert_eq!(testo.matches("=== OBDio ").count(), 2, "{testo}");
    assert_eq!(testo.matches("=== fine ===").count(), 2, "{testo}");
    assert!(
        testo.contains(&format!("Messaggio: {MESSAGGIO}")),
        "{testo}"
    );
    assert!(testo.contains(env!("CARGO_PKG_VERSION")));
    assert!(testo.contains("Thread: (senza nome"), "{testo}");
    assert!(testo.contains("Posizione: tests"), "{testo}");
    assert!(testo.contains("Backtrace:"));
    assert!(
        testo.contains("callback_di_sistema"),
        "frame del callback: {testo}"
    );

    // Al prossimo avvio l'app lo segnalerebbe.
    let nuovi = obdio::registri::errori_nuovi_in(&dir).expect("avviso atteso");
    assert_eq!(nuovi.file, registro);
    let ultima = obdio::registri::ultima_voce(&registro).unwrap();
    assert!(
        ultima.contains("cannot unwind") && ultima.contains("abort"),
        "{ultima}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
