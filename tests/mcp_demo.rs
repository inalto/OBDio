//! Conversazione MCP reale: il thread di lavoro vero, con il simulatore, serve
//! un piccolo client HTTP scritto qui con `TcpStream` (nessuna libreria): la
//! stretta di mano, gli strumenti, i rifiuti (token, `Origin`, dimensione) e
//! lo spegnimento.

mod common;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use common::attendi;
use obdio::mcp::{Livello, McpConfig, StatoMcp, StatoMcpInfo};
use obdio::model::{Marker, Sample, Stato};
use obdio::recording::{Intestazione, SessionData};
use obdio::transport::demo::DemoTransport;
use obdio::worker::{self, Comando, Evento, Sorgente};
use serde_json::{Value, json};

const VIN_DEMO: &str = "DEMO0000000000000";
const TOKEN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const META_VERSIONE: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPACITA: &str = "io.modelcontextprotocol/clientCapabilities";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";

// ---------------------------------------------------------------------------
// Thread di lavoro
// ---------------------------------------------------------------------------

fn cartella(nome: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("obdio_mcp_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn config(livello: Livello) -> McpConfig {
    McpConfig {
        attivo: true,
        // Porta libera scelta dal sistema: quella vera arriva con lo stato.
        porta: 0,
        token: TOKEN.into(),
        livello,
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

    /// Accende il server e restituisce la porta su cui ascolta.
    fn accendi(&self, c: McpConfig) -> u16 {
        self.invia(Comando::ConfiguraMcp(c));
        attendi(&self.rx, 10, |e| match e {
            Evento::Mcp(StatoMcpInfo {
                stato: StatoMcp::InAscolto(p),
                ..
            }) => Some(p),
            Evento::Mcp(i) => panic!("server non avviato: {i:?}"),
            _ => None,
        })
    }

    fn collega(&self, demo: DemoTransport) {
        self.invia(Comando::Collega(Sorgente::Trasporto(Box::new(demo))));
        attendi(&self.rx, 20, |e| match e {
            Evento::Connesso(_) => Some(()),
            Evento::Errore(e) => panic!("errore inatteso: {e:?}"),
            _ => None,
        });
    }

    /// Attende uno stato MCP che soddisfa `f`.
    fn attendi_mcp(&self, secondi: u64, f: impl Fn(&StatoMcpInfo) -> bool) -> StatoMcpInfo {
        attendi(&self.rx, secondi, |e| match e {
            Evento::Mcp(i) if f(&i) => Some(i),
            _ => None,
        })
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
// Client HTTP minimo
// ---------------------------------------------------------------------------

struct Risposta {
    stato: u16,
    testata: String,
    corpo: Vec<u8>,
}

impl Risposta {
    fn testo(&self) -> String {
        String::from_utf8_lossy(&self.corpo).into_owned()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.corpo)
            .unwrap_or_else(|e| panic!("corpo non JSON ({e}): {:?}", self.testo()))
    }

    fn intestazione(&self, nome: &str) -> Option<String> {
        self.testata.lines().find_map(|r| {
            let (n, v) = r.split_once(':')?;
            n.eq_ignore_ascii_case(nome).then(|| v.trim().to_string())
        })
    }
}

fn indirizzo(porta: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, porta))
}

/// Invia byte grezzi e legge la risposta fino alla chiusura della connessione.
fn grezza(porta: u16, richiesta: &[u8]) -> Risposta {
    let mut s = TcpStream::connect_timeout(&indirizzo(porta), Duration::from_secs(5))
        .expect("il server non accetta connessioni");
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    s.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(richiesta).unwrap();
    let mut tutto = Vec::new();
    s.read_to_end(&mut tutto).expect("risposta non ricevuta");
    let fine = tutto
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| {
            panic!(
                "risposta senza testata: {:?}",
                String::from_utf8_lossy(&tutto)
            )
        });
    let testata = String::from_utf8_lossy(&tutto[..fine]).into_owned();
    let stato: u16 = testata
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("riga di stato non valida: {testata:?}"));
    let corpo = tutto[fine + 4..].to_vec();
    if let Some(n) = testata.lines().find_map(|r| {
        let (nome, v) = r.split_once(':')?;
        nome.eq_ignore_ascii_case("content-length")
            .then(|| v.trim().parse::<usize>().unwrap())
    }) {
        assert_eq!(n, corpo.len(), "Content-Length non corrisponde al corpo");
    }
    Risposta {
        stato,
        testata,
        corpo,
    }
}

/// POST su `/mcp` con le intestazioni date (oltre a quelle di base).
fn post(porta: u16, intestazioni: &[(&str, &str)], corpo: &[u8]) -> Risposta {
    let mut r = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{porta}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n",
        corpo.len()
    );
    for (n, v) in intestazioni {
        r.push_str(&format!("{n}: {v}\r\n"));
    }
    r.push_str("\r\n");
    let mut byte = r.into_bytes();
    byte.extend_from_slice(corpo);
    grezza(porta, &byte)
}

fn autorizzazione() -> String {
    format!("Bearer {TOKEN}")
}

/// Client delle revisioni con `initialize`.
struct Client {
    porta: u16,
    versione: String,
    id: u64,
    /// Tutto ciò che il server ha risposto, per cercarvi ciò che non deve uscire.
    visto: String,
}

impl Client {
    /// Stretta di mano completa: `initialize`, poi `notifications/initialized`.
    fn collega(porta: u16) -> Client {
        let mut c = Client {
            porta,
            versione: String::new(),
            id: 0,
            visto: String::new(),
        };
        let r = c.richiesta(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "prova-obdio", "version": "0.1" },
            }),
        );
        let ris = &r["result"];
        assert_eq!(ris["protocolVersion"], "2025-11-25");
        assert_eq!(ris["serverInfo"]["name"], "obdio");
        assert_eq!(ris["capabilities"], json!({ "tools": {} }));
        assert!(
            ris["instructions"]
                .as_str()
                .unwrap()
                .contains("NOT available over MCP")
        );
        c.versione = "2025-11-25".into();
        let auth = autorizzazione();
        let n = post(
            porta,
            &[
                ("Authorization", &auth),
                ("MCP-Protocol-Version", "2025-11-25"),
            ],
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
                .to_string()
                .as_bytes(),
        );
        assert_eq!(n.stato, 202);
        assert!(n.corpo.is_empty());
        c
    }

    fn richiesta(&mut self, metodo: &str, params: Value) -> Value {
        self.id += 1;
        let corpo = json!({ "jsonrpc": "2.0", "id": self.id, "method": metodo, "params": params });
        let auth = autorizzazione();
        let mut intestazioni = vec![("Authorization", auth.as_str())];
        if !self.versione.is_empty() {
            intestazioni.push(("MCP-Protocol-Version", self.versione.as_str()));
        }
        let r = post(self.porta, &intestazioni, corpo.to_string().as_bytes());
        assert_eq!(r.stato, 200, "{metodo}: {}", r.testo());
        assert_eq!(
            r.intestazione("content-type").as_deref(),
            Some("application/json")
        );
        self.visto.push_str(&r.testo());
        let v = r.json();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], self.id);
        v
    }

    /// `tools/call`: il risultato completo (con `isError`).
    fn chiama(&mut self, nome: &str, argomenti: Value) -> Value {
        let v = self.richiesta(
            "tools/call",
            json!({ "name": nome, "arguments": argomenti }),
        );
        assert!(v.get("error").is_none(), "{nome}: {v}");
        v["result"].clone()
    }

    /// `tools/call` riuscita: l'oggetto del risultato.
    fn strumento(&mut self, nome: &str, argomenti: Value) -> Value {
        let r = self.chiama(nome, argomenti);
        assert_eq!(r["isError"], false, "{nome}: {r}");
        // Il testo è lo stesso oggetto di `structuredContent`.
        let testo: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(testo, r["structuredContent"], "{nome}");
        testo
    }

    /// `tools/call` con errore di esecuzione: il messaggio.
    fn errore(&mut self, nome: &str, argomenti: Value) -> String {
        let r = self.chiama(nome, argomenti);
        assert_eq!(r["isError"], true, "{nome}: {r}");
        r["content"][0]["text"].as_str().unwrap().to_string()
    }

    fn nomi_strumenti(&mut self) -> Vec<String> {
        self.richiesta("tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }
}

/// Ripete `f` finché restituisce un valore (al più per `secondi`).
fn finche<T>(secondi: u64, cosa: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let scadenza = Instant::now() + Duration::from_secs(secondi);
    loop {
        if let Some(x) = f() {
            return x;
        }
        assert!(Instant::now() < scadenza, "non è successo in tempo: {cosa}");
        std::thread::sleep(Duration::from_millis(40));
    }
}

// ---------------------------------------------------------------------------
// Prove
// ---------------------------------------------------------------------------

#[test]
fn conversazione_completa_con_il_simulatore() {
    let w = Lavoro::avvia("completa");
    let porta = w.accendi(config(Livello::LetturaEAzioni));
    let mut c = Client::collega(porta);

    // Elenco degli strumenti: otto di lettura e sei azioni, nessuno che cancelli.
    let nomi = c.nomi_strumenti();
    assert_eq!(nomi.len(), 14, "{nomi:?}");
    for atteso in [
        "get_status",
        "list_sensors",
        "get_live_values",
        "get_sensor_history",
        "get_diagnosis",
        "lookup_dtc",
        "list_sessions",
        "get_session_summary",
        "read_diagnosis",
        "start_reading",
        "stop_reading",
        "add_marker",
        "start_recording",
        "stop_recording",
    ] {
        assert!(nomi.iter().any(|n| n == atteso), "manca {atteso}");
    }
    assert!(
        !nomi
            .iter()
            .any(|n| n.contains("clear") || n.contains("erase") || n.contains("cancel"))
    );
    assert_eq!(c.richiesta("ping", json!({}))["result"], json!({}));

    // Prima del collegamento: scollegato, nessuna origine, nessuna diagnosi.
    let s = c.strumento("get_status", json!({}));
    assert_eq!(s["connected"], false);
    assert!(s["origin"].is_null());
    assert_eq!(c.strumento("get_diagnosis", json!({}))["available"], false);
    assert!(
        c.errore("read_diagnosis", json!({}))
            .contains("not connected")
    );
    assert_eq!(
        c.strumento("lookup_dtc", json!({ "code": "p0420" }))["kind"],
        "generic"
    );

    // Collegamento al simulatore, poco prima che la sonda a valle smetta di
    // rispondere (fra 48 e 50,5 s del ciclo demo).
    w.collega(demo().con_fase(45.0));
    let s = c.strumento("get_status", json!({}));
    assert_eq!(s["connected"], true);
    assert_eq!(s["connection"], "connected");
    assert_eq!(s["origin"], "demo_simulator");
    assert!(s["origin_note"].as_str().unwrap().contains("SIMULATED"));
    assert_eq!(s["ecus"], json!(["7E8"]));
    assert_eq!(s["reading_active"], false);
    assert_eq!(s["recording"]["active"], false);
    assert_eq!(s["mcp_permission_level"], "read_and_actions");

    let sensori = c.strumento("list_sensors", json!({}));
    let chiavi: Vec<&str> = sensori["sensors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["key"].as_str().unwrap())
        .collect();
    for k in ["giri", "velocita", "o2s2_tensione", "distanza_parziale"] {
        assert!(chiavi.contains(&k), "manca {k} in {chiavi:?}");
    }
    assert_eq!(sensori["origin"], "demo_simulator");

    // Avvio della lettura e registrazione, chiesti dall'assistente.
    let avvio = c.strumento(
        "start_reading",
        json!({ "keys": ["giri", "velocita", "o2s2_tensione", "inesistente"] }),
    );
    assert_eq!(avvio["reading_active"], true);
    assert_eq!(avvio["keys"], json!(["giri", "velocita", "o2s2_tensione"]));
    assert_eq!(avvio["ignored_keys"][0]["key"], "inesistente");
    let reg = c.strumento("start_recording", json!({}));
    assert_eq!(reg["recording_active"], true);
    let file = reg["file"].as_str().unwrap().to_string();
    assert!(
        file.ends_with(".obdio.jsonl") && !file.contains(['/', '\\']),
        "{file}"
    );

    // Valori numerici per le chiavi lette.
    let valori = finche(15, "valori numerici", || {
        let v = c.strumento("get_live_values", json!({ "keys": ["giri", "velocita"] }));
        (v["values"]["giri"]["value"].is_number() && v["values"]["velocita"]["value"].is_number())
            .then_some(v)
    });
    assert_eq!(valori["origin"], "demo_simulator");
    assert_eq!(valori["reading_active"], true);
    let giri = &valori["values"]["giri"];
    assert_eq!(giri["status"], "ok");
    assert_eq!(giri["unit"], "giri/min");
    assert_eq!(giri["ecu"], "7E8");
    assert!(giri["value"].as_f64().unwrap() > 300.0);
    assert!(giri["age_s"].as_f64().unwrap() < 5.0);
    assert!(giri["min"].as_f64().unwrap() <= giri["max"].as_f64().unwrap());
    assert!(giri["mean"].is_number());
    assert!(
        c.strumento("get_status", json!({}))["reading_keys"]
            .as_array()
            .unwrap()
            .contains(&json!("o2s2_tensione"))
    );

    // La lettura fallita della sonda a valle: null con il suo stato, mai zero.
    let muta = finche(20, "lettura fallita della sonda a valle", || {
        let v = c.strumento("get_live_values", json!({ "keys": ["o2s2_tensione"] }));
        let s = v["values"]["o2s2_tensione"].clone();
        (s.is_object() && s["value"].is_null()).then_some(s)
    });
    assert_eq!(muta["status"], "no_response", "{muta}");
    assert!(muta["failed_reads"].as_u64().unwrap() >= 1);
    c.strumento("add_marker", json!({ "text": "Sonda muta" }));
    // Nella storia la lettura fallita resta, come null; le statistiche non
    // scendono a zero per questo.
    let storia = finche(20, "storia con letture valide e fallite", || {
        let h = c.strumento(
            "get_sensor_history",
            json!({ "key": "o2s2_tensione", "seconds": 60, "max_points": 500 }),
        );
        let st = &h["window_statistics"];
        (st["failed_reads"].as_u64().unwrap() >= 1 && st["valid_samples"].as_u64().unwrap() >= 1)
            .then_some(h)
    });
    assert_eq!(storia["downsampled"], false);
    let punti = storia["points"].as_array().unwrap();
    assert!(punti.len() <= 500);
    assert!(
        punti
            .iter()
            .any(|p| p["value"].is_null() && p["status"] == "no_response")
    );
    assert!(punti.iter().any(|p| p["value"].is_number()));
    assert!(storia["window_statistics"]["min"].as_f64().unwrap() > 0.0);
    // Con pochi punti la storia è ridotta, e lo dice.
    let ridotta = finche(20, "storia ridotta", || {
        let h = c.strumento(
            "get_sensor_history",
            json!({ "key": "giri", "seconds": 60, "max_points": 10 }),
        );
        (h["downsampled"] == true).then_some(h)
    });
    assert!(ridotta["points"].as_array().unwrap().len() <= 10);
    assert!(
        ridotta["downsampling"]
            .as_str()
            .unwrap()
            .contains("buckets")
    );

    // Diagnosi letta su richiesta: P0420, mai il VIN.
    let d = c.strumento("read_diagnosis", json!({}));
    assert_eq!(d["available"], true);
    assert_eq!(d["freshly_read"], true);
    assert_eq!(d["origin"], "demo_simulator");
    assert!(
        d["dtcs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["code"] == "P0420"),
        "{}",
        d["dtcs"]
    );
    assert!(d["vin"].is_null());
    assert!(
        d["mil"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["mil_on"] == true)
    );
    assert_eq!(d["freeze_frame"]["present"], true);
    let d2 = c.strumento("get_diagnosis", json!({}));
    assert_eq!(d2["read_at"], d["read_at"]);
    // La lettura riprende dopo la diagnosi.
    assert_eq!(c.strumento("get_status", json!({}))["reading_active"], true);

    // Fine della registrazione: la sessione compare nell'elenco.
    let stop = c.strumento("stop_recording", json!({}));
    assert_eq!(stop["file"], file.as_str());
    assert!(stop["samples"].as_u64().unwrap() > 10);
    let fermo = c.strumento("stop_reading", json!({}));
    assert_eq!(fermo["was_active"], true);
    let elenco = c.strumento("list_sessions", json!({}));
    assert_eq!(elenco["total"], 1);
    assert_eq!(elenco["sessions"][0]["name"], file.as_str());
    assert_eq!(elenco["sessions"][0]["recorded_from"], "demo_simulator");
    let riepilogo = c.strumento("get_session_summary", json!({ "name": file }));
    assert!(
        riepilogo["markers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["label"] == "Sonda muta")
    );
    let sonda = riepilogo["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "o2s2_tensione")
        .expect("sonda a valle nel riepilogo");
    assert!(sonda["failed_reads"].as_u64().unwrap() >= 1);
    assert!(sonda["min"].as_f64().unwrap() > 0.0);
    assert!(
        c.errore(
            "get_session_summary",
            json!({ "name": "../altro.obdio.jsonl" })
        )
        .contains("No recorded session")
    );

    // La cancellazione dei codici non esiste: errore di protocollo, e i
    // codici sono ancora al loro posto.
    for nome in ["clear_dtc", "erase_codes", "cancella_dtc"] {
        let v = c.richiesta("tools/call", json!({ "name": nome, "arguments": {} }));
        assert_eq!(v["error"]["code"], -32602, "{nome}: {v}");
    }
    let dopo = c.strumento("read_diagnosis", json!({}));
    assert!(
        dopo["dtcs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["code"] == "P0420")
    );
    assert_eq!(dopo["freeze_frame"]["present"], true);

    // In tutto ciò che il server ha risposto il VIN non compare mai.
    assert!(c.visto.len() > 10_000);
    assert!(!c.visto.contains(VIN_DEMO));

    // L'interfaccia viene informata: richieste servite e nome del client.
    let info = w.attendi_mcp(5, |i| i.richieste >= 20 && i.ultimo_client.is_some());
    assert_eq!(info.ultimo_client.as_deref(), Some("prova-obdio 0.1"));
    assert_eq!(info.rifiutate, 0);

    // Spento il server, la porta non accetta più connessioni.
    w.invia(Comando::ConfiguraMcp(McpConfig {
        attivo: false,
        ..config(Livello::LetturaEAzioni)
    }));
    w.attendi_mcp(5, |i| i.stato == StatoMcp::Disattivato);
    assert!(
        TcpStream::connect_timeout(&indirizzo(porta), Duration::from_secs(3)).is_err(),
        "la porta {porta} accetta ancora connessioni"
    );
    w.chiudi();
}

#[test]
fn in_sola_lettura_le_azioni_sono_rifiutate() {
    let w = Lavoro::avvia("sola_lettura");
    let porta = w.accendi(config(Livello::SolaLettura));
    let mut c = Client::collega(porta);
    let nomi = c.nomi_strumenti();
    assert_eq!(nomi.len(), 8, "{nomi:?}");
    for azione in [
        "read_diagnosis",
        "start_reading",
        "stop_reading",
        "add_marker",
        "start_recording",
        "stop_recording",
    ] {
        assert!(!nomi.iter().any(|n| n == azione), "{azione} è in elenco");
    }
    w.collega(demo());
    assert_eq!(
        c.strumento("get_status", json!({}))["mcp_permission_level"],
        "read_only"
    );
    for (nome, argomenti) in [
        ("read_diagnosis", json!({})),
        ("start_reading", json!({ "keys": ["giri"] })),
        ("stop_reading", json!({})),
        ("add_marker", json!({ "text": "x" })),
        ("start_recording", json!({})),
        ("stop_recording", json!({})),
    ] {
        let e = c.errore(nome, argomenti);
        assert!(e.contains("read-only"), "{nome}: {e}");
    }
    // Nulla è successo nell'app: né lettura, né registrazione, né diagnosi.
    std::thread::sleep(Duration::from_millis(300));
    let s = c.strumento("get_status", json!({}));
    assert_eq!(s["reading_active"], false);
    assert_eq!(s["recording"]["active"], false);
    assert_eq!(s["diagnosis_available"], false);
    assert!(
        c.strumento("get_live_values", json!({}))["values"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    // La lettura avviata dall'utente nell'app, invece, si vede.
    w.invia(Comando::AvviaPolling {
        chiavi: vec!["giri".into()],
        prioritarie: vec![],
    });
    finche(15, "valore letto dall'app", || {
        c.strumento("get_live_values", json!({}))["values"]["giri"]["value"]
            .is_number()
            .then_some(())
    });
    assert_eq!(
        c.strumento("get_status", json!({}))["reading_keys"],
        json!(["giri"])
    );
    w.chiudi();
}

#[test]
fn richieste_respinte_token_origine_dimensione() {
    let w = Lavoro::avvia("respinte");
    let porta = w.accendi(config(Livello::SolaLettura));
    let auth = autorizzazione();
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string();

    // Con il token giusto passa.
    let ok = post(porta, &[("Authorization", &auth)], ping.as_bytes());
    assert_eq!(ok.stato, 200);
    assert_eq!(ok.json()["result"], json!({}));
    assert_eq!(ok.intestazione("connection").as_deref(), Some("close"));
    assert!(ok.intestazione("access-control-allow-origin").is_none());

    // 401: senza token, con un token sbagliato, con un altro schema.
    let senza = post(porta, &[], ping.as_bytes());
    assert_eq!(senza.stato, 401);
    assert!(
        senza
            .intestazione("www-authenticate")
            .unwrap()
            .starts_with("Bearer")
    );
    assert!(!senza.testo().contains(TOKEN));
    let mut sbagliato = auth.clone();
    sbagliato.pop();
    sbagliato.push('1');
    for a in [sbagliato.as_str(), "Bearer ", "Basic YTpi", TOKEN] {
        let r = post(porta, &[("Authorization", a)], ping.as_bytes());
        assert_eq!(r.stato, 401, "{a}");
        assert!(r.json().get("result").is_none());
    }

    // 403: una pagina web di un altro sito, anche con il token giusto.
    for origine in [
        "https://evil.example",
        "http://localhost.evil.example",
        "null",
    ] {
        let r = post(
            porta,
            &[("Authorization", &auth), ("Origin", origine)],
            ping.as_bytes(),
        );
        assert_eq!(r.stato, 403, "{origine}");
        assert!(r.intestazione("access-control-allow-origin").is_none());
    }
    // Una pagina di questo computer passa e riceve le intestazioni CORS.
    let locale = post(
        porta,
        &[
            ("Authorization", &auth),
            ("Origin", "http://localhost:6274"),
        ],
        ping.as_bytes(),
    );
    assert_eq!(locale.stato, 200);
    assert_eq!(
        locale
            .intestazione("access-control-allow-origin")
            .as_deref(),
        Some("http://localhost:6274")
    );
    // 403: Host che non è questo computer («DNS rebinding»).
    let rebinding = grezza(
        porta,
        format!(
            "POST /mcp HTTP/1.1\r\nHost: evil.example:{porta}\r\nAuthorization: {auth}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{ping}",
            ping.len()
        )
        .as_bytes(),
    );
    assert_eq!(rebinding.stato, 403);

    // 413: corpo troppo grande, respinto senza leggerlo tutto.
    let mut grande = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{porta}\r\nAuthorization: {auth}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        8 * 1024 * 1024
    )
    .into_bytes();
    grande.extend(std::iter::repeat_n(b' ', 2000));
    let r = grezza(porta, &grande);
    assert_eq!(r.stato, 413);
    // 431: testata troppo grande.
    let mut lunga = format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{porta}\r\n").into_bytes();
    while lunga.len() < 20_000 {
        lunga.extend_from_slice(b"X-Riempitivo: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
    }
    assert_eq!(grezza(porta, &lunga).stato, 431);

    // 405 a GET e DELETE (nessun flusso dal server, nessuna sessione), 404
    // fuori da /mcp, 415 se non è JSON, 204 alla verifica di un browser locale.
    for metodo in ["GET", "DELETE"] {
        let r = grezza(
            porta,
            format!(
                "{metodo} /mcp HTTP/1.1\r\nHost: 127.0.0.1:{porta}\r\nAuthorization: {auth}\r\n\
                 Accept: text/event-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        assert_eq!(r.stato, 405, "{metodo}");
        assert_eq!(r.intestazione("allow").as_deref(), Some("POST, OPTIONS"));
    }
    let altrove = grezza(
        porta,
        format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{porta}\r\n\r\n").as_bytes(),
    );
    assert_eq!(altrove.stato, 404);
    let modulo = grezza(
        porta,
        format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{porta}\r\nAuthorization: {auth}\r\n\
             Content-Type: text/plain\r\nContent-Length: {}\r\n\r\n{ping}",
            ping.len()
        )
        .as_bytes(),
    );
    assert_eq!(modulo.stato, 415);
    let preliminare = grezza(
        porta,
        format!(
            "OPTIONS /mcp HTTP/1.1\r\nHost: 127.0.0.1:{porta}\r\nOrigin: http://127.0.0.1:6274\r\n\
             Access-Control-Request-Method: POST\r\n\r\n"
        )
        .as_bytes(),
    );
    assert_eq!(preliminare.stato, 204);
    assert!(
        preliminare
            .intestazione("access-control-allow-headers")
            .unwrap()
            .contains("authorization")
    );

    // Errori JSON-RPC attraverso HTTP: -32700, -32600 (lotto), -32601.
    let rotto = post(porta, &[("Authorization", &auth)], b"{non json");
    assert_eq!(rotto.stato, 400);
    assert_eq!(rotto.json()["error"]["code"], -32700);
    let lotto = post(
        porta,
        &[("Authorization", &auth)],
        format!("[{ping},{ping}]").as_bytes(),
    );
    assert_eq!(lotto.stato, 400);
    assert_eq!(lotto.json()["error"]["code"], -32600);
    let ignoto = post(
        porta,
        &[("Authorization", &auth)],
        json!({ "jsonrpc": "2.0", "id": 5, "method": "prompts/list" })
            .to_string()
            .as_bytes(),
    );
    assert_eq!(ignoto.stato, 200);
    assert_eq!(ignoto.json()["error"]["code"], -32601);
    assert_eq!(ignoto.json()["id"], 5);
    let versione = post(
        porta,
        &[
            ("Authorization", &auth),
            ("MCP-Protocol-Version", "2024-11-05"),
        ],
        ping.as_bytes(),
    );
    assert_eq!(versione.stato, 400);
    assert_eq!(versione.json()["error"]["code"], -32022);

    // Corpo inviato a pezzi (chunked) e in due tempi: accettato.
    let meta = ping.len() / 2;
    let a_pezzi = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost:{porta}\r\nAuthorization: {auth}\r\n\
         Content-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n\
         {:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        meta,
        &ping[..meta],
        ping.len() - meta,
        &ping[meta..]
    );
    let r = grezza(porta, a_pezzi.as_bytes());
    assert_eq!(r.stato, 200, "{}", r.testo());
    assert_eq!(r.json()["result"], json!({}));

    // L'interfaccia sa quante richieste sono state respinte per sicurezza:
    // cinque 401 e quattro 403.
    let info = w.attendi_mcp(5, |i| i.rifiutate >= 9);
    assert_eq!(info.rifiutate, 9);
    assert!(info.ultimo_errore.is_some());
    w.chiudi();
}

#[test]
fn revisione_senza_stato_2026_07_28() {
    let w = Lavoro::avvia("moderna");
    let porta = w.accendi(config(Livello::SolaLettura));
    w.collega(demo());
    let auth = autorizzazione();
    let chiedi = |metodo: &str, nome: Option<&str>, mut params: Value| {
        params["_meta"] = json!({
            META_VERSIONE: "2026-07-28",
            META_CLIENT: { "name": "client-moderno", "version": "3.0" },
            META_CAPACITA: {},
        });
        let corpo = json!({ "jsonrpc": "2.0", "id": "r1", "method": metodo, "params": params });
        let mut h = vec![
            ("Authorization", auth.as_str()),
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", metodo),
        ];
        if let Some(n) = nome {
            h.push(("Mcp-Name", n));
        }
        post(porta, &h, corpo.to_string().as_bytes())
    };

    // Nessuna stretta di mano: la prima richiesta è già quella utile.
    let scoperta = chiedi("server/discover", None, json!({}));
    assert_eq!(scoperta.stato, 200);
    let ris = &scoperta.json()["result"];
    assert_eq!(ris["resultType"], "complete");
    assert_eq!(
        ris["supportedVersions"],
        json!(["2026-07-28", "2025-11-25", "2025-06-18"])
    );
    assert_eq!(ris["capabilities"], json!({ "tools": {} }));
    assert_eq!(
        ris["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "obdio"
    );
    assert!(ris["ttlMs"].is_u64());

    let elenco = chiedi("tools/list", None, json!({}));
    let ris = &elenco.json()["result"];
    assert_eq!(ris["resultType"], "complete");
    assert_eq!(ris["tools"].as_array().unwrap().len(), 8);
    assert_eq!(ris["cacheScope"], "private");

    let stato = chiedi(
        "tools/call",
        Some("get_status"),
        json!({ "name": "get_status" }),
    );
    assert_eq!(stato.stato, 200);
    let ris = &stato.json()["result"];
    assert_eq!(ris["resultType"], "complete");
    assert_eq!(ris["isError"], false);
    assert_eq!(ris["structuredContent"]["origin"], "demo_simulator");
    assert_eq!(stato.json()["id"], "r1");

    // Intestazione e corpo in disaccordo: 400 con -32020.
    let disaccordo = chiedi(
        "tools/call",
        Some("get_diagnosis"),
        json!({ "name": "get_status" }),
    );
    assert_eq!(disaccordo.stato, 400);
    assert_eq!(disaccordo.json()["error"]["code"], -32020);
    let senza_nome = chiedi("tools/call", None, json!({ "name": "get_status" }));
    assert_eq!(senza_nome.json()["error"]["code"], -32020);
    // Metodo non realizzato: 404 con -32601 nel corpo.
    let ignoto = chiedi("resources/list", None, json!({}));
    assert_eq!(ignoto.stato, 404);
    assert_eq!(ignoto.json()["error"]["code"], -32601);
    // Versione non servita: 400 con l'elenco di quelle servite.
    let corpo = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/list",
        "params": { "_meta": { META_VERSIONE: "2027-01-01", META_CAPACITA: {} } },
    });
    let futura = post(
        porta,
        &[
            ("Authorization", &auth),
            ("MCP-Protocol-Version", "2027-01-01"),
            ("Mcp-Method", "tools/list"),
        ],
        corpo.to_string().as_bytes(),
    );
    assert_eq!(futura.stato, 400);
    assert_eq!(futura.json()["error"]["code"], -32022);
    assert_eq!(futura.json()["error"]["data"]["supported"][0], "2026-07-28");
    // Anche qui il token è obbligatorio.
    let corpo = json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover",
        "params": { "_meta": { META_VERSIONE: "2026-07-28", META_CAPACITA: {} } } });
    let senza = post(
        porta,
        &[
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "server/discover"),
        ],
        corpo.to_string().as_bytes(),
    );
    assert_eq!(senza.stato, 401);

    let info = w.attendi_mcp(5, |i| i.ultimo_client.is_some());
    assert_eq!(info.ultimo_client.as_deref(), Some("client-moderno 3.0"));
    w.chiudi();
}

#[test]
fn molte_richieste_insieme_aspettano_il_turno() {
    let w = Lavoro::avvia("insieme");
    let porta = w.accendi(config(Livello::SolaLettura));
    w.collega(demo());
    // Più richieste dei posti disponibili (otto): nessuna viene respinta né
    // troncata, le altre aspettano.
    let fili: Vec<_> = (0..40)
        .map(|i| {
            std::thread::spawn(move || {
                let auth = autorizzazione();
                let corpo = json!({
                    "jsonrpc": "2.0", "id": i, "method": "tools/call",
                    "params": { "name": "get_status", "arguments": {} },
                });
                let r = post(
                    porta,
                    &[("Authorization", &auth)],
                    corpo.to_string().as_bytes(),
                );
                assert_eq!(r.stato, 200, "{}", r.testo());
                let v = r.json();
                assert_eq!(v["id"], i);
                assert_eq!(v["result"]["structuredContent"]["origin"], "demo_simulator");
            })
        })
        .collect();
    for f in fili {
        f.join().unwrap();
    }
    let info = w.attendi_mcp(5, |i| i.richieste >= 40);
    assert_eq!(info.richieste, 40);
    w.chiudi();
}

/// Una sessione minima da rivivere.
fn sessione() -> SessionData {
    let campione = |t: f64, valore: Option<f64>| Sample {
        t,
        ts: "2026-02-03T10:00:00.000+01:00".into(),
        ecu: "7E8".into(),
        key: "giri".into(),
        pid: 0x0C,
        valore,
        unita: "giri/min".into(),
        stato: if valore.is_some() {
            Stato::Ok
        } else {
            Stato::RispostaMancante
        },
    };
    SessionData {
        percorso: PathBuf::from("viaggio_di_prova.obdio.jsonl"),
        intestazione: Intestazione {
            versione: 1,
            creata: "2026-02-03T10:00:00.000+01:00".into(),
            demo: false,
            adattatore: "ELM327 v2.2".into(),
            protocollo: "ISO 15765-4 CAN".into(),
            ecu: vec!["7E8".into()],
            ..Default::default()
        },
        campioni: (0..600)
            .map(|i| campione(i as f64 * 0.1, (i % 50 != 7).then_some(900.0 + i as f64)))
            .collect(),
        marcatori: vec![Marker {
            t: 1.0,
            ts: "2026-02-03T10:00:01.000+01:00".into(),
            etichetta: "Partenza".into(),
        }],
        ..Default::default()
    }
}

#[test]
fn la_riproduzione_non_passa_per_dati_attuali() {
    let w = Lavoro::avvia("riproduzione");
    let porta = w.accendi(config(Livello::LetturaEAzioni));
    let mut c = Client::collega(porta);
    w.invia(Comando::Collega(Sorgente::Riproduzione(Box::new(
        sessione(),
    ))));
    let s = finche(10, "riproduzione avviata", || {
        let s = c.strumento("get_status", json!({}));
        (s["origin"] == "replay" && s["replay"]["duration_s"].is_number()).then_some(s)
    });
    assert!(s["origin_note"].as_str().unwrap().contains("NOT current"));
    assert_eq!(s["replay"]["file"], "viaggio_di_prova.obdio.jsonl");
    assert_eq!(s["replay"]["recorded_from"], "vehicle");
    assert_eq!(s["replay"]["recorded_at"], "2026-02-03T10:00:00.000+01:00");
    assert_eq!(s["reading_active"], true);
    // I valori scorrono e portano l'origine «replay».
    let v = finche(10, "valori della riproduzione", || {
        let v = c.strumento("get_live_values", json!({}));
        v["values"]["giri"].is_object().then_some(v)
    });
    assert_eq!(v["origin"], "replay");
    // Durante la riproduzione le azioni sulla centralina non hanno senso.
    for (nome, argomenti) in [
        ("read_diagnosis", json!({})),
        ("start_reading", json!({ "keys": ["giri"] })),
        ("add_marker", json!({ "text": "x" })),
        ("start_recording", json!({})),
    ] {
        let e = c.errore(nome, argomenti);
        assert!(e.contains("replay"), "{nome}: {e}");
    }
    assert_eq!(c.strumento("get_diagnosis", json!({}))["available"], false);
    // Chiusa la riproduzione i suoi dati spariscono.
    w.invia(Comando::Scollega);
    finche(10, "riproduzione chiusa", || {
        let s = c.strumento("get_status", json!({}));
        (s["connected"] == false).then_some(())
    });
    assert!(
        c.strumento("get_live_values", json!({}))["values"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    w.chiudi();
}

#[test]
fn porta_occupata_riaccensione_e_uscita_senza_blocchi() {
    let w = Lavoro::avvia("porta");
    // Un altro programma occupa la porta: errore chiaro, nessun ripiego.
    let altro = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let occupata = altro.local_addr().unwrap().port();
    w.invia(Comando::ConfiguraMcp(McpConfig {
        porta: occupata,
        ..config(Livello::SolaLettura)
    }));
    let info = w.attendi_mcp(10, |_| true);
    match &info.stato {
        StatoMcp::Errore(e) => {
            assert!(
                e.contains(&format!("la porta {occupata} è già usata")),
                "{e}"
            );
        }
        altro => panic!("atteso un errore, avuto {altro:?}"),
    }
    assert_eq!(
        info.ultimo_errore,
        Some(info.stato.testo()[8..].to_string())
    );
    // Senza token il server non parte.
    w.invia(Comando::ConfiguraMcp(McpConfig {
        token: String::new(),
        ..config(Livello::SolaLettura)
    }));
    assert!(matches!(
        w.attendi_mcp(10, |_| true).stato,
        StatoMcp::Errore(_)
    ));
    drop(altro);

    // Spegnere e riaccendere sulla stessa porta funziona subito.
    let porta = w.accendi(config(Livello::SolaLettura));
    w.invia(Comando::ConfiguraMcp(McpConfig {
        attivo: false,
        ..config(Livello::SolaLettura)
    }));
    w.attendi_mcp(5, |i| i.stato == StatoMcp::Disattivato);
    let di_nuovo = w.accendi(McpConfig {
        porta,
        ..config(Livello::SolaLettura)
    });
    assert_eq!(di_nuovo, porta);
    // Cambiare livello riavvia il server: gli strumenti cambiano.
    let mut c = Client::collega(porta);
    assert_eq!(c.nomi_strumenti().len(), 8);
    let con_azioni = w.accendi(McpConfig {
        porta,
        ..config(Livello::LetturaEAzioni)
    });
    assert_eq!(con_azioni, porta);
    assert_eq!(c.nomi_strumenti().len(), 14);

    // Un client che apre la connessione e non invia nulla non impedisce
    // all'app di chiudersi.
    let muto = TcpStream::connect_timeout(&indirizzo(porta), Duration::from_secs(5)).unwrap();
    let mezzo = TcpStream::connect_timeout(&indirizzo(porta), Duration::from_secs(5)).unwrap();
    (&mezzo)
        .write_all(b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Le")
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let inizio = Instant::now();
    let dir = w.dir.clone();
    w.tx.send(Comando::Esci).unwrap();
    w.handle.join().unwrap();
    assert!(
        inizio.elapsed() < Duration::from_secs(4),
        "uscita lenta: {:?}",
        inizio.elapsed()
    );
    assert!(
        TcpStream::connect_timeout(&indirizzo(porta), Duration::from_secs(3)).is_err(),
        "la porta {porta} accetta ancora connessioni dopo l'uscita"
    );
    drop((muto, mezzo));
    let _ = std::fs::remove_dir_all(dir);
}

/// Non è una prova automatica: tiene acceso per un po' il thread di lavoro
/// vero con il simulatore e il server MCP, per collegarvi a mano un client
/// reale senza aprire l'interfaccia (vedi docs/MCP.md, «Provare senza
/// interfaccia»).
///
/// ```text
/// OBDIO_MCP_PROVA_PORTA=8765 OBDIO_MCP_PROVA_TOKEN=<almeno 32 caratteri> ///   cargo test --test mcp_demo server_per_prova_manuale -- --ignored --nocapture
/// ```
#[test]
#[ignore = "server dimostrativo da avviare a mano"]
fn server_per_prova_manuale() {
    let var = |nome: &str| std::env::var(nome).ok();
    let porta: u16 = var("OBDIO_MCP_PROVA_PORTA")
        .and_then(|p| p.parse().ok())
        .unwrap_or(8765);
    let secondi: u64 = var("OBDIO_MCP_PROVA_SECONDI")
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let livello = if var("OBDIO_MCP_PROVA_AZIONI").is_some() {
        Livello::LetturaEAzioni
    } else {
        Livello::SolaLettura
    };
    let token = var("OBDIO_MCP_PROVA_TOKEN").unwrap_or_else(|| TOKEN.to_string());
    let w = Lavoro::avvia("manuale");
    let porta = w.accendi(McpConfig {
        attivo: true,
        porta,
        token,
        livello,
    });
    w.collega(demo());
    w.invia(Comando::LeggiDiagnosi);
    w.invia(Comando::AvviaPolling {
        chiavi: ["giri", "velocita", "temp_refrigerante", "o2s2_tensione"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        prioritarie: vec![],
    });
    println!("server MCP dimostrativo su http://127.0.0.1:{porta}/mcp per {secondi} s");
    let fine = Instant::now() + Duration::from_secs(secondi);
    let mut ultimo = None;
    while Instant::now() < fine {
        if let Ok(Evento::Mcp(i)) = w.rx.recv_timeout(Duration::from_millis(200)) {
            ultimo = Some(i);
        }
    }
    println!("stato finale: {ultimo:?}");
    w.chiudi();
}
