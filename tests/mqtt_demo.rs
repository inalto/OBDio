//! Conversazione MQTT reale: il thread di lavoro vero, con il simulatore, parla
//! con un piccolo broker di prova in questo stesso processo (socket TCP su
//! 127.0.0.1). Il broker capisce solo quanto serve: CONNECT (con ultima
//! volontà e credenziali), PUBLISH QoS 0 e 1, PINGREQ e DISCONNECT.

mod common;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use common::{Broker, attendi};
use obdio::elm::ConfermaCancellazione;
use obdio::mqtt::{MqttConfig, StatoMqtt};
use obdio::transport::demo::DemoTransport;
use obdio::worker::{self, Comando, Evento, Sorgente};
use serde_json::Value;

const VIN_DEMO: &str = "DEMO0000000000000";

// ---------------------------------------------------------------------------
// Utilità
// ---------------------------------------------------------------------------

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("obdio_mqtt_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn config(porta: u16) -> MqttConfig {
    MqttConfig {
        attivo: true,
        host: "127.0.0.1".into(),
        porta,
        utente: "mario".into(),
        password: "segreta".into(),
        id_client: "test-obdio".into(),
        veicolo: "auto".into(),
        intervallo_sensore_ms: 0,
        intervallo_istantanea_ms: 100,
        ..Default::default()
    }
}

struct Lavoro {
    tx: Sender<Comando>,
    rx: Receiver<Evento>,
    handle: JoinHandle<()>,
    dir: PathBuf,
}

impl Lavoro {
    fn avvia(nome: &str) -> Lavoro {
        let dir = cartella(nome);
        let (tx, rx, handle) = worker::avvia(dir.clone(), || {});
        Lavoro {
            tx,
            rx,
            handle,
            dir,
        }
    }

    fn invia(&self, c: Comando) {
        self.tx.send(c).unwrap();
    }

    fn collega(&self, demo: DemoTransport) {
        self.invia(Comando::Collega(Sorgente::Trasporto(Box::new(demo))));
        attendi(&self.rx, 20, |e| match e {
            Evento::Connesso(_) => Some(()),
            Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
            _ => None,
        });
    }

    fn polling(&self, chiavi: &[&str]) {
        self.invia(Comando::AvviaPolling {
            chiavi: chiavi.iter().map(|s| s.to_string()).collect(),
            prioritarie: vec![],
        });
    }

    /// Attende lo stato MQTT che soddisfa `f`.
    fn attendi_mqtt(&self, secondi: u64, f: impl Fn(&StatoMqtt) -> bool) {
        attendi(&self.rx, secondi, |e| match e {
            Evento::Mqtt(i) if f(&i.stato) => Some(()),
            _ => None,
        });
    }

    fn chiudi(self) {
        self.tx.send(Comando::Esci).unwrap();
        self.handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn demo() -> DemoTransport {
    DemoTransport::con_latenza(Duration::from_millis(3))
}

// ---------------------------------------------------------------------------
// Prove
// ---------------------------------------------------------------------------

#[test]
fn conversazione_completa_con_il_simulatore() {
    let broker = Broker::avvia(0);
    let w = Lavoro::avvia("completa");
    w.invia(Comando::ConfiguraMqtt(config(broker.porta)));

    // CONNECT: credenziali e ultima volontà trattenuta «offline».
    let c = broker.attendi(10, |r| r.connect.first().cloned());
    assert_eq!(c.client_id, "test-obdio");
    assert!(c.pulita);
    assert_eq!(c.utente.as_deref(), Some("mario"));
    assert_eq!(c.password.as_deref(), Some("segreta"));
    let will = c.will.expect("manca l'ultima volontà");
    assert_eq!(will.argomento, "obdio/auto/stato");
    assert_eq!(will.payload, br#"{"stato":"offline"}"#);
    assert!(will.trattenuto);
    assert_eq!(will.qos, 0);
    w.attendi_mqtt(10, |s| *s == StatoMqtt::Connesso);

    // Senza collegamento al veicolo lo stato trattenuto è «offline».
    let p = broker.pubblicazione(10, "obdio/auto/stato", |_| true);
    assert_eq!(p.json()["stato"], "offline");
    assert!(p.trattenuto);

    // Collegamento al simulatore: stato online, origine sintetica.
    w.collega(demo());
    let online = broker.pubblicazione(10, "obdio/auto/stato", |p| p.testo().contains("online"));
    assert!(online.trattenuto);
    let j = online.json();
    assert_eq!(j["stato"], "online");
    assert_eq!(j["origine"], "demo_sintetico");
    assert_eq!(j["app"], "OBDio");
    assert!(j["versione"].is_string());

    // Sensori: un numero in testo, non trattenuto.
    let chiavi = ["giri", "velocita", "temp_refrigerante", "carico"];
    w.polling(&chiavi);
    for k in chiavi {
        let p = broker.pubblicazione(15, &format!("obdio/auto/sensori/{k}"), |_| true);
        assert!(!p.trattenuto, "{k}");
        let v: f64 = p.testo().parse().unwrap_or_else(|_| {
            panic!("payload di {k} non numerico: {:?}", p.testo());
        });
        assert!(v.is_finite() && v >= 0.0, "{k}: {v}");
    }

    // Istantanea: JSON con le chiavi lette.
    let ist = broker.attendi(15, |r| {
        r.pubblicazioni
            .iter()
            .filter(|p| p.argomento == "obdio/auto/istantanea")
            .map(|p| p.json())
            .find(|j| chiavi.iter().all(|k| j["valori"].get(*k).is_some()))
    });
    assert_eq!(ist["origine"], "demo_sintetico");
    assert!(ist["ts"].as_str().unwrap().contains('T'));
    for k in chiavi {
        let v = &ist["valori"][k];
        assert_eq!(v["stato"], "ok", "{k}");
        assert!(v["valore"].is_number(), "{k}");
        assert_eq!(v["ecu"], "7E8", "{k}");
        assert!(v["unita"].is_string());
    }
    assert_eq!(ist["valori"]["velocita"]["unita"], "km/h");

    // Diagnosi: P0420, trattenuta, senza VIN.
    w.invia(Comando::LeggiDiagnosi);
    let d = broker.pubblicazione(20, "obdio/auto/diagnosi", |_| true);
    assert!(d.trattenuto);
    assert!(d.testo().contains("P0420"));
    assert!(!d.testo().contains(VIN_DEMO));
    assert_eq!(d.json()["dtc"][0]["codice"], "P0420");
    assert_eq!(d.json()["demo"], true);

    // Marcatore e cancellazione dei codici.
    w.invia(Comando::Marcatore("prova".into()));
    let m = broker.pubblicazione(10, "obdio/auto/evento", |p| p.testo().contains("marcatore"));
    assert!(!m.trattenuto);
    assert_eq!(m.json()["etichetta"], "prova");
    w.invia(Comando::CancellaDtc(
        ConfermaCancellazione::utente_ha_confermato(),
    ));
    let e = broker.pubblicazione(20, "obdio/auto/evento", |p| {
        p.testo().contains("codici_cancellati")
    });
    assert_eq!(e.json()["esito"], "accettata");
    assert_eq!(e.json()["origine"], "demo_sintetico");
    // La diagnosi riletta dopo la cancellazione sostituisce la precedente.
    broker.attendi(20, |r| {
        let n = r
            .pubblicazioni
            .iter()
            .filter(|p| p.argomento == "obdio/auto/diagnosi")
            .count();
        (n >= 3).then_some(())
    });

    // Il VIN non compare in nessun messaggio.
    assert!(
        broker
            .tutte()
            .iter()
            .all(|p| !p.testo().contains(VIN_DEMO) && !p.argomento.contains(VIN_DEMO))
    );

    // Scollegando il veicolo lo stato torna «offline».
    w.invia(Comando::Scollega);
    broker.attendi(10, |r| {
        r.pubblicazioni
            .iter()
            .rev()
            .find(|p| p.argomento == "obdio/auto/stato")
            .filter(|p| p.testo().contains("offline"))
            .map(|_| ())
    });

    // Spegnere MQTT pubblica «offline» e chiude con DISCONNECT.
    w.invia(Comando::ConfiguraMqtt(MqttConfig {
        attivo: false,
        ..config(broker.porta)
    }));
    w.attendi_mqtt(10, |s| *s == StatoMqtt::Disattivato);
    broker.attendi(10, |r| (r.disconnect >= 1).then_some(()));
    let ultimo = broker
        .tutte()
        .into_iter()
        .rev()
        .find(|p| p.argomento == "obdio/auto/stato")
        .unwrap();
    assert_eq!(ultimo.json()["stato"], "offline");
    assert!(ultimo.trattenuto);
    w.chiudi();
}

#[test]
fn valore_assente_e_null_con_il_suo_stato() {
    let broker = Broker::avvia(0);
    let w = Lavoro::avvia("assente");
    w.invia(Comando::ConfiguraMqtt(config(broker.porta)));
    w.attendi_mqtt(10, |s| *s == StatoMqtt::Connesso);
    // La sonda a valle non risponde per 2,5 s attorno al secondo 48–50,5.
    w.collega(demo().con_fase(48.0));
    w.polling(&["giri", "o2s2_tensione"]);

    let ist = broker.attendi(15, |r| {
        r.pubblicazioni
            .iter()
            .filter(|p| p.argomento == "obdio/auto/istantanea")
            .map(|p| p.json())
            .find(|j| {
                j["valori"]["o2s2_tensione"]
                    .get("valore")
                    .is_some_and(Value::is_null)
            })
    });
    let o2 = &ist["valori"]["o2s2_tensione"];
    assert_eq!(o2["stato"], "risposta_mancante");
    assert!(o2.get("valore").is_some(), "il campo c'è, con null");
    assert_eq!(ist["valori"]["giri"]["stato"], "ok");

    // Sul sensore non esce nulla per il valore assente, e mai uno zero.
    for p in broker.tutte() {
        if p.argomento == "obdio/auto/sensori/o2s2_tensione" {
            let v: f64 = p.testo().parse().expect("solo numeri");
            assert!(v > 0.04, "valore sospetto {v}");
        }
    }
    w.chiudi();
}

#[test]
fn discovery_home_assistant_trattenuta() {
    let broker = Broker::avvia(0);
    let w = Lavoro::avvia("discovery");
    w.invia(Comando::ConfiguraMqtt(MqttConfig {
        discovery_home_assistant: true,
        ..config(broker.porta)
    }));
    w.attendi_mqtt(10, |s| *s == StatoMqtt::Connesso);
    w.collega(demo());
    w.polling(&["giri", "temp_refrigerante"]);

    let p = broker.pubblicazione(15, "homeassistant/sensor/obdio_auto_giri/config", |_| true);
    assert!(p.trattenuto);
    let j = p.json();
    assert_eq!(j["unique_id"], "obdio_auto_giri");
    assert_eq!(j["state_topic"], "obdio/auto/sensori/giri");
    assert_eq!(j["unit_of_measurement"], "giri/min");
    assert_eq!(j["availability_topic"], "obdio/auto/stato");
    assert_eq!(j["availability_template"], "{{ value_json.stato }}");
    assert_eq!(j["device"]["identifiers"][0], "obdio_auto");
    assert_eq!(j["device"]["manufacturer"], "OBDio");
    assert_eq!(j["state_class"], "measurement");

    let t = broker.pubblicazione(
        15,
        "homeassistant/sensor/obdio_auto_temp_refrigerante/config",
        |_| true,
    );
    assert_eq!(t.json()["device_class"], "temperature");
    assert!(t.trattenuto);

    // Senza discovery non esce nulla sul prefisso di Home Assistant: lo prova
    // la prova «conversazione_completa», che non ne ha.
    w.chiudi();
}

#[test]
fn qos_1_con_conferme_del_broker() {
    let broker = Broker::avvia(0);
    let w = Lavoro::avvia("qos1");
    w.invia(Comando::ConfiguraMqtt(MqttConfig {
        qos: 1,
        ..config(broker.porta)
    }));
    let c = broker.attendi(10, |r| r.connect.first().cloned());
    assert_eq!(c.will.unwrap().qos, 1);
    w.collega(demo());
    w.polling(&["giri", "velocita", "carico"]);
    // Più dei 100 messaggi «in volo» ammessi: senza PUBACK la coda si fermerebbe.
    broker.attendi(20, |r| {
        let n = r
            .pubblicazioni
            .iter()
            .filter(|p| p.argomento.starts_with("obdio/auto/sensori/"))
            .count();
        (n >= 250).then_some(())
    });
    let tutte = broker.tutte();
    assert!(
        tutte
            .iter()
            .filter(|p| p.argomento.starts_with("obdio/auto/sensori/"))
            .all(|p| p.qos == 1)
    );
    assert!(
        tutte
            .iter()
            .filter(|p| p.argomento == "obdio/auto/stato")
            .all(|p| p.qos == 1 && p.trattenuto)
    );
    w.chiudi();
}

#[test]
fn broker_che_cade_il_polling_continua_e_si_riconnette() {
    let mut broker = Broker::avvia(0);
    let porta = broker.porta;
    let w = Lavoro::avvia("caduta");
    w.invia(Comando::ConfiguraMqtt(config(porta)));
    w.collega(demo());
    w.polling(&["giri", "velocita"]);
    w.attendi_mqtt(10, |s| *s == StatoMqtt::Connesso);
    broker.pubblicazione(15, "obdio/auto/sensori/giri", |_| true);

    // Il broker sparisce a metà: i campioni continuano ad arrivare alla GUI.
    broker.ferma();
    let mut dopo = 0usize;
    let mut visto_problema = false;
    let mut scartati = 0u64;
    attendi(&w.rx, 30, |e| {
        match e {
            Evento::Campioni(v) => dopo += v.len(),
            Evento::Mqtt(i) => {
                visto_problema |= matches!(i.stato, StatoMqtt::Errore(_) | StatoMqtt::Connessione);
                scartati = scartati.max(i.scartati);
            }
            Evento::Scollegato { .. } => panic!("il polling non doveva fermarsi"),
            Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
            _ => {}
        }
        (dopo >= 60 && visto_problema && scartati > 0).then_some(())
    });

    // Un nuovo broker sulla stessa porta: ritorna «connesso» e riparte tutto.
    let nuovo = Broker::avvia(porta);
    let mut ancora = 0usize;
    attendi(&w.rx, 60, |e| match e {
        Evento::Campioni(v) => {
            ancora += v.len();
            None
        }
        Evento::Scollegato { .. } => panic!("il polling non doveva fermarsi"),
        Evento::Mqtt(i) if i.stato == StatoMqtt::Connesso => Some(()),
        _ => None,
    });
    assert!(
        ancora > 0,
        "i campioni non devono fermarsi durante l'attesa"
    );
    let c = nuovo.attendi(10, |r| r.connect.first().cloned());
    assert_eq!(c.utente.as_deref(), Some("mario"));
    // Lo stato trattenuto viene rinviato alla riconnessione...
    let stato = nuovo.pubblicazione(10, "obdio/auto/stato", |_| true);
    assert!(stato.trattenuto);
    assert_eq!(stato.json()["origine"], "demo_sintetico");
    // ...e i sensori ricominciano.
    nuovo.pubblicazione(15, "obdio/auto/sensori/giri", |_| true);
    w.chiudi();
}

#[test]
fn spegnere_mqtt_con_broker_muto_non_blocca() {
    // Accetta la connessione TCP ma non risponde mai al CONNECT.
    let ascolto = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let porta = ascolto.local_addr().unwrap().port();
    ascolto.set_nonblocking(true).unwrap();
    let w = Lavoro::avvia("muto");
    w.invia(Comando::ConfiguraMqtt(config(porta)));
    let scadenza = Instant::now() + Duration::from_secs(10);
    let _muta = loop {
        match ascolto.accept() {
            Ok((s, _)) => break s,
            Err(_) if Instant::now() < scadenza => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => panic!("il client non si è collegato: {e}"),
        }
    };
    // Il polling non risente del broker muto.
    w.collega(demo());
    w.polling(&["giri"]);
    let mut visti = 0;
    attendi(&w.rx, 10, |e| {
        if let Evento::Campioni(v) = e {
            visti += v.len();
        }
        (visti >= 20).then_some(())
    });

    let inizio = Instant::now();
    w.invia(Comando::ConfiguraMqtt(MqttConfig {
        attivo: false,
        ..config(porta)
    }));
    w.attendi_mqtt(10, |s| *s == StatoMqtt::Disattivato);
    assert!(
        inizio.elapsed() < Duration::from_secs(5),
        "la chiusura ha atteso {:?}",
        inizio.elapsed()
    );
    // Anche l'uscita dell'app con MQTT attivo e broker muto è limitata.
    w.invia(Comando::ConfiguraMqtt(config(porta)));
    std::thread::sleep(Duration::from_millis(300));
    let inizio = Instant::now();
    w.chiudi();
    assert!(
        inizio.elapsed() < Duration::from_secs(6),
        "l'uscita ha atteso {:?}",
        inizio.elapsed()
    );
}

#[test]
fn configurazione_non_valida_dice_perche() {
    let w = Lavoro::avvia("non_valida");
    w.invia(Comando::ConfiguraMqtt(MqttConfig {
        attivo: true,
        ..Default::default()
    }));
    let info = attendi(&w.rx, 10, |e| match e {
        Evento::Mqtt(i) => Some(i),
        _ => None,
    });
    let StatoMqtt::Errore(m) = info.stato else {
        panic!("atteso un errore: {:?}", info.stato)
    };
    assert!(m.contains("indirizzo del broker"), "{m}");
    w.chiudi();
}
