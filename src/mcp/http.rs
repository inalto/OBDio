//! Trasporto «Streamable HTTP» del server MCP, su `127.0.0.1`.
//!
//! Un piccolo server HTTP/1.1 fatto con `std::net`, senza altre librerie:
//! una richiesta per connessione (`Connection: close`), risposte JSON intere
//! (nessun flusso avviato dal server: `GET` risponde 405, come la specifica
//! consente).
//!
//! Difese, nell'ordine in cui scattano:
//! 1. ascolto solo su `127.0.0.1`: dalla rete non si arriva;
//! 2. al più [`MAX_CONNESSIONI`] connessioni servite insieme: le altre
//!    aspettano il loro turno nella coda del sistema operativo;
//! 3. testata al più di [`MAX_TESTATA`] byte e tempo massimo per richiesta;
//! 4. `Host` deve essere locale e `Origin`, se presente, pure (403): una
//!    pagina web aperta nel browser non può parlare con il server, nemmeno
//!    con il trucco del «DNS rebinding»;
//! 5. token `Authorization: Bearer …` confrontato in tempo costante (401);
//! 6. `Content-Type: application/json` obbligatorio (415): un modulo HTML
//!    non può inviare una richiesta valida;
//! 7. corpo al più di [`MAX_CORPO`] byte (413).

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::json;

use super::archivio::Archivio;
use super::protocollo::{self, Intestazioni};
use super::strumenti::Contesto;
use super::{Azione, McpConfig, PERCORSO, StatoMcp, StatoMcpInfo, token_accettabile};

/// Dimensione massima della testata (riga di richiesta e intestazioni).
pub const MAX_TESTATA: usize = 16 * 1024;
/// Dimensione massima del corpo di una richiesta.
pub const MAX_CORPO: usize = 1024 * 1024;
/// Connessioni servite insieme.
pub const MAX_CONNESSIONI: usize = 8;
/// Tempo massimo per ricevere una richiesta intera.
const TEMPO_RICHIESTA: Duration = Duration::from_secs(10);
/// Tempo massimo per scrivere la risposta.
const TEMPO_SCRITTURA: Duration = Duration::from_secs(10);
/// Tempo massimo di attesa del thread di ascolto alla chiusura.
const ATTESA_CHIUSURA: Duration = Duration::from_secs(2);
/// Passo del ciclo di ascolto: entro questo tempo si accorge della chiusura.
const PASSO_ASCOLTO: Duration = Duration::from_millis(15);

// ---------------------------------------------------------------------------
// Lettura della richiesta
// ---------------------------------------------------------------------------

/// Riga di richiesta e intestazioni.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Testata {
    pub metodo: String,
    /// Percorso senza la parte dopo `?`.
    pub percorso: String,
    /// Nomi in minuscolo, nell'ordine d'arrivo.
    intestazioni: Vec<(String, String)>,
}

impl Testata {
    /// Valore dell'intestazione (nome in minuscolo); la prima se ripetuta.
    pub fn intestazione(&self, nome: &str) -> Option<&str> {
        self.intestazioni
            .iter()
            .find(|(n, _)| n == nome)
            .map(|(_, v)| v.as_str())
    }

    fn ripetuta(&self, nome: &str) -> bool {
        self.intestazioni.iter().filter(|(n, _)| n == nome).count() > 1
    }
}

/// Richiesta non accettabile: codice HTTP e spiegazione.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rifiuto {
    pub stato: u16,
    pub messaggio: String,
}

fn rifiuto(stato: u16, messaggio: impl Into<String>) -> Rifiuto {
    Rifiuto {
        stato,
        messaggio: messaggio.into(),
    }
}

/// Errore di lettura: la connessione è caduta (non si risponde) oppure la
/// richiesta va rifiutata.
#[derive(Debug)]
pub enum Errore {
    Caduta,
    Rifiuto(Rifiuto),
}

impl From<Rifiuto> for Errore {
    fn from(r: Rifiuto) -> Self {
        Errore::Rifiuto(r)
    }
}

fn errore_io(e: &io::Error) -> Errore {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
            Errore::Rifiuto(rifiuto(408, "request not received in time"))
        }
        _ => Errore::Caduta,
    }
}

/// Lettore con memoria: ciò che arriva oltre la testata resta per il corpo.
pub struct Lettore<R> {
    r: R,
    buf: Vec<u8>,
}

impl<R: Read> Lettore<R> {
    pub fn new(r: R) -> Self {
        Lettore {
            r,
            buf: Vec::with_capacity(2048),
        }
    }

    /// Legge altri byte; `Ok(0)` se l'altro capo ha chiuso.
    fn riempi(&mut self) -> Result<usize, Errore> {
        let mut pezzo = [0u8; 4096];
        loop {
            match self.r.read(&mut pezzo) {
                Ok(n) => {
                    self.buf.extend_from_slice(&pezzo[..n]);
                    return Ok(n);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(errore_io(&e)),
            }
        }
    }

    /// Legge riga di richiesta e intestazioni, fino alla riga vuota.
    pub fn testata(&mut self) -> Result<Testata, Errore> {
        let fine = loop {
            if let Some(p) = trova(&self.buf, b"\r\n\r\n") {
                break p;
            }
            if self.buf.len() > MAX_TESTATA {
                return Err(rifiuto(431, "request header too large").into());
            }
            if self.riempi()? == 0 {
                return Err(Errore::Caduta);
            }
        };
        if fine > MAX_TESTATA {
            return Err(rifiuto(431, "request header too large").into());
        }
        let testo = std::str::from_utf8(&self.buf[..fine])
            .map_err(|_| rifiuto(400, "request header is not valid UTF-8"))?
            .to_string();
        self.buf.drain(..fine + 4);
        Ok(interpreta_testata(&testo)?)
    }

    /// Legge esattamente `n` byte del corpo.
    fn esatti(&mut self, n: usize) -> Result<Vec<u8>, Errore> {
        while self.buf.len() < n {
            if self.riempi()? == 0 {
                return Err(Errore::Caduta);
            }
        }
        Ok(self.buf.drain(..n).collect())
    }

    /// Legge una riga terminata da CR LF (senza il terminatore).
    fn riga(&mut self, massimo: usize) -> Result<String, Errore> {
        loop {
            if let Some(p) = trova(&self.buf, b"\r\n") {
                let riga: Vec<u8> = self.buf.drain(..p + 2).take(p).collect();
                return String::from_utf8(riga)
                    .map_err(|_| rifiuto(400, "malformed chunked body").into());
            }
            if self.buf.len() > massimo {
                return Err(rifiuto(400, "malformed chunked body").into());
            }
            if self.riempi()? == 0 {
                return Err(Errore::Caduta);
            }
        }
    }

    /// Legge il corpo secondo la testata: `Content-Length` oppure a pezzi
    /// (`Transfer-Encoding: chunked`), mai oltre `massimo` byte.
    pub fn corpo(&mut self, t: &Testata, massimo: usize) -> Result<Vec<u8>, Errore> {
        match lunghezza_corpo(t, massimo)? {
            Lunghezza::Esatta(n) => self.esatti(n),
            Lunghezza::APezzi => {
                let mut corpo = Vec::new();
                loop {
                    let riga = self.riga(256)?;
                    let cifre = riga.split(';').next().unwrap_or("").trim();
                    let n = usize::from_str_radix(cifre, 16)
                        .map_err(|_| rifiuto(400, "malformed chunked body"))?;
                    if n == 0 {
                        // Eventuali intestazioni di coda, fino alla riga vuota.
                        while !self.riga(MAX_TESTATA)?.is_empty() {}
                        return Ok(corpo);
                    }
                    if corpo.len().saturating_add(n) > massimo {
                        return Err(corpo_troppo_grande(massimo).into());
                    }
                    corpo.extend(self.esatti(n)?);
                    if !self.riga(2)?.is_empty() {
                        return Err(rifiuto(400, "malformed chunked body").into());
                    }
                }
            }
        }
    }
}

fn trova(dove: &[u8], che: &[u8]) -> Option<usize> {
    dove.windows(che.len()).position(|w| w == che)
}

fn corpo_troppo_grande(massimo: usize) -> Rifiuto {
    rifiuto(
        413,
        format!("request body too large (the limit is {massimo} bytes)"),
    )
}

fn carattere_di_nome(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// Interpreta riga di richiesta e intestazioni (senza la riga vuota finale).
pub fn interpreta_testata(testo: &str) -> Result<Testata, Rifiuto> {
    let mut righe = testo.split("\r\n");
    let prima = righe.next().unwrap_or("");
    let mut parti = prima.split(' ');
    let (Some(metodo), Some(bersaglio), Some(versione), None) =
        (parti.next(), parti.next(), parti.next(), parti.next())
    else {
        return Err(rifiuto(400, "malformed request line"));
    };
    if metodo.is_empty() || !metodo.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(rifiuto(400, "malformed request line"));
    }
    if !matches!(versione, "HTTP/1.1" | "HTTP/1.0") {
        return Err(rifiuto(505, "only HTTP/1.1 is supported"));
    }
    if !bersaglio.starts_with('/') || bersaglio.bytes().any(|b| b <= b' ' || b == 0x7F) {
        return Err(rifiuto(400, "malformed request target"));
    }
    let percorso = bersaglio.split(['?', '#']).next().unwrap_or("").to_string();
    let mut intestazioni = Vec::new();
    for riga in righe {
        // Le righe di continuazione sono vietate (e pericolose).
        if riga.starts_with([' ', '\t']) {
            return Err(rifiuto(400, "folded header lines are not accepted"));
        }
        let Some((nome, valore)) = riga.split_once(':') else {
            return Err(rifiuto(400, "malformed header line"));
        };
        if nome.is_empty() || !nome.bytes().all(carattere_di_nome) {
            return Err(rifiuto(400, "malformed header name"));
        }
        let valore = valore.trim_matches([' ', '\t']);
        if valore
            .bytes()
            .any(|b| (b < 0x20 && b != b'\t') || b == 0x7F)
        {
            return Err(rifiuto(400, "malformed header value"));
        }
        intestazioni.push((nome.to_ascii_lowercase(), valore.to_string()));
    }
    let t = Testata {
        metodo: metodo.to_string(),
        percorso,
        intestazioni,
    };
    // Intestazioni che decidono chi sei o quanto è lungo il corpo: una sola.
    for nome in [
        "host",
        "origin",
        "authorization",
        "content-length",
        "content-type",
        "transfer-encoding",
    ] {
        if t.ripetuta(nome) {
            return Err(rifiuto(400, format!("repeated `{nome}` header")));
        }
    }
    Ok(t)
}

enum Lunghezza {
    Esatta(usize),
    APezzi,
}

fn lunghezza_corpo(t: &Testata, massimo: usize) -> Result<Lunghezza, Rifiuto> {
    let dichiarata = t.intestazione("content-length");
    match t.intestazione("transfer-encoding") {
        Some(_) if dichiarata.is_some() => Err(rifiuto(
            400,
            "both Content-Length and Transfer-Encoding are present",
        )),
        Some(te) if te.eq_ignore_ascii_case("chunked") => Ok(Lunghezza::APezzi),
        Some(_) => Err(rifiuto(501, "unsupported Transfer-Encoding")),
        None => {
            let Some(cl) = dichiarata else {
                return Err(rifiuto(411, "Content-Length is required"));
            };
            if cl.is_empty() || !cl.bytes().all(|b| b.is_ascii_digit()) {
                return Err(rifiuto(400, "malformed Content-Length"));
            }
            match cl.parse::<usize>() {
                Ok(n) if n <= massimo => Ok(Lunghezza::Esatta(n)),
                _ => Err(corpo_troppo_grande(massimo)),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Controlli prima di leggere il corpo
// ---------------------------------------------------------------------------

/// Vero se `host` (con o senza porta) indica questo stesso computer.
fn host_locale(host: &str) -> bool {
    let senza_porta = if let Some(resto) = host.strip_prefix('[') {
        // IPv6 fra parentesi quadre, con porta facoltativa.
        match resto.split_once(']') {
            Some((indirizzo, coda)) if coda.is_empty() || porta_valida(coda) => {
                return indirizzo == "::1";
            }
            _ => return false,
        }
    } else {
        match host.rsplit_once(':') {
            Some((nome, porta)) if porta_valida(&format!(":{porta}")) => nome,
            Some(_) => return false,
            None => host,
        }
    };
    senza_porta == "127.0.0.1" || senza_porta.eq_ignore_ascii_case("localhost")
}

fn porta_valida(coda: &str) -> bool {
    coda.strip_prefix(':')
        .is_some_and(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Vero se l'origine dichiarata da un browser è una pagina di questo stesso
/// computer (`http://localhost:…`, `http://127.0.0.1:…`, `http://[::1]:…`).
/// `null` e ogni altro sito sono rifiutati.
pub fn origine_ammessa(origine: &str) -> bool {
    let Some((schema, resto)) = origine.split_once("://") else {
        return false;
    };
    if !(schema.eq_ignore_ascii_case("http") || schema.eq_ignore_ascii_case("https")) {
        return false;
    }
    // Un'origine non ha percorso, credenziali, né altro dopo l'host.
    if resto.is_empty() || resto.contains(['/', '@', '?', '#', '\\', ' ']) {
        return false;
    }
    host_locale(resto)
}

/// Confronto in tempo costante: la durata non dipende da dove i due valori
/// differiscono.
pub fn uguali_tempo_costante(a: &[u8], b: &[u8]) -> bool {
    let mut diversi = a.len() ^ b.len();
    // Si scorre sempre tutto il valore ricevuto.
    for (i, x) in a.iter().enumerate() {
        let y = b.get(i).copied().unwrap_or(0);
        diversi |= (x ^ y) as usize;
    }
    std::hint::black_box(diversi) == 0
}

/// Vero se l'intestazione `Authorization` porta il token giusto.
pub fn token_valido(autorizzazione: Option<&str>, token: &str) -> bool {
    let Some(a) = autorizzazione else {
        return false;
    };
    let Some((schema, valore)) = a.split_once(' ') else {
        return false;
    };
    // Un token vuoto non apre mai, nemmeno se la configurazione fosse vuota.
    !token.is_empty()
        && schema.eq_ignore_ascii_case("bearer")
        && uguali_tempo_costante(valore.trim().as_bytes(), token.as_bytes())
}

/// Che cosa fare di una richiesta dopo averne letta la testata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decisione {
    /// Richiesta POST autenticata: leggere il corpo ed elaborarlo.
    Elabora,
    /// Richiesta di verifica di un browser (`OPTIONS`): 204.
    Preliminare,
    /// Da respingere. `sicurezza` è vero per i rifiuti di `Host`, `Origin` e
    /// token, che vengono contati.
    Respinta { rifiuto: Rifiuto, sicurezza: bool },
}

fn respinta(stato: u16, messaggio: &str, sicurezza: bool) -> Decisione {
    Decisione::Respinta {
        rifiuto: rifiuto(stato, messaggio),
        sicurezza,
    }
}

/// Applica i controlli che non richiedono il corpo.
pub fn decidi(t: &Testata, token: &str) -> Decisione {
    match t.intestazione("host") {
        Some(h) if host_locale(h) => {}
        Some(_) => {
            return respinta(
                403,
                "forbidden: the Host header does not name this computer",
                true,
            );
        }
        None => return respinta(400, "the Host header is required", false),
    }
    if let Some(o) = t.intestazione("origin")
        && !origine_ammessa(o)
    {
        return respinta(
            403,
            "forbidden: requests from web pages of other sites are not accepted",
            true,
        );
    }
    if t.percorso != PERCORSO && t.percorso.strip_suffix('/') != Some(PERCORSO) {
        return respinta(404, "not found: the MCP endpoint is /mcp", false);
    }
    match t.metodo.as_str() {
        "POST" => {}
        "OPTIONS" => return Decisione::Preliminare,
        // Nessun flusso avviato dal server, nessuna sessione da chiudere.
        _ => {
            return respinta(
                405,
                "method not allowed: this endpoint accepts POST only",
                false,
            );
        }
    }
    if !token_valido(t.intestazione("authorization"), token) {
        return respinta(
            401,
            "unauthorized: send the token shown in OBDio as `Authorization: Bearer <token>`",
            true,
        );
    }
    let tipo = t
        .intestazione("content-type")
        .and_then(|c| c.split(';').next())
        .map(str::trim)
        .unwrap_or("");
    if !tipo.eq_ignore_ascii_case("application/json") {
        return respinta(
            415,
            "unsupported media type: Content-Type must be application/json",
            false,
        );
    }
    if let Some(accetta) = t.intestazione("accept") {
        let a = accetta.to_ascii_lowercase();
        if !(a.contains("application/json") || a.contains("application/*") || a.contains("*/*")) {
            return respinta(
                406,
                "not acceptable: the response is application/json",
                false,
            );
        }
    }
    match lunghezza_corpo(t, MAX_CORPO) {
        Ok(_) => Decisione::Elabora,
        Err(rifiuto) => Decisione::Respinta {
            rifiuto,
            sicurezza: false,
        },
    }
}

// ---------------------------------------------------------------------------
// Scrittura della risposta
// ---------------------------------------------------------------------------

fn frase(stato: u16) -> &'static str {
    match stato {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "Error",
    }
}

/// Compone la risposta HTTP. `origine` è l'origine locale del browser, se la
/// richiesta ne dichiarava una ammessa.
pub fn componi(stato: u16, corpo: Option<&[u8]>, origine: Option<&str>) -> Vec<u8> {
    let mut testa = format!(
        "HTTP/1.1 {stato} {}\r\nConnection: close\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n",
        frase(stato)
    );
    if let Some(c) = corpo {
        testa.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            c.len()
        ));
    } else if stato != 204 {
        testa.push_str("Content-Length: 0\r\n");
    }
    match stato {
        401 => testa.push_str("WWW-Authenticate: Bearer realm=\"OBDio MCP\"\r\n"),
        405 => testa.push_str("Allow: POST, OPTIONS\r\n"),
        _ => {}
    }
    if let Some(o) = origine {
        // Solo per pagine di questo computer (per esempio un ispettore MCP).
        testa.push_str(&format!(
            "Access-Control-Allow-Origin: {o}\r\nVary: Origin\r\n\
             Access-Control-Allow-Methods: POST, OPTIONS\r\n\
             Access-Control-Allow-Headers: authorization, content-type, accept, \
             mcp-protocol-version, mcp-method, mcp-name, mcp-session-id\r\n\
             Access-Control-Max-Age: 600\r\n"
        ));
    }
    testa.push_str("\r\n");
    let mut v = testa.into_bytes();
    if let Some(c) = corpo {
        v.extend_from_slice(c);
    }
    v
}

fn corpo_rifiuto(r: &Rifiuto) -> Vec<u8> {
    json!({
        "error": frase(r.stato),
        "message": r.messaggio,
        "about": protocollo::descrizione_breve(),
    })
    .to_string()
    .into_bytes()
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

fn blocca<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Stato condiviso fra il thread di ascolto, le connessioni e chi chiede lo
/// stato.
struct Condiviso {
    ctx: Contesto,
    token: String,
    porta: u16,
    stop: Arc<AtomicBool>,
    connessioni: AtomicUsize,
    richieste: AtomicU64,
    rifiutate: AtomicU64,
    ultimo_client: Mutex<Option<String>>,
    ultimo_errore: Mutex<Option<String>>,
}

/// Il server in ascolto. Lasciarlo cadere lo ferma.
pub struct Server {
    condiviso: Arc<Condiviso>,
    ascolto: Mutex<Option<(JoinHandle<()>, Receiver<()>)>>,
}

/// Messaggio per l'utente quando la porta non si apre.
fn errore_porta(porta: u16, e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::AddrInUse => format!(
            "la porta {porta} è già usata da un altro programma. Chiudi quel programma oppure \
             scegli un'altra porta: OBDio non ne sceglie una diversa da solo."
        ),
        io::ErrorKind::PermissionDenied => format!(
            "il sistema non permette di usare la porta {porta} (può essere riservata). Scegli \
             un'altra porta."
        ),
        _ => format!("impossibile aprire la porta {porta} su 127.0.0.1: {e}"),
    }
}

impl Server {
    /// Apre `127.0.0.1:porta` e comincia a servire in un thread proprio. Con
    /// la porta 0 il sistema ne sceglie una libera (vedi [`Server::porta`]).
    pub fn avvia(
        config: &McpConfig,
        archivio: Arc<Archivio>,
        azioni: Sender<Azione>,
        cartella_sessioni: PathBuf,
    ) -> Result<Server, String> {
        if !token_accettabile(&config.token) {
            return Err(
                "manca il token di accesso (o non è valido): disattiva e riattiva il server \
                 dalle Impostazioni, oppure premi «Rigenera»."
                    .into(),
            );
        }
        // Mai 0.0.0.0: solo l'interfaccia locale.
        let ascolto = TcpListener::bind((Ipv4Addr::LOCALHOST, config.porta))
            .map_err(|e| errore_porta(config.porta, &e))?;
        let porta = ascolto
            .local_addr()
            .map_err(|e| errore_porta(config.porta, &e))?
            .port();
        ascolto
            .set_nonblocking(true)
            .map_err(|e| errore_porta(porta, &e))?;
        let stop = Arc::new(AtomicBool::new(false));
        let condiviso = Arc::new(Condiviso {
            ctx: Contesto {
                archivio,
                azioni,
                livello: config.livello,
                cartella_sessioni,
                stop: stop.clone(),
                azione_in_corso: Mutex::new(()),
            },
            token: config.token.clone(),
            porta,
            stop,
            connessioni: AtomicUsize::new(0),
            richieste: AtomicU64::new(0),
            rifiutate: AtomicU64::new(0),
            ultimo_client: Mutex::new(None),
            ultimo_errore: Mutex::new(None),
        });
        let c = condiviso.clone();
        let (fine_tx, fine_rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("obdio-mcp".into())
            .spawn(move || {
                ascolta(ascolto, &c);
                let _ = fine_tx.send(());
            })
            .map_err(|e| format!("impossibile avviare il thread del server MCP: {e}"))?;
        Ok(Server {
            condiviso,
            ascolto: Mutex::new(Some((handle, fine_rx))),
        })
    }

    /// Porta effettiva su cui il server ascolta.
    pub fn porta(&self) -> u16 {
        self.condiviso.porta
    }

    /// Stato corrente e contatori.
    pub fn stato(&self) -> StatoMcpInfo {
        let c = &self.condiviso;
        StatoMcpInfo {
            stato: StatoMcp::InAscolto(c.porta),
            richieste: c.richieste.load(Ordering::Relaxed),
            rifiutate: c.rifiutate.load(Ordering::Relaxed),
            ultimo_client: blocca(&c.ultimo_client).clone(),
            ultimo_errore: blocca(&c.ultimo_errore).clone(),
        }
    }

    /// Smette di accettare connessioni e libera la porta. Attende il thread
    /// di ascolto per un tempo limitato; le connessioni ancora aperte
    /// finiscono da sole (le attese degli strumenti si interrompono subito).
    pub fn ferma(&self) {
        self.condiviso.stop.store(true, Ordering::SeqCst);
        if let Some((handle, fine)) = blocca(&self.ascolto).take() {
            match fine.recv_timeout(ATTESA_CHIUSURA) {
                Err(mpsc::RecvTimeoutError::Timeout) => drop(handle),
                _ => {
                    let _ = handle.join();
                }
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.ferma();
    }
}

/// Segnaposto di una connessione in corso: liberato comunque vada.
struct Posto<'a>(&'a AtomicUsize);

impl Drop for Posto<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn ascolta(ascolto: TcpListener, c: &Arc<Condiviso>) {
    while !c.stop.load(Ordering::Relaxed) {
        // Tutti i posti occupati: le nuove connessioni aspettano nella coda
        // del sistema (ogni connessione servita ha un tempo massimo).
        if c.connessioni.load(Ordering::SeqCst) >= MAX_CONNESSIONI {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        match ascolto.accept() {
            Ok((flusso, _)) => {
                // Su Windows il socket accettato eredita la modalità non bloccante.
                if flusso.set_nonblocking(false).is_err() {
                    continue;
                }
                let _ = flusso.set_nodelay(true);
                let _ = flusso.set_write_timeout(Some(TEMPO_SCRITTURA));
                c.connessioni.fetch_add(1, Ordering::SeqCst);
                let c2 = c.clone();
                let avviato = std::thread::Builder::new()
                    .name("obdio-mcp-conn".into())
                    .spawn(move || {
                        let _posto = Posto(&c2.connessioni);
                        servi(flusso, &c2);
                    });
                if avviato.is_err() {
                    c.connessioni.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(PASSO_ASCOLTO),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            // Errori passeggeri di una singola connessione: si continua.
            Err(_) => std::thread::sleep(PASSO_ASCOLTO),
        }
    }
    // `ascolto` esce di scope qui: la porta si libera.
}

/// Lettura con una scadenza complessiva: ogni `read` attende al più il tempo
/// che resta.
struct ConScadenza<'a> {
    flusso: &'a TcpStream,
    scadenza: Instant,
}

impl Read for ConScadenza<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let resto = self.scadenza.saturating_duration_since(Instant::now());
        if resto.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.flusso.set_read_timeout(Some(resto))?;
        let mut f = self.flusso;
        f.read(buf)
    }
}

/// Scrive la risposta e chiude con garbo: prima chiude la scrittura, poi
/// scarta per un attimo ciò che il client sta ancora inviando, così la
/// risposta non va persa quando la richiesta viene respinta a metà.
fn rispondi(mut flusso: &TcpStream, byte: &[u8]) {
    if flusso.write_all(byte).is_err() {
        return;
    }
    let _ = flusso.flush();
    let _ = flusso.shutdown(Shutdown::Write);
    let _ = flusso.set_read_timeout(Some(Duration::from_millis(150)));
    let fine = Instant::now() + Duration::from_millis(400);
    let mut scarto = [0u8; 8192];
    let mut totale = 0usize;
    while Instant::now() < fine && totale < 256 * 1024 {
        match flusso.read(&mut scarto) {
            Ok(0) | Err(_) => break,
            Ok(n) => totale += n,
        }
    }
}

fn ora_breve() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// Serve una connessione: una richiesta, una risposta.
fn servi(flusso: TcpStream, c: &Condiviso) {
    let mut lettore = Lettore::new(ConScadenza {
        flusso: &flusso,
        scadenza: Instant::now() + TEMPO_RICHIESTA,
    });
    let testata = match lettore.testata() {
        Ok(t) => t,
        Err(Errore::Caduta) => return,
        Err(Errore::Rifiuto(r)) => {
            rispondi(&flusso, &componi(r.stato, Some(&corpo_rifiuto(&r)), None));
            return;
        }
    };
    // L'origine torna nelle intestazioni solo se è una pagina locale.
    let origine = testata
        .intestazione("origin")
        .filter(|o| origine_ammessa(o))
        .map(str::to_string);
    let origine = origine.as_deref();
    match decidi(&testata, &c.token) {
        Decisione::Elabora => {}
        Decisione::Preliminare => {
            rispondi(&flusso, &componi(204, None, origine));
            return;
        }
        Decisione::Respinta { rifiuto, sicurezza } => {
            if sicurezza {
                c.rifiutate.fetch_add(1, Ordering::Relaxed);
                *blocca(&c.ultimo_errore) = Some(format!(
                    "richiesta respinta alle {} ({}): {}",
                    ora_breve(),
                    rifiuto.stato,
                    match rifiuto.stato {
                        401 => "token mancante o errato",
                        _ => "non proveniva da questo computer (Host o Origin)",
                    }
                ));
            }
            rispondi(
                &flusso,
                &componi(rifiuto.stato, Some(&corpo_rifiuto(&rifiuto)), origine),
            );
            return;
        }
    }
    if c.stop.load(Ordering::Relaxed) {
        let r = rifiuto(503, "the OBDio MCP server is shutting down");
        rispondi(&flusso, &componi(503, Some(&corpo_rifiuto(&r)), origine));
        return;
    }
    if testata
        .intestazione("expect")
        .is_some_and(|e| e.eq_ignore_ascii_case("100-continue"))
    {
        let mut f = &flusso;
        if f.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").is_err() {
            return;
        }
    }
    let corpo = match lettore.corpo(&testata, MAX_CORPO) {
        Ok(b) => b,
        Err(Errore::Caduta) => return,
        Err(Errore::Rifiuto(r)) => {
            rispondi(
                &flusso,
                &componi(r.stato, Some(&corpo_rifiuto(&r)), origine),
            );
            return;
        }
    };
    c.richieste.fetch_add(1, Ordering::Relaxed);
    let h = Intestazioni {
        versione: testata.intestazione("mcp-protocol-version"),
        metodo: testata.intestazione("mcp-method"),
        nome: testata.intestazione("mcp-name"),
    };
    let risposta = protocollo::gestisci(&c.ctx, &h, &corpo);
    if let Some(cliente) = risposta.client {
        *blocca(&c.ultimo_client) = Some(cliente);
    }
    let byte = risposta.corpo.map(|v| v.to_string().into_bytes());
    rispondi(&flusso, &componi(risposta.stato, byte.as_deref(), origine));
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn testata(righe: &[&str]) -> Testata {
        interpreta_testata(&righe.join("\r\n")).unwrap()
    }

    fn post(extra: &[&str]) -> Testata {
        let autorizzazione = format!("Authorization: Bearer {TOKEN}");
        let mut righe = vec![
            "POST /mcp HTTP/1.1",
            "Host: 127.0.0.1:8765",
            "Content-Type: application/json",
            "Accept: application/json, text/event-stream",
            "Content-Length: 2",
        ];
        if !extra.iter().any(|r| r.starts_with("Authorization")) {
            righe.push(&autorizzazione);
        }
        righe.extend(extra);
        interpreta_testata(&righe.join("\r\n")).unwrap()
    }

    fn stato_di(d: &Decisione) -> (u16, bool) {
        match d {
            Decisione::Respinta { rifiuto, sicurezza } => (rifiuto.stato, *sicurezza),
            altro => panic!("attesa una richiesta respinta, avuto {altro:?}"),
        }
    }

    #[test]
    fn token_mancante_errato_giusto() {
        assert_eq!(decidi(&post(&[]), TOKEN), Decisione::Elabora);
        // Mancante.
        let senza = testata(&[
            "POST /mcp HTTP/1.1",
            "Host: 127.0.0.1:8765",
            "Content-Type: application/json",
            "Content-Length: 2",
        ]);
        assert_eq!(stato_di(&decidi(&senza, TOKEN)), (401, true));
        // Errato: un carattere diverso, più corto, più lungo, vuoto, altro schema.
        let mut quasi = TOKEN.to_string();
        quasi.replace_range(63.., "0");
        for a in [
            format!("Authorization: Bearer {quasi}"),
            format!("Authorization: Bearer {}", &TOKEN[..63]),
            format!("Authorization: Bearer {TOKEN}0"),
            "Authorization: Bearer ".to_string(),
            "Authorization: Bearer".to_string(),
            format!("Authorization: Basic {TOKEN}"),
            format!("Authorization: {TOKEN}"),
        ] {
            assert_eq!(stato_di(&decidi(&post(&[&a]), TOKEN)), (401, true), "{a}");
        }
        // Giusto, anche con lo schema in minuscolo.
        let minuscolo = format!("Authorization: bearer {TOKEN}");
        assert_eq!(decidi(&post(&[&minuscolo]), TOKEN), Decisione::Elabora);
        // Un token vuoto nella configurazione non apre a nessuno.
        assert!(!token_valido(Some("Bearer "), ""));
        assert!(!token_valido(Some("Bearer"), ""));
        assert!(!token_valido(None, TOKEN));

        assert!(uguali_tempo_costante(b"abc", b"abc"));
        assert!(!uguali_tempo_costante(b"abc", b"abd"));
        assert!(!uguali_tempo_costante(b"abc", b"ab"));
        assert!(!uguali_tempo_costante(b"ab", b"abc"));
        assert!(uguali_tempo_costante(b"", b""));
    }

    #[test]
    fn origine_e_host() {
        for o in [
            "http://localhost",
            "http://localhost:6274",
            "http://127.0.0.1:8765",
            "https://LOCALHOST:3000",
            "http://[::1]:5173",
        ] {
            assert!(origine_ammessa(o), "{o}");
            let riga = format!("Origin: {o}");
            assert_eq!(decidi(&post(&[&riga]), TOKEN), Decisione::Elabora, "{o}");
        }
        for o in [
            "null",
            "",
            "https://example.com",
            "http://evil.example",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example",
            "http://evil.example:8765",
            "http://127.0.0.1@evil.example",
            "http://localhost:8765/percorso",
            "http://192.168.1.10:8765",
            "http://127.0.0.2",
            "http://localhost:abc",
            "file://localhost",
            "localhost",
            "http://",
        ] {
            assert!(!origine_ammessa(o), "{o}");
            let riga = format!("Origin: {o}");
            // Respinta anche con il token giusto.
            assert_eq!(
                stato_di(&decidi(&post(&[&riga]), TOKEN)),
                (403, true),
                "{o}"
            );
        }
        // Senza Origin (client non browser) la richiesta passa.
        assert_eq!(decidi(&post(&[]), TOKEN), Decisione::Elabora);

        // Host: solo questo computer («DNS rebinding»).
        for (host, atteso) in [
            ("127.0.0.1", None),
            ("localhost:8765", None),
            ("LOCALHOST", None),
            ("[::1]:8765", None),
            ("evil.example", Some(403)),
            ("evil.example:8765", Some(403)),
            ("127.0.0.1.evil.example", Some(403)),
            ("192.168.1.10:8765", Some(403)),
            ("127.0.0.1:porta", Some(403)),
            ("[::2]:8765", Some(403)),
        ] {
            let riga = format!("Host: {host}");
            let autorizzazione = format!("Authorization: Bearer {TOKEN}");
            let t = testata(&[
                "POST /mcp HTTP/1.1",
                &riga,
                &autorizzazione,
                "Content-Type: application/json",
                "Content-Length: 2",
            ]);
            match atteso {
                None => assert_eq!(decidi(&t, TOKEN), Decisione::Elabora, "{host}"),
                Some(s) => assert_eq!(stato_di(&decidi(&t, TOKEN)), (s, true), "{host}"),
            }
        }
        let senza_host = testata(&["POST /mcp HTTP/1.1", "Content-Length: 2"]);
        assert_eq!(stato_di(&decidi(&senza_host, TOKEN)), (400, false));
    }

    #[test]
    fn percorso_metodo_tipo_e_dimensione() {
        let base = |prima: &str, extra: &[&str]| {
            let autorizzazione = format!("Authorization: Bearer {TOKEN}");
            let mut righe = vec![prima, "Host: localhost:8765"];
            righe.push(&autorizzazione);
            righe.extend(extra);
            interpreta_testata(&righe.join("\r\n")).unwrap()
        };
        let json = ["Content-Type: application/json", "Content-Length: 10"];
        assert_eq!(
            decidi(&base("POST /mcp HTTP/1.1", &json), TOKEN),
            Decisione::Elabora
        );
        assert_eq!(
            decidi(&base("POST /mcp/?x=1 HTTP/1.1", &json), TOKEN),
            Decisione::Elabora
        );
        for percorso in ["/", "/mcp2", "/altro/mcp", "/MCP", "/mcp/x"] {
            let prima = format!("POST {percorso} HTTP/1.1");
            assert_eq!(
                stato_di(&decidi(&base(&prima, &json), TOKEN)),
                (404, false),
                "{percorso}"
            );
        }
        // GET e DELETE: 405 (nessun flusso dal server, nessuna sessione).
        for metodo in ["GET", "DELETE", "PUT", "HEAD"] {
            let prima = format!("{metodo} /mcp HTTP/1.1");
            assert_eq!(
                stato_di(&decidi(&base(&prima, &[]), TOKEN)),
                (405, false),
                "{metodo}"
            );
        }
        assert_eq!(
            decidi(&base("OPTIONS /mcp HTTP/1.1", &[]), TOKEN),
            Decisione::Preliminare
        );
        // Tipo del contenuto: solo JSON (un modulo HTML non passa).
        for tipo in [
            "text/plain",
            "application/x-www-form-urlencoded",
            "multipart/form-data",
            "",
        ] {
            let riga = format!("Content-Type: {tipo}");
            let t = base("POST /mcp HTTP/1.1", &[&riga, "Content-Length: 2"]);
            assert_eq!(stato_di(&decidi(&t, TOKEN)), (415, false), "{tipo}");
        }
        let t = base("POST /mcp HTTP/1.1", &["Content-Length: 2"]);
        assert_eq!(stato_di(&decidi(&t, TOKEN)), (415, false));
        let t = base(
            "POST /mcp HTTP/1.1",
            &[
                "Content-Type: application/json; charset=utf-8",
                "Content-Length: 2",
            ],
        );
        assert_eq!(decidi(&t, TOKEN), Decisione::Elabora);
        let t = base(
            "POST /mcp HTTP/1.1",
            &[json[0], json[1], "Accept: text/event-stream"],
        );
        assert_eq!(stato_di(&decidi(&t, TOKEN)), (406, false));
        // Dimensione: respinta dalla sola testata, senza leggere il corpo.
        let grande = format!("Content-Length: {}", MAX_CORPO + 1);
        let t = base("POST /mcp HTTP/1.1", &[json[0], &grande]);
        assert_eq!(stato_di(&decidi(&t, TOKEN)), (413, false));
        let al_limite = format!("Content-Length: {MAX_CORPO}");
        let t = base("POST /mcp HTTP/1.1", &[json[0], &al_limite]);
        assert_eq!(decidi(&t, TOKEN), Decisione::Elabora);
        let t = base("POST /mcp HTTP/1.1", &[json[0]]);
        assert_eq!(stato_di(&decidi(&t, TOKEN)), (411, false));
        for cl in ["-1", "1e3", "0x10", "99999999999999999999999999", "2, 2"] {
            let riga = format!("Content-Length: {cl}");
            let t = base("POST /mcp HTTP/1.1", &[json[0], &riga]);
            let (stato, _) = stato_di(&decidi(&t, TOKEN));
            assert!(stato == 400 || stato == 413, "{cl}: {stato}");
        }
        let t = base(
            "POST /mcp HTTP/1.1",
            &[json[0], json[1], "Transfer-Encoding: chunked"],
        );
        assert_eq!(stato_di(&decidi(&t, TOKEN)), (400, false));
        let t = base("POST /mcp HTTP/1.1", &[json[0], "Transfer-Encoding: gzip"]);
        assert_eq!(stato_di(&decidi(&t, TOKEN)), (501, false));
    }

    #[test]
    fn testate_malformate() {
        for (testo, stato) in [
            ("", 400),
            ("POST /mcp", 400),
            ("POST  /mcp HTTP/1.1", 400),
            ("post /mcp HTTP/1.1", 400),
            ("POST /mcp HTTP/2", 505),
            ("POST mcp HTTP/1.1", 400),
            ("POST http://evil.example/mcp HTTP/1.1", 400),
            ("POST /mcp HTTP/1.1\r\nSenzaDuePunti", 400),
            ("POST /mcp HTTP/1.1\r\nNome Con Spazi: x", 400),
            ("POST /mcp HTTP/1.1\r\n: vuoto", 400),
            ("POST /mcp HTTP/1.1\r\nA: 1\r\n continuazione", 400),
            ("POST /mcp HTTP/1.1\r\nA: valore\u{0}nullo", 400),
            ("POST /mcp HTTP/1.1\r\nHost: a\r\nHost: b", 400),
            (
                "POST /mcp HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2",
                400,
            ),
            (
                "POST /mcp HTTP/1.1\r\nAuthorization: a\r\nauthorization: b",
                400,
            ),
            ("POST /mcp HTTP/1.1\r\nOrigin: a\r\nORIGIN: b", 400),
        ] {
            assert_eq!(
                interpreta_testata(testo).unwrap_err().stato,
                stato,
                "{testo:?}"
            );
        }
        let t = testata(&[
            "POST /mcp?a=b HTTP/1.1",
            "HOST:  localhost ",
            "X-Altro: 1",
            "x-altro: 2",
        ]);
        assert_eq!(t.metodo, "POST");
        assert_eq!(t.percorso, "/mcp");
        assert_eq!(t.intestazione("host"), Some("localhost"));
        assert_eq!(t.intestazione("x-altro"), Some("1"));
        assert_eq!(t.intestazione("assente"), None);
    }

    /// Consegna i byte a piccoli pezzi, come una rete lenta.
    struct APezzetti<'a>(&'a [u8], usize);

    impl Read for APezzetti<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.1.min(self.0.len()).min(buf.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    fn leggi(byte: &[u8], pezzo: usize) -> Result<(Testata, Vec<u8>), Errore> {
        let mut l = Lettore::new(APezzetti(byte, pezzo));
        let t = l.testata()?;
        let corpo = l.corpo(&t, 64)?;
        Ok((t, corpo))
    }

    fn rifiuto_di<T: std::fmt::Debug>(r: Result<T, Errore>) -> u16 {
        match r {
            Err(Errore::Rifiuto(r)) => r.stato,
            altro => panic!("atteso un rifiuto, avuto {altro:?}"),
        }
    }

    #[test]
    fn lettura_a_pezzi_e_limiti() {
        let richiesta = b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nciao!";
        for pezzo in [1, 3, 7, 4096] {
            let (t, corpo) = leggi(richiesta, pezzo).unwrap();
            assert_eq!(t.intestazione("content-length"), Some("5"));
            assert_eq!(corpo, b"ciao!");
        }
        // Corpo a pezzi (chunked), con estensione e intestazione di coda.
        let a_pezzi =
            b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n\
            3\r\ncia\r\n2;x=y\r\no!\r\n0\r\nCoda: 1\r\n\r\n";
        for pezzo in [1, 5, 4096] {
            assert_eq!(leggi(a_pezzi, pezzo).unwrap().1, b"ciao!");
        }
        // Oltre il limite (64 byte in questa prova): 413, dichiarato o a pezzi.
        let grande = b"POST /mcp HTTP/1.1\r\nContent-Length: 65\r\n\r\n";
        assert_eq!(rifiuto_di(leggi(grande, 4096)), 413);
        let mut pezzi_grandi = b"POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for _ in 0..5 {
            pezzi_grandi.extend_from_slice(b"10\r\n0123456789abcdef\r\n");
        }
        pezzi_grandi.extend_from_slice(b"0\r\n\r\n");
        assert_eq!(rifiuto_di(leggi(&pezzi_grandi, 4096)), 413);
        let enorme =
            b"POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nFFFFFFFFFFFFFFFF\r\n";
        assert_eq!(rifiuto_di(leggi(enorme, 4096)), 413);
        for rotto in [
            &b"POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"[..],
            &b"POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nciaoX\r\n0\r\n\r\n"[..],
        ] {
            assert_eq!(rifiuto_di(leggi(rotto, 4096)), 400);
        }
        // Testata senza fine: 431 invece di riempire la memoria.
        let mut infinita = b"POST /mcp HTTP/1.1\r\n".to_vec();
        while infinita.len() < MAX_TESTATA + 5000 {
            infinita.extend_from_slice(b"X-Riempitivo: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
        }
        assert_eq!(rifiuto_di(leggi(&infinita, 4096)), 431);
        // Connessione chiusa a metà: nessuna risposta da dare.
        assert!(matches!(
            leggi(b"POST /mcp HTTP/1.1\r\nHost: loc", 4096),
            Err(Errore::Caduta)
        ));
        assert!(matches!(
            leggi(
                b"POST /mcp HTTP/1.1\r\nContent-Length: 9\r\n\r\ncorto",
                4096
            ),
            Err(Errore::Caduta)
        ));
        assert!(matches!(leggi(b"", 4096), Err(Errore::Caduta)));
    }

    #[test]
    fn risposte_composte() {
        let r = String::from_utf8(componi(200, Some(b"{}"), None)).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(r.contains("Content-Type: application/json\r\n"));
        assert!(r.contains("Content-Length: 2\r\n"));
        assert!(r.contains("Connection: close\r\n"));
        assert!(r.ends_with("\r\n\r\n{}"));
        // Senza Origin locale nessuna intestazione CORS.
        assert!(!r.contains("Access-Control"));
        let accettata = String::from_utf8(componi(202, None, None)).unwrap();
        assert!(accettata.starts_with("HTTP/1.1 202 Accepted\r\n"));
        assert!(accettata.contains("Content-Length: 0\r\n"));
        assert!(accettata.ends_with("\r\n\r\n"));
        let non_autorizzata = String::from_utf8(componi(401, Some(b"{}"), None)).unwrap();
        assert!(non_autorizzata.contains("WWW-Authenticate: Bearer"));
        let metodo = String::from_utf8(componi(405, Some(b"{}"), None)).unwrap();
        assert!(metodo.contains("Allow: POST, OPTIONS\r\n"));
        let locale = String::from_utf8(componi(204, None, Some("http://localhost:6274"))).unwrap();
        assert!(locale.contains("Access-Control-Allow-Origin: http://localhost:6274\r\n"));
        assert!(!locale.contains("Content-Length"));
    }

    #[test]
    fn porta_occupata_e_token_mancante() {
        let (tx, _rx) = mpsc::channel();
        let config = McpConfig {
            attivo: true,
            porta: 0,
            token: TOKEN.into(),
            ..Default::default()
        };
        let primo = Server::avvia(
            &config,
            Arc::new(Archivio::default()),
            tx.clone(),
            std::env::temp_dir(),
        )
        .unwrap();
        let porta = primo.porta();
        assert_ne!(porta, 0);
        assert_eq!(primo.stato().stato, StatoMcp::InAscolto(porta));
        // La stessa porta una seconda volta: errore chiaro, nessun ripiego.
        let occupata = McpConfig {
            porta,
            ..config.clone()
        };
        let e = Server::avvia(
            &occupata,
            Arc::new(Archivio::default()),
            tx.clone(),
            std::env::temp_dir(),
        )
        .err()
        .expect("la porta è occupata");
        assert!(e.contains(&format!("la porta {porta} è già usata")), "{e}");
        // Fermato il primo, la porta torna libera in fretta.
        let inizio = Instant::now();
        primo.ferma();
        assert!(inizio.elapsed() < Duration::from_secs(2));
        let secondo = Server::avvia(
            &occupata,
            Arc::new(Archivio::default()),
            tx.clone(),
            std::env::temp_dir(),
        )
        .unwrap();
        assert_eq!(secondo.porta(), porta);
        drop(secondo);
        // Senza token il server non parte.
        let senza = McpConfig {
            token: String::new(),
            ..config
        };
        let e = Server::avvia(
            &senza,
            Arc::new(Archivio::default()),
            tx,
            std::env::temp_dir(),
        )
        .err()
        .expect("manca il token");
        assert!(e.contains("token"), "{e}");
    }
}
