//! Riproduzione attraverso il thread di lavoro reale: una sessione demo
//! registrata davvero viene riaperta e rivissuta; i campioni rientrano per la
//! stessa strada dei dati dal vivo, MQTT compreso.

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use common::{Broker, attendi};
use obdio::elm::ConfermaCancellazione;
use obdio::model::{Marker, Origine, Sample, Stato};
use obdio::mqtt::MqttConfig;
use obdio::recording::{self, SessionData};
use obdio::scambio;
use obdio::transport::demo::DemoTransport;
use obdio::worker::{
    self, Comando, ComandoRiproduzione, ErroreUtente, Evento, InfoConn, Sorgente, StatoRiproduzione,
};

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("obdio_riproduzione_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Registra una volta, con il simulatore, una sessione di circa 4 secondi che
/// attraversa l'interruzione della sonda a valle (letture senza risposta),
/// con diagnosi e due marcatori.
fn sessione_demo() -> &'static SessionData {
    static S: OnceLock<SessionData> = OnceLock::new();
    S.get_or_init(|| {
        let dir = cartella("registrata");
        let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
        // La sonda a valle non risponde fra 48 e 50,5 s del ciclo demo.
        let demo = DemoTransport::con_latenza(Duration::from_millis(3)).con_fase(47.0);
        tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(demo))))
            .unwrap();
        attendi(&rx, 20, |e| matches!(e, Evento::Connesso(_)).then_some(()));
        tx.send(Comando::LeggiDiagnosi).unwrap();
        attendi(&rx, 20, |e| matches!(e, Evento::Diagnosi(_)).then_some(()));
        tx.send(Comando::AvviaRegistrazione).unwrap();
        let percorso = attendi(&rx, 10, |e| match e {
            Evento::RegistrazioneAvviata { percorso, .. } => Some(percorso),
            _ => None,
        });
        tx.send(Comando::AvviaPolling {
            chiavi: ["giri", "velocita", "o2s2_tensione", "temp_refrigerante"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            prioritarie: vec!["o2s2_tensione".into()],
        })
        .unwrap();
        let inizio = Instant::now();
        let mut mancanti = 0;
        let mut marcato = false;
        while inizio.elapsed() < Duration::from_millis(4000) || mancanti == 0 {
            assert!(
                inizio.elapsed() < Duration::from_secs(20),
                "nessuna lettura fallita"
            );
            if let Ok(Evento::Campioni(v)) = rx.recv_timeout(Duration::from_millis(100)) {
                mancanti += v.iter().filter(|c| c.stato != Stato::Ok).count();
            }
            if !marcato && inizio.elapsed() > Duration::from_millis(700) {
                tx.send(Comando::Marcatore("Minimo".into())).unwrap();
                marcato = true;
            }
        }
        tx.send(Comando::Marcatore("Fine, a \"caldo\"".into()))
            .unwrap();
        tx.send(Comando::FermaRegistrazione).unwrap();
        attendi(&rx, 10, |e| {
            matches!(e, Evento::RegistrazioneFermata { .. }).then_some(())
        });
        tx.send(Comando::Esci).unwrap();
        handle.join().unwrap();
        let s = recording::carica(&percorso).unwrap();
        assert!(s.intestazione.demo);
        assert_eq!(s.marcatori.len(), 2);
        assert!(s.diagnosi.is_some());
        assert!(
            s.campioni
                .iter()
                .any(|c| c.key == "o2s2_tensione" && c.stato == Stato::RispostaMancante),
            "la sessione deve contenere letture fallite"
        );
        s
    })
}

/// Avvia la riproduzione e restituisce le informazioni del collegamento.
fn avvia(
    tx: &std::sync::mpsc::Sender<Comando>,
    rx: &Receiver<Evento>,
    s: &SessionData,
) -> InfoConn {
    tx.send(Comando::Collega(Sorgente::Riproduzione(Box::new(
        s.clone(),
    ))))
    .unwrap();
    attendi(rx, 10, |e| match e {
        Evento::Connesso(i) => Some(*i),
        Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
        _ => None,
    })
}

/// Raccoglie campioni e marcatori fino a che la riproduzione finisce.
fn fino_alla_fine(rx: &Receiver<Evento>, secondi: u64) -> (Vec<Sample>, Vec<Marker>) {
    let mut campioni = Vec::new();
    let mut marcatori = Vec::new();
    attendi(rx, secondi, |e| match e {
        Evento::Campioni(v) => {
            campioni.extend(v);
            None
        }
        Evento::Marcatore(m) => {
            marcatori.push(m);
            None
        }
        Evento::Salto { posizione } => panic!("salto inatteso a {posizione}"),
        Evento::Riproduzione(StatoRiproduzione { finita: true, .. }) => Some(()),
        _ => None,
    });
    (campioni, marcatori)
}

#[test]
fn riproduzione_a_10x_identica_alla_registrazione() {
    let s = sessione_demo();
    let dir = cartella("dieci");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let info = avvia(&tx, &rx, s);
    assert_eq!(info.origine, Origine::Riproduzione);
    assert!(info.in_riproduzione());
    assert!(info.demo, "sessione demo: dati sintetici");
    let r = info.riproduzione.as_ref().unwrap();
    assert_eq!(r.file, s.percorso.file_name().unwrap().to_string_lossy());
    assert_eq!(r.registrata_il, s.intestazione.creata);
    assert!(r.sessione_demo);
    assert_eq!(info.ecu, vec!["7E8".to_string()]);
    for pid in [0x0C, 0x0D, 0x05, 0x15] {
        assert!(info.unione.contains(&pid), "PID {pid:02X}");
    }
    assert!(!info.unione.contains(&0x5C), "l'olio non è nella sessione");
    assert!(info.supportati["7E8"].contains(&0x0C));
    // La diagnosi registrata arriva subito.
    let d = attendi(&rx, 5, |e| match e {
        Evento::Diagnosi(d) => Some(*d),
        _ => None,
    });
    assert_eq!(d.dtc[0].codice, "P0420");

    tx.send(Comando::Riproduzione(ComandoRiproduzione::Velocita(10.0)))
        .unwrap();
    let inizio = Instant::now();
    let (campioni, marcatori) = fino_alla_fine(&rx, 20);
    let durata = s.campioni.last().unwrap().t;
    assert!(
        inizio.elapsed().as_secs_f64() < durata / 2.0,
        "a 10× doveva durare circa {:.1} s, è durata {:?}",
        durata / 10.0,
        inizio.elapsed()
    );
    // Stessi campioni, nello stesso ordine, con chiavi, valori e stati
    // (letture fallite comprese), e i tempi della sessione.
    assert_eq!(campioni.len(), s.campioni.len());
    assert_eq!(&campioni, &s.campioni);
    assert!(
        campioni
            .iter()
            .any(|c| c.stato == Stato::RispostaMancante && c.valore.is_none())
    );
    assert_eq!(marcatori, s.marcatori);

    // Durante la riproduzione si rifiuta ciò che toccherebbe una centralina.
    tx.send(Comando::AvviaRegistrazione).unwrap();
    tx.send(Comando::CancellaDtc(
        ConfermaCancellazione::utente_ha_confermato(),
    ))
    .unwrap();
    tx.send(Comando::LeggiDiagnosi).unwrap();
    tx.send(Comando::Marcatore("no".into())).unwrap();
    let mut rifiuti = Vec::new();
    attendi(&rx, 5, |e| match e {
        Evento::Errore(ErroreUtente::Riproduzione(m)) => {
            rifiuti.push(m);
            (rifiuti.len() == 4).then_some(())
        }
        Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
        Evento::RegistrazioneAvviata { .. } => panic!("non doveva registrare"),
        Evento::CancellazioneDtc(_) => panic!("non doveva cancellare"),
        Evento::Diagnosi(_) => panic!("non doveva leggere la diagnosi"),
        Evento::Marcatore(_) => panic!("non doveva aggiungere marcatori"),
        _ => None,
    });
    assert!(rifiuti[0].contains("non si registra"), "{}", rifiuti[0]);
    assert!(rifiuti[1].contains("codici guasto"), "{}", rifiuti[1]);
    assert!(rifiuti[2].contains("diagnosi"), "{}", rifiuti[2]);
    assert_eq!(
        ErroreUtente::Riproduzione(String::new()).titolo(),
        "Non disponibile durante la riproduzione"
    );

    // Chiudere la riproduzione è una disconnessione normale.
    tx.send(Comando::Scollega).unwrap();
    let motivo = attendi(&rx, 5, |e| match e {
        Evento::Scollegato { motivo } => Some(motivo),
        _ => None,
    });
    assert_eq!(motivo, None);
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    // Nessun file è stato scritto: niente registrazione, niente copie.
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pausa_ferma_il_flusso_e_il_salto_riparte_dalla_posizione() {
    let s = sessione_demo();
    let durata = s.campioni.last().unwrap().t;
    let dir = cartella("pausa");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    avvia(&tx, &rx, s);
    // Qualche campione a 1×, poi pausa.
    let mut visti = 0;
    attendi(&rx, 10, |e| {
        if let Evento::Campioni(v) = e {
            visti += v.len();
        }
        (visti >= 5).then_some(())
    });
    // La selezione della lettura è accettata e non cambia nulla.
    tx.send(Comando::AvviaPolling {
        chiavi: vec!["giri".into()],
        prioritarie: vec![],
    })
    .unwrap();
    tx.send(Comando::Riproduzione(ComandoRiproduzione::Pausa))
        .unwrap();
    let ferma = attendi(&rx, 5, |e| match e {
        Evento::Riproduzione(st) if st.in_pausa => Some(st),
        Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
        _ => None,
    });
    std::thread::sleep(Duration::from_millis(400));
    let dopo: Vec<Evento> = rx.try_iter().collect();
    assert!(
        dopo.iter().all(|e| !matches!(e, Evento::Campioni(_))),
        "in pausa non arrivano campioni"
    );

    // Salto in avanti: un evento dedicato, poi solo campioni dalla posizione.
    let meta = (durata * 0.6).max(ferma.posizione + 0.5);
    tx.send(Comando::Riproduzione(ComandoRiproduzione::VaiA(meta)))
        .unwrap();
    let p = attendi(&rx, 5, |e| match e {
        Evento::Salto { posizione } => Some(posizione),
        Evento::Campioni(_) => panic!("campioni prima del salto"),
        _ => None,
    });
    assert!((p - meta).abs() < 1e-9);
    tx.send(Comando::Riproduzione(ComandoRiproduzione::Riprendi))
        .unwrap();
    let primi = attendi(&rx, 10, |e| match e {
        Evento::Campioni(v) => Some(v),
        _ => None,
    });
    assert!(
        primi.iter().all(|c| c.t >= meta - 1e-9),
        "nulla da prima della posizione"
    );
    let atteso = s.campioni.iter().position(|c| c.t >= meta).unwrap();
    assert_eq!(primi[0], s.campioni[atteso]);

    // Salto indietro all'inizio: di nuovo dal primo campione.
    tx.send(Comando::Riproduzione(ComandoRiproduzione::VaiA(0.0)))
        .unwrap();
    attendi(&rx, 5, |e| match e {
        Evento::Salto { posizione } => {
            assert_eq!(posizione, 0.0);
            Some(())
        }
        _ => None,
    });
    let primi = attendi(&rx, 10, |e| match e {
        Evento::Campioni(v) => Some(v),
        _ => None,
    });
    assert_eq!(primi[0], s.campioni[0]);

    // Con la ripetizione, arrivati alla fine si riparte da zero (salto).
    tx.send(Comando::Riproduzione(ComandoRiproduzione::Ripeti(true)))
        .unwrap();
    tx.send(Comando::Riproduzione(ComandoRiproduzione::VaiA(
        durata - 0.3,
    )))
    .unwrap();
    attendi(&rx, 5, |e| match e {
        Evento::Salto { posizione } if posizione > 0.0 => Some(()),
        _ => None,
    });
    attendi(&rx, 10, |e| match e {
        Evento::Salto { posizione: 0.0 } => Some(()),
        Evento::Riproduzione(StatoRiproduzione { finita: true, .. }) => {
            panic!("con la ripetizione non finisce")
        }
        _ => None,
    });
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mqtt_dice_riproduzione_mai_veicolo() {
    let s = sessione_demo();
    let broker = Broker::avvia(0);
    let dir = cartella("mqtt");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    tx.send(Comando::ConfiguraMqtt(MqttConfig {
        attivo: true,
        host: "127.0.0.1".into(),
        porta: broker.porta,
        id_client: "test-riproduzione".into(),
        veicolo: "auto".into(),
        intervallo_sensore_ms: 0,
        intervallo_istantanea_ms: 100,
        ..Default::default()
    }))
    .unwrap();
    broker.pubblicazione(10, "obdio/auto/stato", |p| p.testo().contains("offline"));
    avvia(&tx, &rx, s);

    let online = broker.pubblicazione(10, "obdio/auto/stato", |p| p.testo().contains("online"));
    assert!(online.trattenuto);
    assert_eq!(online.json()["origine"], "riproduzione");
    let ist = broker.attendi(15, |r| {
        r.pubblicazioni
            .iter()
            .filter(|p| p.argomento == "obdio/auto/istantanea")
            .map(|p| p.json())
            .find(|j| j["valori"].get("giri").is_some())
    });
    assert_eq!(ist["origine"], "riproduzione");
    let giri = broker.pubblicazione(10, "obdio/auto/sensori/giri", |_| true);
    assert!(giri.testo().parse::<f64>().is_ok());
    let m = broker.pubblicazione(10, "obdio/auto/evento", |p| p.testo().contains("Minimo"));
    assert_eq!(m.json()["origine"], "riproduzione");
    // Nessun messaggio dice che i dati vengono dal veicolo o dal simulatore
    // dal vivo, e la diagnosi registrata non sostituisce quella attuale.
    for p in broker.tutte() {
        let t = p.testo();
        assert!(
            !t.contains("\"veicolo\"") && !t.contains("demo_sintetico"),
            "{t}"
        );
        assert_ne!(p.argomento, "obdio/auto/diagnosi");
    }
    tx.send(Comando::Scollega).unwrap();
    broker.attendi(10, |r| {
        r.pubblicazioni
            .iter()
            .rev()
            .find(|p| p.argomento == "obdio/auto/stato")
            .filter(|p| p.testo().contains("offline"))
            .map(|_| ())
    });
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sessione_torque_importata_rivissuta_con_i_parametri_liberi() {
    let dir = cartella("torque");
    let testo = "GPS Time,Device Time,Engine RPM(rpm),GPS Speed (Meters/second),Speed (OBD)(mph)\n\
                 ,21-Nov-2026 19:37:41.000,800,0,0\n\
                 ,21-Nov-2026 19:37:41.100,-,0.5,1\n\
                 ,21-Nov-2026 19:37:41.200,900,1.5,-\n";
    let imp = scambio::importa_byte(testo.as_bytes(), "trackLog.csv").unwrap();
    let s = scambio::salva_importata(&dir, &imp).unwrap();
    let s = recording::carica(&s.percorso).unwrap();
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let info = avvia(&tx, &rx, &s);
    assert!(!info.demo);
    assert!(info.liberi.contains("torque:gps_speed"));
    assert!(info.unione.contains(&0x0C) && info.unione.contains(&0x0D));
    assert!(
        info.riproduzione
            .unwrap()
            .importata
            .unwrap()
            .contains("Torque")
    );
    let (campioni, _) = fino_alla_fine(&rx, 10);
    assert_eq!(campioni, s.campioni);
    let giri: Vec<(Option<f64>, Stato)> = campioni
        .iter()
        .filter(|c| c.key == "giri")
        .map(|c| (c.valore, c.stato))
        .collect();
    assert_eq!(
        giri,
        vec![
            (Some(800.0), Stato::Ok),
            (None, Stato::RispostaMancante),
            (Some(900.0), Stato::Ok)
        ]
    );
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
