//! Controllo VERO dell'ultima versione su GitHub (api.github.com), come lo fa
//! l'app: una sola richiesta GET senza credenziali. Non scarica e non
//! installa nulla.
//!
//! ```text
//! cargo run --example controlla_aggiornamenti
//! ```
//!
//! Serve dopo aver creato il repository `inalto/OBDio` e pubblicato il primo
//! rilascio: stampa che cosa vede l'app (etichetta, file attesi presenti o
//! no, valutazione rispetto alla versione di questa build).

#[cfg(not(target_os = "android"))]
fn main() {
    use obdio::aggiornamenti::rilascio::{self, NOME_SOMME};
    use obdio::aggiornamenti::{Cliente, ConfigAggiornamenti, ImpostazioniRete, Versione, valuta};

    let imp = ImpostazioniRete::github();
    println!("Richiesta: GET {}", imp.indirizzo_ultimo());
    let installata = Versione::corrente();
    println!("Versione di questa build: {installata}");
    match Cliente::new(imp).ultimo_rilascio() {
        Ok(r) => {
            println!("Etichetta: {}", r.etichetta);
            println!("Titolo: {}", r.nome);
            println!("Pagina: {}", r.pagina);
            println!(
                "Pubblicato: {}",
                r.pubblicato.as_deref().unwrap_or("(non indicato)")
            );
            println!("Bozza: {} · pre-rilascio: {}", r.bozza, r.pre_rilascio);
            println!("File allegati:");
            for f in &r.risorse {
                println!("  {} ({} byte)", f.nome, f.dimensione);
            }
            match &r.versione {
                Some(v) => {
                    {
                        let p = "windows-x64";
                        let exe = rilascio::nome_eseguibile(v, p);
                        let zip = rilascio::nome_pacchetto(v, p);
                        println!(
                            "Atteso {exe}: {}",
                            if r.risorsa(&exe).is_some() {
                                "presente"
                            } else {
                                "MANCANTE"
                            }
                        );
                        println!(
                            "Atteso {zip}: {}",
                            if r.risorsa(&zip).is_some() {
                                "presente"
                            } else {
                                "mancante (solo informativo)"
                            }
                        );
                    }
                    println!(
                        "Atteso {NOME_SOMME}: {}",
                        if r.risorsa(NOME_SOMME).is_some() {
                            "presente"
                        } else {
                            "MANCANTE"
                        }
                    );
                }
                None => println!("Etichetta non riconosciuta come versione: l'app la ignora."),
            }
            println!(
                "Valutazione: {:?}",
                valuta(&installata, &r, &ConfigAggiornamenti::default())
            );
        }
        Err(e) => {
            println!("Esito: {e:?}");
            println!("Messaggio per l'utente: {}", e.messaggio());
        }
    }
}

#[cfg(target_os = "android")]
fn main() {}
