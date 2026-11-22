//! Prova VERA dell'installazione di un aggiornamento da GitHub, su una copia.
//!
//! Scarica l'ultimo rilascio di `inalto/OBDio`, verifica dimensione e SHA-256
//! e lo mette al posto del file indicato, esattamente come fa l'app. Va usata
//! su una COPIA di un eseguibile più vecchio in una cartella di prova, mai
//! sull'installazione in uso.
//!
//! ```text
//! cargo run --release --example prova_installazione -- <copia di OBDio.exe> <versione della copia>
//! ```

#[cfg(not(target_os = "android"))]
fn main() {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use obdio::aggiornamenti::{Cliente, ImpostazioniRete, Versione, scarica_e_installa};

    let mut argomenti = std::env::args().skip(1);
    let (Some(exe), Some(versione)) = (argomenti.next(), argomenti.next()) else {
        eprintln!("uso: prova_installazione <copia di OBDio.exe> <versione della copia>");
        std::process::exit(2);
    };
    let exe = PathBuf::from(exe);
    let Some(installata) = Versione::leggi(&versione) else {
        eprintln!("versione non valida: {versione}");
        std::process::exit(2);
    };
    let cliente = Cliente::new(ImpostazioniRete::github());
    let rilascio = match cliente.ultimo_rilascio() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("controllo fallito: {}", e.messaggio());
            std::process::exit(1);
        }
    };
    println!(
        "Ultimo rilascio: {} — copia da aggiornare: {installata}",
        rilascio.etichetta
    );
    let annulla = AtomicBool::new(false);
    let mut ultimo = 0u64;
    let mut progresso = |fatti: u64, totale: u64| {
        // Una riga ogni 2 MB circa.
        if fatti - ultimo >= 2_000_000 || fatti == totale {
            ultimo = fatti;
            println!("  scaricati {fatti} di {totale} byte");
        }
    };
    match scarica_e_installa(
        &cliente,
        &rilascio,
        &installata,
        &exe,
        "windows-x64",
        &annulla,
        &mut progresso,
    ) {
        Ok(i) => {
            println!("Installata la versione {}", i.versione);
            println!("Eseguibile: {}", i.eseguibile.display());
            println!("Copia precedente: {}", i.vecchio.display());
        }
        Err(e) => {
            eprintln!("installazione rifiutata: {}", e.messaggio());
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "android")]
fn main() {}
