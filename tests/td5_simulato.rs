//! Modalità Land Rover Td5 attraverso il thread di lavoro reale, con
//! l'adattatore OBDLink e la centralina Td5 simulati: collegamento, lettura,
//! registrazione, riapertura della sessione, diagnosi senza cancellazione,
//! mantenimento a riposo, nessun VIN nei registri e nei file.
//!
//! Il simulatore risponde come OBDio si aspetta: queste prove non dicono
//! nulla su come si comporti un adattatore vero (docs/RICERCA_TD5.md).

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::attendi;
use obdio::elm::ConfermaCancellazione;
use obdio::model::{Origine, Stato};
use obdio::recording;
use obdio::td5::simulato::SimulatoreTd5;
use obdio::worker::{self, Comando, ErroreUtente, Evento, InfoConn, Sorgente};

const VIN_PROVA: &str = "TDCINQUE123456789";

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("obdio_td5_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn td5(sim: SimulatoreTd5) -> Sorgente {
    Sorgente::Td5(Box::new(Sorgente::Trasporto(Box::new(sim))))
}

#[test]
fn collegamento_lettura_registrazione_e_riapertura() {
    let dir = cartella("flusso");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let sim = SimulatoreTd5::new().senza_identificativi(&[obdio::td5::richieste::IdLocale::Egr]);
    tx.send(Comando::Collega(td5(sim))).unwrap();
    let mut registro: Vec<String> = Vec::new();
    let info: InfoConn = attendi(&rx, 20, |e| match e {
        Evento::Registro(r) => {
            registro.extend(r);
            None
        }
        Evento::Connesso(i) => Some(*i),
        Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
        _ => None,
    });
    assert!(info.td5 && info.demo);
    assert_eq!(info.origine, Origine::Demo);
    assert_eq!(info.ecu, vec!["Td5".to_string()]);
    assert!(
        info.adattatore.contains("OBDLink CX"),
        "{}",
        info.adattatore
    );
    assert!(
        info.protocollo.contains("strategia A"),
        "{}",
        info.protocollo
    );
    assert!(info.avvisi.iter().any(|a| a.contains("mai provata")));
    // Parametri forniti: quelli OBD-II con lo stesso significato e quelli
    // solo del Td5; l'EGR rifiutato dalla centralina non c'è.
    let chiave = |k: &str| obdio::obd::pids::per_key(k).unwrap().pid;
    for k in [
        "giri",
        "velocita",
        "temp_refrigerante",
        "temp_aspirazione",
        "map",
        "pressione_barometrica",
        "tensione_modulo",
        "td5_temp_gasolio",
        "td5_pedale_1",
        "td5_iniettore_5",
        "td5_frizione",
    ] {
        assert!(info.unione.contains(&chiave(k)), "manca {k}");
    }
    assert!(!info.unione.contains(&chiave("td5_egr")));
    assert!(!info.unione.contains(&chiave("carico")));
    // Il registro dice come si è aperta la sessione.
    let testo = registro.join("\n");
    assert!(
        testo.contains("→ STIFI 25, 25, 8113F7810C   ← 03 C1 57 8F AA"),
        "{testo}"
    );
    assert!(testo.contains("→ 0210A0   ← 02 50 A0 F2"), "{testo}");

    let chiavi: Vec<String> = [
        "giri",
        "temp_refrigerante",
        "td5_temp_gasolio",
        "map",
        "td5_pedale_2",
        "td5_egr",
        "velocita",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    tx.send(Comando::AvviaPolling {
        chiavi,
        prioritarie: vec!["giri".into()],
    })
    .unwrap();
    tx.send(Comando::AvviaRegistrazione).unwrap();
    let percorso = attendi(&rx, 10, |e| match e {
        Evento::RegistrazioneAvviata { percorso, .. } => Some(percorso),
        _ => None,
    });
    let mut valori: BTreeMap<String, f64> = BTreeMap::new();
    let mut conteggi: BTreeMap<String, usize> = BTreeMap::new();
    let fine = Instant::now() + Duration::from_millis(1200);
    while Instant::now() < fine {
        if let Ok(Evento::Campioni(v)) = rx.recv_timeout(Duration::from_millis(200)) {
            for c in v {
                assert_eq!(c.stato == Stato::Ok, c.valore.is_some(), "{c:?}");
                if let Some(x) = c.valore {
                    valori.insert(c.key.clone(), x);
                }
                *conteggi.entry(c.key).or_default() += 1;
            }
        }
    }
    assert_eq!(valori["giri"], 744.0);
    assert!((valori["temp_refrigerante"] - 70.6).abs() < 1e-9);
    assert!((valori["td5_temp_gasolio"] - 64.0).abs() < 1e-9);
    assert!((valori["map"] - 102.17).abs() < 1e-9);
    assert!((valori["td5_pedale_2"] - 4.42).abs() < 1e-9);
    assert_eq!(valori["velocita"], 0.0);
    // L'EGR non fornito non compare mai, nemmeno come zero.
    assert!(!conteggi.contains_key("td5_egr"));
    // La distanza parziale si calcola anche dal Td5.
    assert!(conteggi.contains_key(obdio::obd::pids::CHIAVE_DISTANZA_PARZIALE));
    // Priorità ai giri.
    assert!(conteggi["giri"] > conteggi["map"]);

    // Diagnosi: guasti del Td5, VIN solo in memoria, nessuna cancellazione.
    tx.send(Comando::FermaPolling).unwrap();
    tx.send(Comando::LeggiDiagnosi).unwrap();
    let d = attendi(&rx, 20, |e| match e {
        Evento::Registro(r) => {
            registro.extend(r);
            None
        }
        Evento::Diagnosi(d) => Some(*d),
        _ => None,
    });
    let codici: Vec<&str> = d.dtc.iter().map(|x| x.codice.as_str()).collect();
    assert_eq!(codici, ["02-4", "32-1"]);
    assert_eq!(d.vin.as_deref(), Some(VIN_PROVA));
    assert!(d.td5.is_some());
    tx.send(Comando::CancellaDtc(
        ConfermaCancellazione::utente_ha_confermato(),
    ))
    .unwrap();
    let e = attendi(&rx, 10, |e| match e {
        Evento::Errore(e) => Some(e),
        _ => None,
    });
    assert!(
        matches!(&e, ErroreUtente::CancellazioneNonEseguita(m) if m.contains("sola lettura")),
        "{e:?}"
    );

    tx.send(Comando::FermaRegistrazione).unwrap();
    let (chiuso, csv) = attendi(&rx, 10, |e| match e {
        Evento::RegistrazioneFermata { percorso, csv, .. } => Some((percorso, csv)),
        Evento::Registro(r) => {
            registro.extend(r);
            None
        }
        _ => None,
    });
    assert_eq!(chiuso, percorso);
    tx.send(Comando::Scollega).unwrap();
    attendi(&rx, 10, |e| match e {
        Evento::Scollegato { .. } => Some(()),
        Evento::Registro(r) => {
            registro.extend(r);
            None
        }
        _ => None,
    });
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();

    // La sessione si riapre senza veicolo, con i parametri del Td5.
    let s = recording::carica(&percorso).unwrap();
    assert!(s.intestazione.demo);
    assert!(s.intestazione.protocollo.contains("Td5"));
    assert!(
        s.campioni
            .iter()
            .any(|c| c.key == "td5_temp_gasolio" && c.ecu == "Td5")
    );
    let p = obdio::riproduzione::presenti(&s);
    assert!(p.unione.contains(&chiave("td5_temp_gasolio")));
    let rilette = s.diagnosi.as_ref().unwrap();
    assert!(rilette.vin.is_none());
    assert!(rilette.td5.is_some());

    // VIN mai nei registri né nei file.
    let testo_registro = registro.join("\n");
    assert!(testo_registro.contains("021A87   ← (risposta omessa dal registro)"));
    let testo_sessione = std::fs::read_to_string(&percorso).unwrap();
    let testo_csv = std::fs::read_to_string(csv.expect("CSV non scritto")).unwrap();
    for t in [&testo_registro, &testo_sessione, &testo_csv] {
        assert!(!t.contains(VIN_PROVA));
        assert!(!t.contains("54 44 43 49 4E 51"));
    }
    assert!(testo_csv.contains("td5_temp_gasolio"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_riposo_il_thread_di_lavoro_tiene_viva_la_sessione() {
    let dir = cartella("riposo");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let sim = SimulatoreTd5::new().con_scadenza(Duration::from_millis(1600));
    let c = sim.controllo();
    tx.send(Comando::Collega(td5(sim))).unwrap();
    attendi(&rx, 20, |e| match e {
        Evento::Connesso(_) => Some(()),
        Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
        _ => None,
    });
    // Nessun comando per 3,5 s: solo il mantenimento del thread di lavoro.
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(c.inizializzazioni(), 1, "la sessione è caduta a riposo");
    let mantenimenti = c
        .messaggi()
        .iter()
        .filter(|m| m.as_slice() == [0x3E, 0x01])
        .count();
    assert!(mantenimenti >= 2, "{mantenimenti}");
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sessione_caduta_durante_la_lettura_e_poi_persa() {
    let dir = cartella("caduta");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let sim = SimulatoreTd5::new().con_inizializzazioni_massime(2);
    let c = sim.controllo();
    tx.send(Comando::Collega(td5(sim))).unwrap();
    attendi(&rx, 20, |e| match e {
        Evento::Connesso(_) => Some(()),
        _ => None,
    });
    tx.send(Comando::AvviaPolling {
        chiavi: vec!["giri".into()],
        prioritarie: vec![],
    })
    .unwrap();
    // Prima caduta: ripristino trasparente, i giri continuano ad arrivare.
    c.caduta_sessione.store(true, Ordering::Relaxed);
    let mut registro = Vec::new();
    attendi(&rx, 10, |e| match e {
        Evento::Registro(r) => {
            registro.extend(r);
            None
        }
        Evento::Campioni(v) => v
            .iter()
            .any(|x| x.key == "giri" && x.stato == Stato::Ok && c.inizializzazioni() == 2)
            .then_some(()),
        Evento::Scollegato { .. } => panic!("scollegato alla prima caduta"),
        _ => None,
    });
    assert!(registro.join("\n").contains("Sessione Td5 caduta"));
    // Seconda caduta: la centralina non si sveglia più, errore esplicito.
    c.caduta_sessione.store(true, Ordering::Relaxed);
    let motivo = attendi(&rx, 10, |e| match e {
        Evento::Scollegato { motivo } => Some(motivo),
        _ => None,
    });
    let motivo = motivo.expect("caduta senza motivo");
    assert!(motivo.contains("non si è riaperta"), "{motivo}");
    assert_eq!(c.inizializzazioni(), 3);
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn adattatore_elm327_rifiutato_con_spiegazione() {
    let dir = cartella("elm");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    tx.send(Comando::Collega(td5(SimulatoreTd5::new().senza_stn())))
        .unwrap();
    let e = attendi(&rx, 20, |e| match e {
        Evento::Errore(e) => Some(e),
        Evento::Connesso(_) => panic!("collegato con un ELM327"),
        _ => None,
    });
    assert!(
        matches!(e, ErroreUtente::AdattatoreNonAdattoTd5(_)),
        "{e:?}"
    );
    assert!(e.messaggio().contains("OBDLink CX"));
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn demo_td5_con_dati_sintetici() {
    let dir = cartella("demo");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    tx.send(Comando::Collega(Sorgente::DemoTd5)).unwrap();
    let info: InfoConn = attendi(&rx, 30, |e| match e {
        Evento::Connesso(i) => Some(*i),
        Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
        _ => None,
    });
    assert!(info.td5 && info.demo);
    assert!(info.avvisi.iter().any(|a| a.contains("Modalità demo")));
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
