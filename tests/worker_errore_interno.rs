//! Un panic nel thread di lavoro non lascia l'app in attesa per sempre: il
//! thread lo segnala con `ErroreUtente::ErroreInterno`, chiude collegamento e
//! registrazione (che resta leggibile) e accetta il comando successivo.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use obdio::recording;
use obdio::transport::demo::DemoTransport;
use obdio::transport::{Scollegato, Transport};
use obdio::worker::{self, Comando, ErroreUtente, Evento, InfoConn, Sorgente, StatoConn};

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("obdio_interno_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn attendi<T>(rx: &Receiver<Evento>, secondi: u64, mut f: impl FnMut(Evento) -> Option<T>) -> T {
    let scadenza = Instant::now() + Duration::from_secs(secondi);
    loop {
        let resto = scadenza
            .checked_duration_since(Instant::now())
            .expect("evento atteso non arrivato in tempo");
        let ev = rx
            .recv_timeout(resto)
            .expect("evento atteso non arrivato in tempo");
        if let Some(x) = f(ev) {
            return x;
        }
    }
}

fn connessione(rx: &Receiver<Evento>) -> InfoConn {
    attendi(rx, 20, |e| match e {
        Evento::Connesso(i) => Some(*i),
        Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
        _ => None,
    })
}

/// Trasporto di prova: il simulatore, che va in panic in scrittura o in
/// lettura quando lo si chiede.
struct Bizzoso {
    demo: DemoTransport,
    in_scrittura: Arc<AtomicBool>,
    in_lettura: Arc<AtomicBool>,
}

impl Transport for Bizzoso {
    fn write(&mut self, data: &[u8]) -> Result<(), Scollegato> {
        if self.in_scrittura.load(Ordering::Relaxed) {
            panic!("panic di prova in scrittura");
        }
        self.demo.write(data)
    }

    fn read(&mut self, attesa: Duration) -> Result<Vec<u8>, Scollegato> {
        if self.in_lettura.load(Ordering::Relaxed) {
            panic!("panic di prova in lettura");
        }
        self.demo.read(attesa)
    }

    fn descrizione(&self) -> String {
        "trasporto di prova che va in panic".into()
    }
}

/// Attende l'errore interno e lo stato scollegato che lo segue.
fn errore_interno(rx: &Receiver<Evento>) -> (String, Option<PathBuf>) {
    let mut cosa = None;
    let mut registrazione = None;
    attendi(rx, 20, |e| match e {
        Evento::Errore(ErroreUtente::ErroreInterno(c)) => {
            cosa = Some(c);
            None
        }
        Evento::RegistrazioneFermata { percorso, .. } => {
            registrazione = Some(percorso);
            None
        }
        Evento::Stato(StatoConn::Scollegato) if cosa.is_some() => Some(()),
        Evento::Errore(altro) => panic!("errore inatteso: {altro:?}"),
        _ => None,
    });
    (cosa.unwrap(), registrazione)
}

#[test]
fn panic_in_scrittura_durante_il_collegamento() {
    let dir = cartella("scrittura");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let t = Bizzoso {
        demo: DemoTransport::con_latenza(Duration::from_millis(1)),
        in_scrittura: Arc::new(AtomicBool::new(true)),
        in_lettura: Arc::new(AtomicBool::new(false)),
    };
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(t))))
        .unwrap();
    let (cosa, _) = errore_interno(&rx);
    assert_eq!(cosa, "il collegamento");
    let messaggio = ErroreUtente::ErroreInterno(cosa).messaggio();
    assert!(
        messaggio
            .starts_with("Errore interno durante il collegamento: il collegamento è stato chiuso.")
            && messaggio.contains("registro degli errori"),
        "{messaggio}"
    );

    // Il thread è vivo: un collegamento demo successivo funziona.
    tx.send(Comando::Collega(Sorgente::Demo)).unwrap();
    let info = connessione(&rx);
    assert!(info.demo);
    tx.send(Comando::Esci).unwrap();
    handle.join().expect("il thread di lavoro non deve morire");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn panic_in_lettura_durante_il_polling_conserva_la_registrazione() {
    let dir = cartella("lettura");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let in_lettura = Arc::new(AtomicBool::new(false));
    let t = Bizzoso {
        demo: DemoTransport::con_latenza(Duration::from_millis(2)),
        in_scrittura: Arc::new(AtomicBool::new(false)),
        in_lettura: in_lettura.clone(),
    };
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(t))))
        .unwrap();
    connessione(&rx);
    tx.send(Comando::AvviaPolling {
        chiavi: vec!["giri".into(), "velocita".into()],
        prioritarie: vec![],
    })
    .unwrap();
    tx.send(Comando::AvviaRegistrazione).unwrap();
    let percorso = attendi(&rx, 10, |e| match e {
        Evento::RegistrazioneAvviata { percorso, .. } => Some(percorso),
        _ => None,
    });
    let mut visti = 0;
    attendi(&rx, 10, |e| {
        if let Evento::Campioni(v) = e {
            visti += v.len();
        }
        (visti >= 10).then_some(())
    });

    in_lettura.store(true, Ordering::Relaxed);
    let (cosa, chiusa) = errore_interno(&rx);
    assert_eq!(cosa, "la lettura dei sensori");
    assert_eq!(chiusa.as_ref(), Some(&percorso), "registrazione non chiusa");
    // La registrazione è leggibile e contiene i campioni già scritti.
    let s = recording::carica(&percorso).expect("sessione illeggibile dopo il panic");
    assert!(s.campioni.len() >= 10, "{} campioni", s.campioni.len());

    // Nessun altro campione: il polling è fermo.
    std::thread::sleep(Duration::from_millis(300));
    assert!(rx.try_iter().all(|e| !matches!(e, Evento::Campioni(_))));

    // E il thread accetta un nuovo collegamento.
    tx.send(Comando::Collega(Sorgente::Demo)).unwrap();
    assert!(connessione(&rx).demo);
    tx.send(Comando::Esci).unwrap();
    handle.join().expect("il thread di lavoro non deve morire");
    let _ = std::fs::remove_dir_all(&dir);
}
