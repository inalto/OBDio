//! Prove d'integrazione degli aggiornamenti con un server HTTP di prova in
//! questo stesso processo (HTTP in chiaro, ammesso solo dalle impostazioni
//! `ImpostazioniRete::per_prove`): controllo, scaricamento, verifica e
//! sostituzione di un finto eseguibile in una cartella temporanea.
//!
//! Nessuna prova contatta GitHub; quella vera è
//! `cargo run --example controlla_aggiornamenti`.

#![cfg(not(target_os = "android"))]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use obdio::aggiornamenti::sostituzione::{self, DEROGA_SVILUPPO};
use obdio::aggiornamenti::{
    Cliente, ConfigAggiornamenti, ErroreAggiornamento, ImpostazioniRete, Notizia, Servizio,
    Valutazione, Versione, rete, scarica_e_installa, somme, valuta,
};

const PIATTAFORMA: &str = "windows-x64";

/// Le prove di installazione girano in una build di debug: serve la deroga
/// esplicita, che la versione per l'utente non imposta mai.
fn consenti_installazione() {
    static UNA_VOLTA: Once = Once::new();
    UNA_VOLTA.call_once(|| {
        // SAFETY: impostata una sola volta, prima di qualunque richiesta di
        // rete di queste prove; nessun codice C di questo processo legge
        // l'ambiente in parallelo (le letture di Rust passano dal suo lock).
        unsafe { std::env::set_var(DEROGA_SVILUPPO, "1") };
    });
}

// ---------------------------------------------------------------------------
// Server HTTP di prova
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Risposta {
    stato: u16,
    intestazioni: Vec<(String, String)>,
    corpo: Vec<u8>,
    /// Pausa fra un pezzo da 16 kB e l'altro (scaricamento lento).
    pausa: Option<Duration>,
    /// Non risponde affatto (per il tempo massimo).
    muto: bool,
}

impl Risposta {
    fn ok(corpo: impl Into<Vec<u8>>) -> Self {
        Risposta {
            stato: 200,
            intestazioni: Vec::new(),
            corpo: corpo.into(),
            pausa: None,
            muto: false,
        }
    }
    fn stato(stato: u16) -> Self {
        Risposta {
            stato,
            ..Risposta::ok("")
        }
    }
    fn con(mut self, nome: &str, valore: &str) -> Self {
        self.intestazioni.push((nome.into(), valore.into()));
        self
    }
    fn rinvio(verso: &str) -> Self {
        Risposta::stato(302).con("Location", verso)
    }
}

/// Richieste ricevute: percorso e intestazioni (nomi in minuscolo).
type Richieste = Mutex<Vec<(String, Vec<(String, String)>)>>;

struct Server {
    base: String,
    percorsi: Arc<Mutex<HashMap<String, Risposta>>>,
    richieste: Arc<Richieste>,
}

impl Server {
    fn avvia() -> Self {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let percorsi: Arc<Mutex<HashMap<String, Risposta>>> = Arc::default();
        let richieste: Arc<Richieste> = Arc::default();
        let (p, r) = (percorsi.clone(), richieste.clone());
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(conn) = conn else { continue };
                let (p, r) = (p.clone(), r.clone());
                std::thread::spawn(move || servi(conn, &p, &r));
            }
        });
        Server {
            base,
            percorsi,
            richieste,
        }
    }

    fn metti(&self, percorso: &str, r: Risposta) {
        self.percorsi.lock().unwrap().insert(percorso.into(), r);
    }

    fn url(&self, percorso: &str) -> String {
        format!("{}{percorso}", self.base)
    }

    fn richieste_a(&self, prefisso: &str) -> usize {
        self.richieste
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.starts_with(prefisso))
            .count()
    }

    fn cliente(&self) -> Cliente {
        Cliente::new(self.impostazioni())
    }

    fn impostazioni(&self) -> ImpostazioniRete {
        ImpostazioniRete::per_prove(&self.base)
    }
}

fn servi(conn: TcpStream, percorsi: &Mutex<HashMap<String, Risposta>>, richieste: &Richieste) {
    let mut lettore = BufReader::new(conn.try_clone().unwrap());
    let mut prima = String::new();
    if lettore.read_line(&mut prima).is_err() {
        return;
    }
    let percorso = prima.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut intestazioni = Vec::new();
    loop {
        let mut riga = String::new();
        if lettore.read_line(&mut riga).unwrap_or(0) == 0 || riga.trim().is_empty() {
            break;
        }
        if let Some((n, v)) = riga.split_once(':') {
            intestazioni.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    richieste
        .lock()
        .unwrap()
        .push((percorso.clone(), intestazioni));
    let risposta = percorsi
        .lock()
        .unwrap()
        .get(&percorso)
        .cloned()
        .unwrap_or_else(|| Risposta::stato(404));
    let mut conn = conn;
    if risposta.muto {
        std::thread::sleep(Duration::from_secs(5));
        return;
    }
    let mut testa = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        risposta.stato,
        risposta.corpo.len()
    );
    for (n, v) in &risposta.intestazioni {
        testa.push_str(&format!("{n}: {v}\r\n"));
    }
    testa.push_str("\r\n");
    if conn.write_all(testa.as_bytes()).is_err() {
        return;
    }
    match risposta.pausa {
        None => {
            let _ = conn.write_all(&risposta.corpo);
        }
        Some(p) => {
            for pezzo in risposta.corpo.chunks(16 * 1024) {
                if conn.write_all(pezzo).is_err() {
                    return;
                }
                let _ = conn.flush();
                std::thread::sleep(p);
            }
        }
    }
    let _ = conn.flush();
}

// ---------------------------------------------------------------------------
// Rilascio di prova
// ---------------------------------------------------------------------------

/// Una versione sicuramente più nuova di quella in prova.
fn versione_nuova() -> String {
    let v = Versione::corrente();
    format!("{}.0.0", v.maggiore + 1)
}

/// Contenuto del finto eseguibile nuovo: abbastanza grande da arrivare in
/// più pezzi.
fn binario_nuovo() -> Vec<u8> {
    (0..300_000u32).map(|i| (i % 251) as u8).collect()
}

fn json_rilascio(s: &Server, etichetta: &str, dimensione_exe: usize, con_somme: bool) -> String {
    let v = etichetta.trim_start_matches('v');
    let mut risorse = vec![
        format!(
            r#"{{"name":"OBDio-{v}-windows-x64.zip","size":10,"state":"uploaded","browser_download_url":"{}"}}"#,
            s.url(&format!("/download/{etichetta}/OBDio-{v}-windows-x64.zip"))
        ),
        format!(
            r#"{{"name":"OBDio-{v}-windows-x64.exe","size":{dimensione_exe},"state":"uploaded","browser_download_url":"{}"}}"#,
            s.url(&format!("/download/{etichetta}/OBDio-{v}-windows-x64.exe"))
        ),
    ];
    if con_somme {
        risorse.push(format!(
            r#"{{"name":"SHA256SUMS.txt","size":200,"state":"uploaded","browser_download_url":"{}"}}"#,
            s.url(&format!("/download/{etichetta}/SHA256SUMS.txt"))
        ));
    }
    format!(
        r#"{{"tag_name":"{etichetta}","name":"OBDio {v}","body":"Note","draft":false,"prerelease":false,
            "html_url":"https://github.com/inalto/OBDio/releases/tag/{etichetta}",
            "published_at":"2026-12-01T10:00:00Z","assets":[{}]}}"#,
        risorse.join(",")
    )
}

/// Pubblica sul server un rilascio completo: API, eseguibile (con un rinvio
/// come fa github.com) e somme. `alterato` serve un binario diverso da
/// quello delle somme.
fn pubblica(s: &Server, alterato: bool) -> (String, Vec<u8>) {
    let etichetta = format!("v{}", versione_nuova());
    let v = versione_nuova();
    let bin = binario_nuovo();
    let nome = format!("OBDio-{v}-windows-x64.exe");
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::ok(json_rilascio(s, &etichetta, bin.len(), true))
            .con("Content-Type", "application/json"),
    );
    s.metti(
        &format!("/download/{etichetta}/{nome}"),
        Risposta::rinvio(&s.url("/oggetti/eseguibile?firma=abc")),
    );
    let mut servito = bin.clone();
    if alterato {
        servito[1234] ^= 0xFF;
    }
    s.metti("/oggetti/eseguibile?firma=abc", Risposta::ok(servito));
    s.metti(
        &format!("/download/{etichetta}/SHA256SUMS.txt"),
        Risposta::ok(format!(
            "{}  OBDio-{v}-windows-x64.zip\r\n{} *{nome}\r\n",
            somme::sha256(b"zip"),
            somme::sha256(&bin)
        )),
    );
    (etichetta, bin)
}

/// Cartella temporanea con un finto `OBDio.exe`.
fn installazione(nome: &str) -> (PathBuf, PathBuf) {
    let d = std::env::temp_dir().join(format!("obdio_agg_int_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let exe = d.join("OBDio.exe");
    std::fs::write(&exe, b"versione in uso").unwrap();
    (d, exe)
}

fn installa(
    s: &Server,
    exe: &Path,
    annulla: &AtomicBool,
    progresso: &mut dyn FnMut(u64, u64),
) -> Result<obdio::aggiornamenti::Installato, ErroreAggiornamento> {
    consenti_installazione();
    let c = s.cliente();
    let r = c.ultimo_rilascio().unwrap();
    scarica_e_installa(
        &c,
        &r,
        &Versione::corrente(),
        exe,
        PIATTAFORMA,
        annulla,
        progresso,
    )
}

fn nessun_residuo(exe: &Path) {
    assert!(
        !sostituzione::percorso_nuovo(exe).exists(),
        "nessun file parziale deve restare"
    );
}

// ---------------------------------------------------------------------------
// Prove
// ---------------------------------------------------------------------------

#[test]
fn versione_nuova_scaricata_verificata_e_sostituita() {
    let s = Server::avvia();
    let (etichetta, bin) = pubblica(&s, false);
    let (d, exe) = installazione("completa");

    let c = s.cliente();
    let r = c.ultimo_rilascio().unwrap();
    assert_eq!(r.etichetta, etichetta);
    assert_eq!(
        valuta(&Versione::corrente(), &r, &ConfigAggiornamenti::default()),
        Valutazione::Disponibile
    );
    // Solo le intestazioni previste: nessun cookie né credenziale.
    {
        let richieste = s.richieste.lock().unwrap();
        let (_, h) = &richieste[0];
        let ua = h.iter().find(|(n, _)| n == "user-agent").unwrap();
        assert_eq!(ua.1, format!("OBDio/{}", env!("CARGO_PKG_VERSION")));
        assert!(
            h.iter()
                .any(|(n, v)| n == "accept" && v == "application/vnd.github+json")
        );
        assert!(!h.iter().any(|(n, _)| n == "cookie" || n == "authorization"));
    }

    let annulla = AtomicBool::new(false);
    let mut ultimo = (0, 0);
    let fatto = installa(&s, &exe, &annulla, &mut |a, b| ultimo = (a, b)).unwrap();
    assert_eq!(ultimo, (bin.len() as u64, bin.len() as u64));
    assert_eq!(fatto.versione, versione_nuova());
    assert_eq!(std::fs::read(&exe).unwrap(), bin);
    assert_eq!(std::fs::read(&fatto.vecchio).unwrap(), b"versione in uso");
    assert_eq!(fatto.vecchio, d.join("OBDio.vecchio.exe"));
    nessun_residuo(&exe);
    // Il rinvio è stato seguito.
    assert_eq!(s.richieste_a("/oggetti/eseguibile"), 1);

    // Al prossimo avvio la copia vecchia sparisce.
    assert!(sostituzione::pulisci_residui(
        &sostituzione::FileReali,
        &exe
    ));
    assert!(!fatto.vecchio.exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn binario_alterato_rifiutato() {
    let s = Server::avvia();
    pubblica(&s, true);
    let (d, exe) = installazione("alterato");
    let esito = installa(&s, &exe, &AtomicBool::new(false), &mut |_, _| {});
    assert!(
        matches!(esito, Err(ErroreAggiornamento::SommaDiversa { .. })),
        "{esito:?}"
    );
    assert_eq!(std::fs::read(&exe).unwrap(), b"versione in uso");
    assert!(!sostituzione::percorso_vecchio(&exe).exists());
    nessun_residuo(&exe);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn dimensione_diversa_da_quella_dichiarata() {
    let s = Server::avvia();
    let (etichetta, bin) = pubblica(&s, false);
    // L'API dichiara un byte in più di quanti ne arrivano.
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::ok(json_rilascio(&s, &etichetta, bin.len() + 1, true)),
    );
    let (d, exe) = installazione("dimensione");
    let esito = installa(&s, &exe, &AtomicBool::new(false), &mut |_, _| {});
    assert!(
        matches!(esito, Err(ErroreAggiornamento::DimensioneDiversa { .. })),
        "{esito:?}"
    );
    assert_eq!(std::fs::read(&exe).unwrap(), b"versione in uso");
    nessun_residuo(&exe);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn somme_mancanti_o_senza_il_file() {
    let s = Server::avvia();
    let (etichetta, bin) = pubblica(&s, false);
    let (d, exe) = installazione("somme");
    // Rilascio senza SHA256SUMS.txt: nulla viene scaricato.
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::ok(json_rilascio(&s, &etichetta, bin.len(), false)),
    );
    let esito = installa(&s, &exe, &AtomicBool::new(false), &mut |_, _| {});
    assert!(matches!(esito, Err(ErroreAggiornamento::SommaMancante(_))));
    assert_eq!(s.richieste_a("/oggetti/"), 0);
    // Somme presenti ma senza la riga dell'eseguibile.
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::ok(json_rilascio(&s, &etichetta, bin.len(), true)),
    );
    s.metti(
        &format!("/download/{etichetta}/SHA256SUMS.txt"),
        Risposta::ok(format!("{}  altro.exe\n", somme::sha256(&bin))),
    );
    let esito = installa(&s, &exe, &AtomicBool::new(false), &mut |_, _| {});
    assert!(matches!(esito, Err(ErroreAggiornamento::SommaMancante(_))));
    assert_eq!(s.richieste_a("/oggetti/"), 0);
    assert_eq!(std::fs::read(&exe).unwrap(), b"versione in uso");
    nessun_residuo(&exe);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn annullato_a_meta_non_lascia_file_parziali() {
    let s = Server::avvia();
    pubblica(&s, false);
    // Scaricamento lento: un pezzo da 16 kB ogni 30 ms.
    let mut lento = Risposta::ok(binario_nuovo());
    lento.pausa = Some(Duration::from_millis(30));
    s.metti("/oggetti/eseguibile?firma=abc", lento);
    let (d, exe) = installazione("annullato");
    let annulla = AtomicBool::new(false);
    let mut a_meta = false;
    let esito = installa(&s, &exe, &annulla, &mut |fatti, totale| {
        if fatti > 0 && fatti < totale {
            a_meta = true;
            annulla.store(true, Ordering::Relaxed);
        }
    });
    assert!(a_meta, "lo scaricamento doveva essere a metà");
    assert_eq!(esito, Err(ErroreAggiornamento::Annullato));
    assert_eq!(std::fs::read(&exe).unwrap(), b"versione in uso");
    nessun_residuo(&exe);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn limite_di_richieste_comunicato() {
    let s = Server::avvia();
    let reset = rete::adesso() + 1200;
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::stato(403)
            .con("x-ratelimit-remaining", "0")
            .con("x-ratelimit-reset", &reset.to_string())
            .ok_corpo("{\"message\":\"API rate limit exceeded\"}"),
    );
    match s.cliente().ultimo_rilascio() {
        Err(ErroreAggiornamento::LimiteRichieste { riprova_alle }) => {
            assert_eq!(riprova_alle, Some(reset))
        }
        altro => panic!("{altro:?}"),
    }
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::stato(429).con("Retry-After", "60"),
    );
    match s.cliente().ultimo_rilascio() {
        Err(e @ ErroreAggiornamento::LimiteRichieste { riprova_alle }) => {
            let t = riprova_alle.unwrap();
            assert!((t - rete::adesso() - 60).abs() <= 5);
            assert!(e.messaggio().contains("Riprova dopo le"));
        }
        altro => panic!("{altro:?}"),
    }
    // Un 403 senza segni di limite è un rifiuto qualunque.
    s.metti("/repos/inalto/OBDio/releases/latest", Risposta::stato(403));
    assert_eq!(
        s.cliente().ultimo_rilascio(),
        Err(ErroreAggiornamento::Rifiutato(403))
    );
}

trait CorpoDiProva {
    fn ok_corpo(self, corpo: &str) -> Self;
}

impl CorpoDiProva for Risposta {
    fn ok_corpo(mut self, corpo: &str) -> Self {
        self.corpo = corpo.as_bytes().to_vec();
        self
    }
}

#[test]
fn nessuna_versione_risposte_rotte_e_tempo_scaduto() {
    let s = Server::avvia();
    // Repository senza rilasci (o non ancora creato): 404.
    assert_eq!(
        s.cliente().ultimo_rilascio(),
        Err(ErroreAggiornamento::NessunaVersione)
    );
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::ok("<html>pagina del portale</html>"),
    );
    assert!(matches!(
        s.cliente().ultimo_rilascio(),
        Err(ErroreAggiornamento::RispostaNonValida(_))
    ));
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::ok(vec![b'x'; 3 * 1024 * 1024]),
    );
    assert!(matches!(
        s.cliente().ultimo_rilascio(),
        Err(ErroreAggiornamento::TroppoGrande(_))
    ));
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::ok(vec![0xFF, 0xFE, 0x00, 0x7B]),
    );
    assert!(matches!(
        s.cliente().ultimo_rilascio(),
        Err(ErroreAggiornamento::RispostaNonValida(_))
    ));
    s.metti("/repos/inalto/OBDio/releases/latest", Risposta::stato(500));
    assert_eq!(
        s.cliente().ultimo_rilascio(),
        Err(ErroreAggiornamento::Rifiutato(500))
    );
    // Il server accetta la connessione e tace.
    let mut muto = Risposta::ok("");
    muto.muto = true;
    s.metti("/repos/inalto/OBDio/releases/latest", muto);
    let mut imp = s.impostazioni();
    imp.tempo_lettura = Duration::from_millis(400);
    imp.tempo_api = Duration::from_millis(800);
    let inizio = Instant::now();
    assert_eq!(
        Cliente::new(imp).ultimo_rilascio(),
        Err(ErroreAggiornamento::TempoScaduto)
    );
    assert!(inizio.elapsed() < Duration::from_secs(4));
}

#[test]
fn rinvii_verso_host_non_ammessi_rifiutati() {
    let s = Server::avvia();
    let (etichetta, _) = pubblica(&s, false);
    let nome = format!("OBDio-{}-windows-x64.exe", versione_nuova());
    s.metti(
        &format!("/download/{etichetta}/{nome}"),
        Risposta::rinvio("http://evil.example/OBDio.exe"),
    );
    let (d, exe) = installazione("rinvio");
    let esito = installa(&s, &exe, &AtomicBool::new(false), &mut |_, _| {});
    assert!(
        matches!(esito, Err(ErroreAggiornamento::IndirizzoNonAmmesso(_))),
        "{esito:?}"
    );
    // Un giro di rinvii senza fine.
    s.metti("/giro", Risposta::rinvio("/giro"));
    s.metti(
        &format!("/download/{etichetta}/{nome}"),
        Risposta::rinvio("/giro"),
    );
    let esito = installa(&s, &exe, &AtomicBool::new(false), &mut |_, _| {});
    assert_eq!(esito, Err(ErroreAggiornamento::TroppiRinvii));
    assert_eq!(std::fs::read(&exe).unwrap(), b"versione in uso");
    nessun_residuo(&exe);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn versione_non_piu_nuova_o_piattaforma_diversa_rifiutate() {
    consenti_installazione();
    let s = Server::avvia();
    let attuale = format!("v{}", env!("CARGO_PKG_VERSION"));
    s.metti(
        "/repos/inalto/OBDio/releases/latest",
        Risposta::ok(json_rilascio(&s, &attuale, 10, true)),
    );
    let (d, exe) = installazione("vecchia");
    let c = s.cliente();
    let r = c.ultimo_rilascio().unwrap();
    let esito = scarica_e_installa(
        &c,
        &r,
        &Versione::corrente(),
        &exe,
        PIATTAFORMA,
        &AtomicBool::new(false),
        &mut |_, _| {},
    );
    assert!(matches!(
        esito,
        Err(ErroreAggiornamento::NonPiuNuova { .. })
    ));

    // Versione nuova, ma per un'altra piattaforma non c'è il file.
    pubblica(&s, false);
    let r = c.ultimo_rilascio().unwrap();
    let esito = scarica_e_installa(
        &c,
        &r,
        &Versione::corrente(),
        &exe,
        "linux-x64",
        &AtomicBool::new(false),
        &mut |_, _| {},
    );
    assert!(matches!(
        esito,
        Err(ErroreAggiornamento::RisorsaMancante(_))
    ));

    // Cartella in cui non si può scrivere: nessuno scaricamento.
    let altrove = d.join("non_esiste").join("OBDio.exe");
    let esito = scarica_e_installa(
        &c,
        &r,
        &Versione::corrente(),
        &altrove,
        PIATTAFORMA,
        &AtomicBool::new(false),
        &mut |_, _| {},
    );
    assert!(matches!(
        esito,
        Err(ErroreAggiornamento::CartellaNonScrivibile(..))
    ));
    assert_eq!(s.richieste_a("/download/"), 0);
    assert_eq!(std::fs::read(&exe).unwrap(), b"versione in uso");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn servizio_in_un_thread_a_parte() {
    consenti_installazione();
    let s = Server::avvia();
    let (_, bin) = pubblica(&s, false);
    let (d, exe) = installazione("servizio");
    let notifiche = Arc::new(AtomicUsize::new(0));
    let n = notifiche.clone();
    let servizio = Servizio::avvia(s.impostazioni(), Some(exe.clone()), move || {
        n.fetch_add(1, Ordering::Relaxed);
    });
    let attendi = |servizio: &Servizio| -> Notizia {
        let fine = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(n) = servizio.ricevi() {
                return n;
            }
            assert!(Instant::now() < fine, "nessuna notizia dal servizio");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    servizio.controlla();
    let r = match attendi(&servizio) {
        Notizia::Controllo { esito, .. } => esito.unwrap(),
        altro => panic!("{altro:?}"),
    };
    servizio.installa(*r);
    let installato = loop {
        match attendi(&servizio) {
            Notizia::Progresso { scaricati, totale } => assert!(scaricati <= totale),
            Notizia::Installazione(esito) => break esito.unwrap(),
            altro => panic!("{altro:?}"),
        }
    };
    assert_eq!(std::fs::read(&installato.eseguibile).unwrap(), bin);
    assert!(notifiche.load(Ordering::Relaxed) >= 2);
    drop(servizio);
    let _ = std::fs::remove_dir_all(d);
}

/// Su Windows un programma in esecuzione non si sovrascrive ma si rinomina:
/// la sostituzione vera, con un eseguibile davvero in esecuzione (una copia
/// di `ping.exe` di sistema verso 127.0.0.1, che resta aperta qualche
/// secondo). Poi la pulizia, rimandata finché il vecchio è in uso.
#[cfg(windows)]
#[test]
fn sostituzione_di_un_eseguibile_in_esecuzione() {
    let sistema = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let ping = sistema.join("System32").join("PING.EXE");
    if !ping.exists() {
        eprintln!("ping.exe assente: prova saltata");
        return;
    }
    let (d, exe) = installazione("in_esecuzione");
    std::fs::copy(&ping, &exe).unwrap();
    let mut processo = std::process::Command::new(&exe)
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    // Sovrascriverlo direttamente non si può.
    assert!(std::fs::write(&exe, b"x").is_err());
    let nuovo = sostituzione::percorso_nuovo(&exe);
    std::fs::write(&nuovo, b"versione nuova").unwrap();
    let s = sostituzione::sostituisci(&sostituzione::FileReali, &exe, &nuovo).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"versione nuova");
    assert!(s.vecchio.exists());
    // Il vecchio è ancora in esecuzione: la pulizia aspetta senza errori.
    assert!(processo.try_wait().unwrap().is_none());
    assert!(!sostituzione::pulisci_residui(
        &sostituzione::FileReali,
        &exe
    ));
    assert!(s.vecchio.exists());
    let _ = processo.kill();
    let _ = processo.wait();
    let fine = Instant::now() + Duration::from_secs(10);
    while !sostituzione::pulisci_residui(&sostituzione::FileReali, &exe) {
        assert!(
            Instant::now() < fine,
            "il vecchio eseguibile non si elimina"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!s.vecchio.exists());
    assert_eq!(std::fs::read(&exe).unwrap(), b"versione nuova");
    let _ = std::fs::remove_dir_all(d);
}
