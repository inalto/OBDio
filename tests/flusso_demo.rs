//! Flusso completo in demo attraverso il thread di lavoro reale:
//! collegamento, lettura P0420, polling, registrazione, CSV, riapertura.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use obdio::diagnosis::StatoDtc;
use obdio::model::Stato;
use obdio::obd::pids::PresetP0420;
use obdio::recording;
use obdio::summary;
use obdio::transport::copione::Copione;
use obdio::transport::demo::DemoTransport;
use obdio::worker::{self, Comando, ErroreUtente, Evento, InfoConn, Sorgente, StatoConn};

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("obdio_flusso_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Attende il primo evento per cui `f` restituisce un valore.
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

#[test]
fn flusso_completo_in_demo() {
    let dir = cartella("completo");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let demo = DemoTransport::con_latenza(Duration::from_millis(3));
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(demo))))
        .unwrap();

    // Collegamento: protocollo ed ECU del caso di riferimento.
    let info = connessione(&rx);
    assert!(info.demo);
    assert_eq!(info.protocollo_numero, 6);
    assert!(info.protocollo.contains("11 bit") && info.protocollo.contains("500 kbit/s"));
    assert_eq!(info.ecu, vec!["7E8".to_string()]);
    assert_eq!(info.adattatore, "ELM327 v1.5");

    // Diagnosi: P0420 in un'unica riga con tre stati e fermo immagine.
    tx.send(Comando::LeggiDiagnosi).unwrap();
    let mut stati = Vec::new();
    let d = attendi(&rx, 20, |e| match e {
        Evento::Stato(s) => {
            stati.push(s);
            None
        }
        Evento::Diagnosi(d) => Some(*d),
        _ => None,
    });
    assert!(stati.contains(&StatoConn::Diagnosi));
    assert!(d.demo);
    assert_eq!(d.dtc.len(), 1);
    assert_eq!(d.dtc[0].codice, "P0420");
    assert_eq!(d.dtc[0].ecu, "7E8");
    assert_eq!(
        d.dtc[0].stati,
        vec![
            StatoDtc::Confermato,
            StatoDtc::InSospeso,
            StatoDtc::Permanente
        ]
    );
    assert!(d.prontezza[0].mil_accesa);
    assert_eq!(d.prontezza[0].numero_dtc, 1);

    assert!(d.fermo.presente);
    assert_eq!(d.fermo.dtc.as_deref(), Some("P0420"));
    let fermo: BTreeMap<&str, f64> = d
        .fermo
        .righe
        .iter()
        .filter_map(|r| r.valore.map(|v| (r.key.as_str(), v)))
        .collect();
    assert_eq!(fermo["velocita"], 69.0);
    assert_eq!(fermo["giri"], 2128.0);
    assert_eq!(fermo["temp_refrigerante"], 83.0);
    assert_eq!(format!("{:.2}", fermo["stft_b1"]), "0.78");
    assert_eq!(format!("{:.2}", fermo["ltft_b1"]), "8.59");
    assert!(fermo.contains_key("carico") && fermo.contains_key("map"));
    let carburante = d
        .fermo
        .righe
        .iter()
        .find(|r| r.key == "stato_carburante")
        .unwrap();
    assert!(carburante.testo.as_ref().unwrap().contains("Anello chiuso"));
    assert_eq!(d.mode06.test.len(), 1);
    assert!(d.mode06.test[0].scalato.is_some());
    assert!(d.vin.is_some());

    // Riepilogo: senza VIN per impostazione predefinita.
    let vin = d.vin.clone().unwrap();
    let testo = summary::riepilogo(&d, false);
    assert!(testo.contains("MODALITÀ DEMO"));
    assert!(testo.contains("P0420") && testo.contains("confermato, in sospeso, permanente"));
    assert!(testo.contains("2.128 giri/min") && testo.contains("+0,78 %"));
    assert!(testo.contains("+8,59 %") && testo.contains("69 km/h") && testo.contains("83 °C"));
    assert!(!testo.contains(&vin));
    assert!(summary::riepilogo(&d, true).contains(&vin));

    // Selezione «Diagnosi P0420» sui PID realmente dichiarati.
    let preset = PresetP0420::calcola(&info.unione);
    assert_eq!(preset.b1s1.as_deref(), Some("o2s1_lambda"));
    assert_eq!(preset.b1s2.as_deref(), Some("o2s2_tensione"));
    let mut chiavi = preset.sonde();
    chiavi.extend(preset.base.iter().map(|(k, _)| k.clone()));
    chiavi.extend(preset.opzionali.iter().map(|(k, _)| k.clone()));
    tx.send(Comando::AvviaPolling {
        chiavi,
        prioritarie: preset.sonde(),
    })
    .unwrap();
    tx.send(Comando::AvviaRegistrazione).unwrap();
    let percorso = attendi(&rx, 10, |e| match e {
        Evento::RegistrazioneAvviata { percorso, .. } => Some(percorso),
        _ => None,
    });

    let mut conteggi: BTreeMap<String, usize> = BTreeMap::new();
    let fine = Instant::now() + Duration::from_millis(1500);
    let mut marcato = false;
    while Instant::now() < fine {
        if let Ok(Evento::Campioni(v)) = rx.recv_timeout(Duration::from_millis(200)) {
            for c in v {
                assert_eq!(c.stato == Stato::Ok, c.valore.is_some());
                *conteggi.entry(c.key).or_default() += 1;
            }
        }
        if !marcato && conteggi.len() > 3 {
            tx.send(Comando::Marcatore("Minimo".into())).unwrap();
            marcato = true;
        }
    }
    assert!(marcato);
    for k in [
        "o2s1_lambda",
        "o2s2_tensione",
        "stft_b1",
        "ltft_b1",
        "giri",
        "velocita",
        "temp_refrigerante",
        "carico",
        "map",
    ] {
        assert!(conteggi.get(k).copied().unwrap_or(0) > 0, "manca {k}");
    }
    // Priorità alle sonde.
    assert!(conteggi["o2s1_lambda"] > 3 * conteggi["giri"]);

    tx.send(Comando::FermaRegistrazione).unwrap();
    let (chiuso, csv, n) = attendi(&rx, 10, |e| match e {
        Evento::RegistrazioneFermata {
            percorso,
            csv,
            campioni,
        } => Some((percorso, csv, campioni)),
        _ => None,
    });
    assert_eq!(chiuso, percorso);
    assert!(n > 20);
    let csv = csv.expect("CSV non scritto");

    tx.send(Comando::Scollega).unwrap();
    let motivo = attendi(&rx, 10, |e| match e {
        Evento::Scollegato { motivo } => Some(motivo),
        _ => None,
    });
    assert_eq!(motivo, None);
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();

    // Riapertura senza veicolo.
    let s = recording::carica(&percorso).unwrap();
    assert!(s.intestazione.demo);
    assert_eq!(s.campioni.len(), n);
    assert_eq!(s.marcatori.len(), 1);
    assert_eq!(s.marcatori[0].etichetta, "Minimo");
    assert_eq!(s.righe_scartate, 0);
    assert_eq!(s.diagnosi.as_ref().unwrap().dtc[0].codice, "P0420");
    assert!(s.campioni.iter().all(|c| c.t >= 0.0));
    assert!(s.campioni.windows(2).all(|w| w[0].t <= w[1].t));

    // CSV: intestazione, origine sintetica, nessun VIN nei file.
    let testo_csv = std::fs::read_to_string(&csv).unwrap();
    assert!(testo_csv.starts_with('\u{feff}'));
    let mut righe = testo_csv.trim_start_matches('\u{feff}').lines();
    assert_eq!(righe.next().unwrap(), recording::INTESTAZIONE_CSV);
    assert_eq!(testo_csv.lines().count(), 1 + n + 1);
    assert!(righe.all(|r| r.ends_with(",demo_sintetico") && r.split(',').count() == 9));
    assert!(testo_csv.contains(",marcatore,Minimo,,,marcatore,"));
    let testo_sessione = std::fs::read_to_string(&percorso).unwrap();
    assert!(!testo_sessione.contains(&vin) && !testo_csv.contains(&vin));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn distanza_parziale_calcolata_dal_worker_e_registrata() {
    let dir = cartella("distanza");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    // Scenario già a regime (69 km/h) fin dal primo campione.
    let demo = DemoTransport::con_latenza(Duration::from_millis(3)).con_fase(55.0);
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(demo))))
        .unwrap();
    connessione(&rx);

    // La registrazione parte prima della lettura, così il primo campione è registrato.
    tx.send(Comando::AvviaRegistrazione).unwrap();
    let percorso = attendi(&rx, 10, |e| match e {
        Evento::RegistrazioneAvviata { percorso, .. } => Some(percorso),
        _ => None,
    });
    // Si chiede solo la velocità: la distanza è calcolata, mai richiesta.
    tx.send(Comando::AvviaPolling {
        chiavi: vec!["velocita".into()],
        prioritarie: vec![],
    })
    .unwrap();

    let mut distanze: Vec<f64> = Vec::new();
    let mut velocita = 0usize;
    let fine = Instant::now() + Duration::from_millis(2000);
    while Instant::now() < fine {
        if let Ok(Evento::Campioni(v)) = rx.recv_timeout(Duration::from_millis(200)) {
            for c in v {
                match c.key.as_str() {
                    "velocita" => {
                        assert_eq!(c.valore, Some(69.0));
                        velocita += 1;
                    }
                    "distanza_parziale" => {
                        assert_eq!(c.unita, "km");
                        assert_eq!(c.stato, Stato::Ok);
                        assert_eq!(c.ecu, "7E8");
                        distanze.push(c.valore.expect("la distanza ha sempre un valore"));
                    }
                    altro => panic!("parametro non richiesto: {altro}"),
                }
            }
        }
    }
    // Un campione di distanza per ogni velocità valida; riparte da zero.
    assert!(velocita > 10, "troppo pochi campioni: {velocita}");
    assert_eq!(distanze.len(), velocita);
    assert_eq!(distanze[0], 0.0);
    assert!(distanze.windows(2).all(|w| w[0] <= w[1]));
    // 69 km/h per circa 2 s sono circa 38 m; mai più della durata attesa.
    let ultima = *distanze.last().unwrap();
    assert!(ultima > 0.005 && ultima < 0.06, "distanza {ultima} km");

    tx.send(Comando::FermaRegistrazione).unwrap();
    let (chiuso, csv, n) = attendi(&rx, 10, |e| match e {
        Evento::RegistrazioneFermata {
            percorso,
            csv,
            campioni,
        } => Some((percorso, csv, campioni)),
        _ => None,
    });
    assert_eq!(chiuso, percorso);
    let csv = csv.expect("CSV non scritto");
    tx.send(Comando::Scollega).unwrap();
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();

    // La distanza è nella sessione registrata, con gli stessi valori.
    let s = recording::carica(&percorso).unwrap();
    assert_eq!(s.campioni.len(), n);
    let registrate: Vec<f64> = s
        .campioni
        .iter()
        .filter(|c| c.key == "distanza_parziale")
        .map(|c| c.valore.unwrap())
        .collect();
    assert!(registrate.len() >= distanze.len());
    assert!(registrate.windows(2).all(|w| w[0] <= w[1]));
    // Stessi valori, salvo l'ultima cifra della lettura del JSON.
    assert!(
        registrate
            .iter()
            .zip(&distanze)
            .all(|(r, d)| (r - d).abs() < 1e-9)
    );

    // E nel CSV, con unità e stato.
    let testo = std::fs::read_to_string(&csv).unwrap();
    let righe: Vec<&str> = testo
        .lines()
        .filter(|r| r.contains(",distanza_parziale,"))
        .collect();
    assert_eq!(righe.len(), registrate.len());
    assert!(righe.iter().all(
        |r| r.contains(",Distanza parziale calcolata,") && r.contains(",km,ok,demo_sintetico")
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancellazione_dei_codici_in_demo() {
    use obdio::elm::{ConfermaCancellazione, EsitoCancellazione, RispostaCancellazione};

    let dir = cartella("cancellazione");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let demo = DemoTransport::con_latenza(Duration::from_millis(3));
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(demo))))
        .unwrap();
    connessione(&rx);

    // Prima: P0420 nei tre stati.
    tx.send(Comando::LeggiDiagnosi).unwrap();
    let prima = attendi(&rx, 20, |e| match e {
        Evento::Diagnosi(d) => Some(*d),
        _ => None,
    });
    assert_eq!(prima.dtc[0].codice, "P0420");
    assert_eq!(prima.dtc[0].stati.len(), 3);
    let vin = prima.vin.clone().expect("VIN demo");

    // Cancellazione confermata.
    tx.send(Comando::CancellaDtc(
        ConfermaCancellazione::utente_ha_confermato(),
    ))
    .unwrap();
    let mut stati = Vec::new();
    let mut marcatori = Vec::new();
    let risultato = attendi(&rx, 20, |e| match e {
        Evento::Stato(s) => {
            stati.push(s);
            None
        }
        Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
        Evento::CancellazioneDtc(r) => Some(*r),
        _ => None,
    });
    let dopo = attendi(&rx, 20, |e| match e {
        Evento::Stato(s) => {
            stati.push(s);
            None
        }
        Evento::Marcatore(m) => {
            marcatori.push(m.etichetta);
            None
        }
        Evento::Diagnosi(d) => Some(*d),
        _ => None,
    });
    attendi(&rx, 10, |e| {
        matches!(e, Evento::Stato(StatoConn::Connesso)).then_some(())
    });
    assert!(stati.contains(&StatoConn::Cancellazione));
    assert!(risultato.demo);
    assert!(risultato.esito.accettata());
    let EsitoCancellazione::Risposte(v) = &risultato.esito else {
        panic!("{:?}", risultato.esito)
    };
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].ecu, "7E8");
    assert_eq!(v[0].risposta, RispostaCancellazione::Accettata);
    assert_eq!(marcatori, vec![worker::MARCATORE_CANCELLAZIONE.to_string()]);

    // Dopo: resta solo il codice permanente, MIL spenta, niente fermo immagine.
    assert_eq!(dopo.dtc.len(), 1);
    assert_eq!(dopo.dtc[0].codice, "P0420");
    assert_eq!(dopo.dtc[0].stati, vec![StatoDtc::Permanente]);
    assert_eq!(dopo.lettura_confermati.stato, Stato::Ok);
    assert_eq!(dopo.lettura_in_sospeso.stato, Stato::Ok);
    assert!(!dopo.prontezza[0].mil_accesa);
    assert_eq!(dopo.prontezza[0].numero_dtc, 0);
    assert!(!dopo.fermo.presente);
    let catalizzatore = dopo.prontezza[0]
        .monitor
        .iter()
        .find(|m| m.nome == "Catalizzatore")
        .unwrap();
    assert!(catalizzatore.disponibile && !catalizzatore.completato);
    assert_eq!(dopo.mode06.test.len(), 1);

    // Copia della diagnosi precedente: P0420 confermato, senza VIN.
    let nome = risultato
        .copia
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        nome.starts_with(recording::PREFISSO_COPIA_DIAGNOSI),
        "{nome}"
    );
    assert!(nome.ends_with("_DEMO.json"), "{nome}");
    assert_eq!(risultato.copia.parent().unwrap(), dir.as_path());
    let copia = std::fs::read_to_string(&risultato.copia).unwrap();
    assert!(copia.contains("P0420"));
    assert!(copia.contains("\"confermato\""));
    assert!(copia.contains("\"in_sospeso\""));
    assert!(!copia.contains(&vin));
    let riletta: obdio::diagnosis::DiagnosisReport = serde_json::from_str(&copia).unwrap();
    assert!(riletta.fermo.presente);
    assert_eq!(riletta.dtc, prima.dtc);

    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancellazione_rifiutata_senza_collegamento() {
    use obdio::elm::ConfermaCancellazione;

    let dir = cartella("cancellazione_scollegato");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    tx.send(Comando::CancellaDtc(
        ConfermaCancellazione::utente_ha_confermato(),
    ))
    .unwrap();
    let e = attendi(&rx, 10, |e| match e {
        Evento::Errore(e) => Some(e),
        Evento::CancellazioneDtc(_) => panic!("non doveva cancellare"),
        _ => None,
    });
    assert!(matches!(e, ErroreUtente::CancellazioneNonEseguita(_)));
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn senza_copia_della_diagnosi_non_si_cancella() {
    use obdio::elm::ConfermaCancellazione;

    let dir = cartella("cancellazione_senza_copia");
    // La «cartella delle sessioni» è un file: la copia non si può scrivere.
    let finta = dir.join("non_una_cartella");
    std::fs::write(&finta, "x").unwrap();
    let (tx, rx, handle) = worker::avvia(finta.clone(), || {});
    let demo = DemoTransport::con_latenza(Duration::from_millis(3));
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(demo))))
        .unwrap();
    connessione(&rx);
    tx.send(Comando::CancellaDtc(
        ConfermaCancellazione::utente_ha_confermato(),
    ))
    .unwrap();
    let mut registro = Vec::new();
    let e = attendi(&rx, 20, |e| match e {
        Evento::Registro(r) => {
            registro.extend(r);
            None
        }
        Evento::Errore(e) => Some(e),
        Evento::CancellazioneDtc(_) => panic!("non doveva cancellare senza copia"),
        _ => None,
    });
    assert!(matches!(e, ErroreUtente::CancellazioneNonEseguita(_)));
    attendi(&rx, 10, |e| {
        if let Evento::Registro(r) = &e {
            registro.extend(r.iter().cloned());
        }
        matches!(e, Evento::Stato(StatoConn::Connesso)).then_some(())
    });
    // La diagnosi è stata letta per la copia, ma il servizio 04 non è partito.
    assert!(registro.iter().any(|r| r.starts_with("→ 03 ")));
    assert!(!registro.iter().any(|r| r.starts_with("→ 04 ")));
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn perdita_del_collegamento_conserva_la_registrazione() {
    let dir = cartella("caduta");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let demo = DemoTransport::con_latenza(Duration::from_millis(3));
    let interruttore = demo.interruttore();
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(demo))))
        .unwrap();
    connessione(&rx);
    tx.send(Comando::AvviaPolling {
        chiavi: vec!["giri".into(), "o2s2_tensione".into()],
        prioritarie: vec![],
    })
    .unwrap();
    tx.send(Comando::AvviaRegistrazione).unwrap();
    attendi(&rx, 10, |e| {
        matches!(e, Evento::RegistrazioneAvviata { .. }).then_some(())
    });
    let mut visti = 0;
    attendi(&rx, 10, |e| {
        if let Evento::Campioni(v) = e {
            visti += v.len();
        }
        (visti >= 10).then_some(())
    });

    interruttore.store(true, Ordering::Relaxed);
    let mut percorso = None;
    let mut polling_fermo = false;
    let mut errore = None;
    let motivo = attendi(&rx, 10, |e| match e {
        Evento::RegistrazioneFermata { percorso: p, .. } => {
            percorso = Some(p);
            None
        }
        Evento::Polling(false) => {
            polling_fermo = true;
            None
        }
        Evento::Errore(e) => {
            errore = Some(e);
            None
        }
        Evento::Scollegato { motivo } => Some(motivo),
        _ => None,
    });
    assert!(
        motivo.is_some(),
        "la caduta deve essere segnalata come tale"
    );
    assert!(polling_fermo);
    assert!(matches!(errore, Some(ErroreUtente::CollegamentoPerso(_))));
    let s = recording::carica(&percorso.expect("registrazione non chiusa")).unwrap();
    assert!(s.campioni.len() >= 10);

    // Nessun altro campione dopo la caduta.
    std::thread::sleep(Duration::from_millis(300));
    assert!(rx.try_iter().all(|e| !matches!(e, Evento::Campioni(_))));
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn errori_distinti_adattatore_e_centralina() {
    let dir = cartella("errori");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let prossimo_errore = |rx: &Receiver<Evento>| {
        attendi(rx, 40, |e| match e {
            Evento::Errore(e) => Some(e),
            Evento::Connesso(_) => panic!("non doveva collegarsi"),
            _ => None,
        })
    };

    // Dispositivo che non parla ELM327.
    let mut c = Copione::new(20);
    c.testo("ATZ", "?\r\r>");
    c.testo("ATE0", "OK\r\r>");
    c.testo("ATI", "?\r\r>");
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(c))))
        .unwrap();
    assert!(matches!(
        prossimo_errore(&rx),
        ErroreUtente::AdattatoreNonRisponde(_)
    ));

    // Adattatore a posto, quadro spento.
    let mut c = Copione::con_inizializzazione(20);
    c.testo("0100", "SEARCHING...\rUNABLE TO CONNECT\r\r>");
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(c))))
        .unwrap();
    assert!(matches!(
        prossimo_errore(&rx),
        ErroreUtente::CentralinaNonRisponde(_)
    ));

    // Senza collegamento i comandi di lettura non fanno nulla.
    tx.send(Comando::LeggiDiagnosi).unwrap();
    tx.send(Comando::AvviaRegistrazione).unwrap();
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    assert!(recording::elenca(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn timeout_ripetuti_fermano_il_polling() {
    use obdio::transport::copione::Battuta;
    let dir = cartella("timeout");
    let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
    let mut c = Copione::con_inizializzazione(20);
    c.testo("0100", "7E8 06 41 00 00 10 00 00\r\r>");
    c.sequenza(
        "010C",
        vec![
            Battuta::Testo("7E8 04 41 0C 21 40\r\r>".into()),
            Battuta::Testo("CAN ERROR\r\r>".into()),
        ],
    );
    tx.send(Comando::Collega(Sorgente::Trasporto(Box::new(c))))
        .unwrap();
    connessione(&rx);
    tx.send(Comando::AvviaPolling {
        chiavi: vec!["giri".into()],
        prioritarie: vec![],
    })
    .unwrap();
    let mut stati = Vec::new();
    let motivo = attendi(&rx, 20, |e| match e {
        Evento::Campioni(v) => {
            stati.extend(v.into_iter().map(|c| (c.stato, c.valore)));
            None
        }
        Evento::Scollegato { motivo } => Some(motivo),
        _ => None,
    });
    assert!(motivo.unwrap().contains("consecutive"));
    assert_eq!(stati[0], (Stato::Ok, Some(2128.0)));
    // Gli errori sono campioni senza valore, mai zeri.
    assert_eq!(stati.len(), 6);
    assert!(
        stati[1..]
            .iter()
            .all(|s| *s == (Stato::ErroreComunicazione, None))
    );
    tx.send(Comando::Esci).unwrap();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
