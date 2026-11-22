//! JSON-RPC 2.0 e Model Context Protocol, lato server.
//!
//! Sono servite due «ere» del protocollo sullo stesso indirizzo, come
//! previsto dalla specifica per un server che vuole parlare con client nuovi
//! e vecchi (`basic/versioning`, «dual-era»):
//!
//! - **2026-07-28** (corrente): senza stato. Ogni richiesta porta in
//!   `params._meta` la versione del protocollo e le capacità del client, e
//!   le intestazioni HTTP `MCP-Protocol-Version`, `Mcp-Method` e `Mcp-Name`
//!   devono coincidere con il corpo. Metodi: `server/discover`, `ping`,
//!   `tools/list`, `tools/call`.
//! - **2025-11-25** e **2025-06-18**: con `initialize` e
//!   `notifications/initialized`. Non viene assegnata una sessione
//!   (`MCP-Session-Id` è facoltativo): il server non ha stato per client.
//!
//! La revisione 2025-03-26 non è servita: richiederebbe i lotti JSON-RPC,
//! che qui sono rifiutati (`-32600`).
//!
//! Questo modulo non sa nulla di socket: riceve le intestazioni che contano e
//! il corpo, restituisce codice HTTP e corpo JSON. Token, `Origin` e limiti
//! sono compito di [`super::http`].

use serde_json::{Map, Value, json};

use super::strumenti::{self, Chiamata, Contesto, NOTA_CANCELLAZIONE};
use super::{Livello, PERCORSO};

/// Revisione corrente, senza stato.
pub const VERSIONE_MODERNA: &str = "2026-07-28";
/// Revisioni con `initialize`, dalla più recente.
pub const VERSIONI_CON_INITIALIZE: [&str; 2] = ["2025-11-25", "2025-06-18"];

const META_VERSIONE: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";
const META_CAPACITA: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";

const ERR_JSON: i64 = -32700;
const ERR_RICHIESTA: i64 = -32600;
const ERR_METODO: i64 = -32601;
const ERR_PARAMETRI: i64 = -32602;
const ERR_INTESTAZIONI: i64 = -32020;
const ERR_VERSIONE: i64 = -32022;

/// Per quanto un client può tenere in memoria l'elenco degli strumenti: poco,
/// perché l'utente può cambiare livello in qualsiasi momento.
const DURATA_CACHE_MS: u64 = 10_000;

/// Intestazioni HTTP che il protocollo guarda.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Intestazioni<'a> {
    /// `MCP-Protocol-Version`.
    pub versione: Option<&'a str>,
    /// `Mcp-Method`.
    pub metodo: Option<&'a str>,
    /// `Mcp-Name`.
    pub nome: Option<&'a str>,
}

/// Risposta da scrivere sul socket.
#[derive(Clone, Debug, PartialEq)]
pub struct Risposta {
    pub stato: u16,
    /// `None`: nessun corpo (202 per notifiche e risposte del client).
    pub corpo: Option<Value>,
    /// Nome e versione del client, se la richiesta li dichiarava.
    pub client: Option<String>,
}

impl Risposta {
    fn errore(stato: u16, id: Option<&Value>, codice: i64, messaggio: impl Into<String>) -> Self {
        Self::errore_con(stato, id, codice, messaggio, None)
    }

    fn errore_con(
        stato: u16,
        id: Option<&Value>,
        codice: i64,
        messaggio: impl Into<String>,
        dati: Option<Value>,
    ) -> Self {
        let mut errore = json!({ "code": codice, "message": messaggio.into() });
        if let Some(d) = dati {
            errore["data"] = d;
        }
        let mut corpo = json!({ "jsonrpc": "2.0", "error": errore });
        // Senza un identificativo leggibile il campo `id` manca.
        if let Some(id) = id {
            corpo["id"] = id.clone();
        }
        Risposta {
            stato,
            corpo: Some(corpo),
            client: None,
        }
    }

    fn accettata() -> Self {
        Risposta {
            stato: 202,
            corpo: None,
            client: None,
        }
    }
}

/// Tutte le revisioni servite, dalla più recente.
pub fn versioni() -> Vec<&'static str> {
    std::iter::once(VERSIONE_MODERNA)
        .chain(VERSIONI_CON_INITIALIZE)
        .collect()
}

fn versione_con_initialize(v: &str) -> bool {
    VERSIONI_CON_INITIALIZE.contains(&v)
}

fn info_server() -> Value {
    json!({
        "name": "obdio",
        "title": "OBDio",
        "version": env!("CARGO_PKG_VERSION"),
    })
}

fn capacita() -> Value {
    // Solo ciò che è realizzato: strumenti, senza notifiche di cambiamento.
    json!({ "tools": {} })
}

/// Istruzioni per il modello, uguali in entrambe le ere.
pub fn istruzioni(livello: Livello) -> String {
    let azioni = match livello {
        Livello::SolaLettura => {
            "This server is in READ-ONLY mode: it exposes no action tools. Only the user can \
             change that, from the OBDio settings."
        }
        Livello::LetturaEAzioni => {
            "The user enabled actions: you can start/stop reading sensors, start/stop \
             recording, add markers and ask the ECU for a fresh diagnosis (read-only OBD \
             services)."
        }
    };
    format!(
        "OBDio is a desktop OBD-II car diagnostic tool (ELM327 adapters). This server exposes \
         what the running app currently knows: connection status, live sensor values, the \
         last diagnosis (trouble codes, readiness, freeze frame, Mode 06) and recorded \
         sessions. It only works while OBDio is open on the user's computer. Start with \
         get_status and ALWAYS check `origin`: `vehicle` is a real car, `demo_simulator` is \
         synthetic data, `replay` is a recorded session played back - never present simulated \
         or replayed data as current readings of a real car. A missing value is null with a \
         status, never 0. Sensor keys and names are Italian. The VIN is never exposed. \
         {azioni} {NOTA_CANCELLAZIONE} Connecting and disconnecting the adapter are also done \
         only by the user in the app. Readings describe what the ECU reports; they do not by \
         themselves establish which component is faulty."
    )
}

/// Decodifica un valore `Mcp-Name` scritto con la sentinella Base64
/// (`=?base64?…?=`); gli altri valori restano come sono.
pub fn decodifica_nome(valore: &str) -> Option<String> {
    let Some(dentro) = valore
        .strip_prefix("=?base64?")
        .and_then(|v| v.strip_suffix("?="))
    else {
        return Some(valore.to_string());
    };
    let mut byte = Vec::with_capacity(dentro.len() * 3 / 4);
    let (mut accumulo, mut bit) = (0u32, 0u32);
    for c in dentro.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => continue,
            _ => return None,
        };
        accumulo = (accumulo << 6) | v as u32;
        bit += 6;
        if bit >= 8 {
            bit -= 8;
            byte.push((accumulo >> bit) as u8);
            accumulo &= (1 << bit) - 1;
        }
    }
    String::from_utf8(byte).ok()
}

/// Nome e versione del client da un oggetto `Implementation`.
fn nome_client(v: Option<&Value>) -> Option<String> {
    let v = v?.as_object()?;
    let pulisci = |s: &str| -> String {
        s.chars()
            .filter(|c| !c.is_control())
            .take(60)
            .collect::<String>()
            .trim()
            .to_string()
    };
    let nome = pulisci(v.get("name")?.as_str()?);
    if nome.is_empty() {
        return None;
    }
    Some(match v.get("version").and_then(Value::as_str) {
        Some(ver) if !ver.trim().is_empty() => format!("{nome} {}", pulisci(ver)),
        _ => nome,
    })
}

/// Era a cui appartiene una richiesta.
enum Era {
    Moderna,
    ConInitialize,
}

/// Elabora il corpo di una richiesta POST già autenticata.
pub fn gestisci(ctx: &Contesto, h: &Intestazioni<'_>, corpo: &[u8]) -> Risposta {
    let v: Value = match serde_json::from_slice(corpo) {
        Ok(v) => v,
        Err(e) => {
            return Risposta::errore(400, None, ERR_JSON, format!("Parse error: {e}"));
        }
    };
    let o = match &v {
        Value::Object(o) => o,
        Value::Array(_) => {
            return Risposta::errore(
                400,
                None,
                ERR_RICHIESTA,
                "Invalid Request: JSON-RPC batches are not supported; send one message per \
                 POST",
            );
        }
        _ => {
            return Risposta::errore(
                400,
                None,
                ERR_RICHIESTA,
                "Invalid Request: expected a JSON-RPC object",
            );
        }
    };
    // Un identificativo valido è una stringa o un numero, mai `null`.
    let id = match o.get("id") {
        None => None,
        Some(i) if i.is_string() || i.is_number() => Some(i),
        Some(_) => {
            return Risposta::errore(
                400,
                None,
                ERR_RICHIESTA,
                "Invalid Request: `id` must be a string or a number",
            );
        }
    };
    if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Risposta::errore(
            400,
            id,
            ERR_RICHIESTA,
            "Invalid Request: `jsonrpc` must be \"2.0\"",
        );
    }
    let metodo = match o.get("method") {
        Some(Value::String(m)) => m.as_str(),
        Some(_) => {
            return Risposta::errore(
                400,
                id,
                ERR_RICHIESTA,
                "Invalid Request: `method` must be a string",
            );
        }
        // Una risposta del client (ammessa dalle revisioni con `initialize`):
        // il server non fa richieste, quindi non c'è nulla da abbinare.
        None if o.contains_key("result") || o.contains_key("error") => {
            return Risposta::accettata();
        }
        None => {
            return Risposta::errore(400, id, ERR_RICHIESTA, "Invalid Request: missing `method`");
        }
    };
    let vuoto = Map::new();
    let params = match o.get("params") {
        None => &vuoto,
        Some(Value::Object(p)) => p,
        Some(_) => {
            return Risposta::errore(
                if id.is_some() { 200 } else { 400 },
                id,
                ERR_PARAMETRI,
                "Invalid params: `params` must be an object",
            );
        }
    };
    let meta = params.get("_meta").and_then(Value::as_object);

    // Una versione dichiarata nell'intestazione deve essere fra quelle servite.
    if let Some(vh) = h.versione
        && !versioni().contains(&vh)
    {
        return versione_non_servita(id, vh);
    }

    // Senza identificativo è una notifica: nessuna risposta, solo 202.
    let Some(id) = id else {
        return Risposta::accettata();
    };

    let era = if metodo == "initialize" {
        Era::ConInitialize
    } else if meta.is_some_and(|m| m.contains_key(META_VERSIONE))
        || h.versione == Some(VERSIONE_MODERNA)
    {
        Era::Moderna
    } else {
        Era::ConInitialize
    };
    match era {
        Era::Moderna => moderna(ctx, h, id, metodo, params, meta),
        Era::ConInitialize => con_initialize(ctx, id, metodo, params),
    }
}

fn versione_non_servita(id: Option<&Value>, richiesta: &str) -> Risposta {
    Risposta::errore_con(
        400,
        id,
        ERR_VERSIONE,
        "Unsupported protocol version",
        Some(json!({ "supported": versioni(), "requested": richiesta })),
    )
}

// ---------------------------------------------------------------------------
// Revisione 2026-07-28 (senza stato)
// ---------------------------------------------------------------------------

fn moderna(
    ctx: &Contesto,
    h: &Intestazioni<'_>,
    id: &Value,
    metodo: &str,
    params: &Map<String, Value>,
    meta: Option<&Map<String, Value>>,
) -> Risposta {
    let id = Some(id);
    // Metadati obbligatori in ogni richiesta.
    let Some(versione) = meta
        .and_then(|m| m.get(META_VERSIONE))
        .and_then(Value::as_str)
    else {
        return Risposta::errore(
            400,
            id,
            ERR_PARAMETRI,
            format!("Invalid params: missing `_meta[\"{META_VERSIONE}\"]`"),
        );
    };
    if versione != VERSIONE_MODERNA {
        return versione_non_servita(id, versione);
    }
    if !meta
        .and_then(|m| m.get(META_CAPACITA))
        .is_some_and(Value::is_object)
    {
        return Risposta::errore(
            400,
            id,
            ERR_PARAMETRI,
            format!("Invalid params: missing `_meta[\"{META_CAPACITA}\"]`"),
        );
    }
    // Le intestazioni devono ripetere ciò che dice il corpo.
    let disaccordo = |m: String| Risposta::errore(400, id, ERR_INTESTAZIONI, m);
    match h.versione {
        None => {
            return disaccordo(
                "Header mismatch: the MCP-Protocol-Version header is missing".into(),
            );
        }
        Some(vh) if vh != versione => {
            return disaccordo(format!(
                "Header mismatch: MCP-Protocol-Version header value '{vh}' does not match body \
                 value '{versione}'"
            ));
        }
        Some(_) => {}
    }
    match h.metodo {
        None => return disaccordo("Header mismatch: the Mcp-Method header is missing".into()),
        Some(mh) if mh != metodo => {
            return disaccordo(format!(
                "Header mismatch: Mcp-Method header value '{mh}' does not match body value \
                 '{metodo}'"
            ));
        }
        Some(_) => {}
    }
    if metodo == "tools/call" {
        let nel_corpo = params.get("name").and_then(Value::as_str);
        match (h.nome.map(decodifica_nome), nel_corpo) {
            (None, _) => {
                return disaccordo("Header mismatch: the Mcp-Name header is missing".into());
            }
            (Some(None), _) => {
                return disaccordo(
                    "Header mismatch: the Mcp-Name header value is malformed".into(),
                );
            }
            (Some(Some(nh)), Some(nc)) if nh != nc => {
                return disaccordo(format!(
                    "Header mismatch: Mcp-Name header value '{nh}' does not match body value \
                     '{nc}'"
                ));
            }
            _ => {}
        }
    }

    let client = nome_client(meta.and_then(|m| m.get(META_CLIENT)));
    let completa = |mut risultato: Value| {
        if let Some(m) = risultato.as_object_mut() {
            m.insert("resultType".into(), json!("complete"));
            m.insert("_meta".into(), json!({ META_SERVER: info_server() }));
        }
        risultato
    };
    let con_cache = |mut risultato: Value| {
        if let Some(m) = risultato.as_object_mut() {
            m.insert("ttlMs".into(), json!(DURATA_CACHE_MS));
            // L'elenco dipende da ciò che l'utente ha scelto in questa app.
            m.insert("cacheScope".into(), json!("private"));
        }
        risultato
    };
    let mut r = match metodo {
        "server/discover" => risultato(
            id,
            con_cache(completa(json!({
                "supportedVersions": versioni(),
                "capabilities": capacita(),
                "instructions": istruzioni(ctx.livello),
            }))),
        ),
        "ping" => risultato(id, completa(json!({}))),
        "tools/list" => match elenco_strumenti(ctx, params) {
            Ok(v) => risultato(id, con_cache(completa(v))),
            Err(e) => Risposta::errore(200, id, ERR_PARAMETRI, e),
        },
        "tools/call" => match chiama_strumento(ctx, params) {
            Ok(v) => risultato(id, completa(v)),
            Err(e) => Risposta::errore(200, id, ERR_PARAMETRI, e),
        },
        // Un metodo non realizzato: 404 con l'errore JSON-RPC nel corpo.
        _ => Risposta::errore(404, id, ERR_METODO, metodo_ignoto(metodo)),
    };
    r.client = client;
    r
}

// ---------------------------------------------------------------------------
// Revisioni con `initialize` (2025-06-18, 2025-11-25)
// ---------------------------------------------------------------------------

fn con_initialize(
    ctx: &Contesto,
    id: &Value,
    metodo: &str,
    params: &Map<String, Value>,
) -> Risposta {
    let id = Some(id);
    match metodo {
        "initialize" => {
            let Some(richiesta) = params.get("protocolVersion").and_then(Value::as_str) else {
                return Risposta::errore(
                    200,
                    id,
                    ERR_PARAMETRI,
                    "Invalid params: missing `protocolVersion`",
                );
            };
            // La versione chiesta se è servita, altrimenti la più recente fra
            // quelle con `initialize`: sarà il client a decidere se proseguire.
            let scelta = if versione_con_initialize(richiesta) {
                richiesta
            } else {
                VERSIONI_CON_INITIALIZE[0]
            };
            let mut r = risultato(
                id,
                json!({
                    "protocolVersion": scelta,
                    "capabilities": capacita(),
                    "serverInfo": info_server(),
                    "instructions": istruzioni(ctx.livello),
                }),
            );
            r.client = nome_client(params.get("clientInfo"));
            r
        }
        "ping" => risultato(id, json!({})),
        "tools/list" => match elenco_strumenti(ctx, params) {
            Ok(v) => risultato(id, v),
            Err(e) => Risposta::errore(200, id, ERR_PARAMETRI, e),
        },
        "tools/call" => match chiama_strumento(ctx, params) {
            Ok(v) => risultato(id, v),
            Err(e) => Risposta::errore(200, id, ERR_PARAMETRI, e),
        },
        _ => Risposta::errore(200, id, ERR_METODO, metodo_ignoto(metodo)),
    }
}

// ---------------------------------------------------------------------------
// Parti comuni
// ---------------------------------------------------------------------------

fn risultato(id: Option<&Value>, risultato: Value) -> Risposta {
    Risposta {
        stato: 200,
        corpo: Some(json!({
            "jsonrpc": "2.0",
            "id": id.cloned().unwrap_or(Value::Null),
            "result": risultato,
        })),
        client: None,
    }
}

fn metodo_ignoto(metodo: &str) -> String {
    let m: String = metodo.chars().take(80).collect();
    format!("Method not found: {m}")
}

fn elenco_strumenti(ctx: &Contesto, params: &Map<String, Value>) -> Result<Value, String> {
    // Una pagina sola: un cursore non può essere valido.
    match params.get("cursor") {
        None | Some(Value::Null) => {}
        Some(_) => return Err("Invalid params: unknown cursor".into()),
    }
    Ok(json!({ "tools": strumenti::elenco(ctx.livello) }))
}

/// `Err` per gli errori di protocollo (strumento sconosciuto, parametri
/// malformati); gli errori di esecuzione sono risultati con `isError`.
fn chiama_strumento(ctx: &Contesto, params: &Map<String, Value>) -> Result<Value, String> {
    let nome = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("Invalid params: `name` must be a string")?;
    let vuoto = Map::new();
    let argomenti = match params.get("arguments") {
        None | Some(Value::Null) => &vuoto,
        Some(Value::Object(a)) => a,
        Some(_) => return Err("Invalid params: `arguments` must be an object".into()),
    };
    match strumenti::chiama(ctx, nome, argomenti) {
        Chiamata::Risultato(v) => Ok(json!({
            "content": [{ "type": "text", "text": v.to_string() }],
            "structuredContent": v,
            "isError": false,
        })),
        Chiamata::Errore(e) => Ok(json!({
            "content": [{ "type": "text", "text": e }],
            "isError": true,
        })),
        Chiamata::Sconosciuto => {
            let n: String = nome.chars().take(80).collect();
            Err(format!("Unknown tool: {n}"))
        }
    }
}

/// Riga di descrizione per chi apre l'indirizzo con un browser o `curl`.
pub fn descrizione_breve() -> String {
    format!(
        "OBDio MCP server: POST JSON-RPC to {PERCORSO} (Streamable HTTP, protocol versions {}).",
        versioni().join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::archivio::{Archivio, prova};
    use crate::mcp::strumenti::tests::contesto;

    const NESSUNA: Intestazioni<'static> = Intestazioni {
        versione: None,
        metodo: None,
        nome: None,
    };

    fn manda(ctx: &Contesto, h: &Intestazioni<'_>, corpo: Value) -> Risposta {
        gestisci(ctx, h, corpo.to_string().as_bytes())
    }

    fn errore(r: &Risposta) -> (u16, i64) {
        let corpo = r.corpo.as_ref().expect("corpo");
        assert_eq!(corpo["jsonrpc"], "2.0");
        assert!(corpo["error"]["message"].is_string());
        assert!(corpo.get("result").is_none());
        (r.stato, corpo["error"]["code"].as_i64().unwrap())
    }

    fn moderno(metodo: &str, mut params: Value) -> Value {
        params["_meta"] = json!({
            META_VERSIONE: VERSIONE_MODERNA,
            META_CLIENT: { "name": "prova", "version": "1.2" },
            META_CAPACITA: {},
        });
        json!({ "jsonrpc": "2.0", "id": 7, "method": metodo, "params": params })
    }

    fn intestazioni(metodo: &'static str, nome: Option<&'static str>) -> Intestazioni<'static> {
        Intestazioni {
            versione: Some(VERSIONE_MODERNA),
            metodo: Some(metodo),
            nome,
        }
    }

    #[test]
    fn errori_json_rpc() {
        let (ctx, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        // -32700: non è JSON. Senza identificativo leggibile, niente `id`.
        let r = gestisci(&ctx, &NESSUNA, b"{non json");
        assert_eq!(errore(&r), (400, -32700));
        assert!(r.corpo.as_ref().unwrap().get("id").is_none());
        assert_eq!(errore(&gestisci(&ctx, &NESSUNA, b"")), (400, -32700));
        assert_eq!(
            errore(&gestisci(&ctx, &NESSUNA, &[0xFF, 0xFE])),
            (400, -32700)
        );
        // -32600: lotti, tipi sbagliati, campi mancanti.
        for corpo in [
            json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }]),
            json!([]),
            json!("ping"),
            json!(42),
            json!(null),
            json!({ "id": 1, "method": "ping" }),
            json!({ "jsonrpc": "1.0", "id": 1, "method": "ping" }),
            json!({ "jsonrpc": "2.0", "id": 1 }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": 5 }),
            json!({ "jsonrpc": "2.0", "id": null, "method": "ping" }),
            json!({ "jsonrpc": "2.0", "id": [1], "method": "ping" }),
            json!({ "jsonrpc": "2.0", "id": true, "method": "ping" }),
        ] {
            assert_eq!(
                errore(&manda(&ctx, &NESSUNA, corpo.clone())),
                (400, -32600),
                "{corpo}"
            );
        }
        // L'identificativo, se leggibile, torna indietro.
        let r = manda(&ctx, &NESSUNA, json!({ "jsonrpc": "1.0", "id": "abc" }));
        assert_eq!(r.corpo.unwrap()["id"], "abc");
        // -32601: metodo sconosciuto (con `initialize`: HTTP 200).
        let r = manda(
            &ctx,
            &NESSUNA,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" }),
        );
        assert_eq!(errore(&r), (200, -32601));
        assert_eq!(r.corpo.unwrap()["id"], 3);
        // -32602: parametri malformati, strumento sconosciuto, cursore.
        for corpo in [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": [1] }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": 5 } }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": "get_status", "arguments": "x" } }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": "inesistente" } }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list",
                    "params": { "cursor": "abc" } }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        ] {
            assert_eq!(
                errore(&manda(&ctx, &NESSUNA, corpo.clone())),
                (200, -32602),
                "{corpo}"
            );
        }
    }

    #[test]
    fn notifiche_e_risposte_del_client_sono_accettate_senza_corpo() {
        let (ctx, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        for corpo in [
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
                    "params": { "requestId": 1 } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/sconosciuta" }),
            json!({ "jsonrpc": "2.0", "id": 9, "result": {} }),
            json!({ "jsonrpc": "2.0", "id": 9, "error": { "code": 1, "message": "x" } }),
        ] {
            let r = manda(&ctx, &NESSUNA, corpo.clone());
            assert_eq!(r.stato, 202, "{corpo}");
            assert!(r.corpo.is_none(), "{corpo}");
        }
    }

    #[test]
    fn negoziazione_della_versione_con_initialize() {
        let (ctx, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        let inizializza = |versione: &str| {
            let r = manda(
                &ctx,
                &NESSUNA,
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": versione,
                        "capabilities": {},
                        "clientInfo": { "name": "claude-code", "version": "2.1.0" },
                    }
                }),
            );
            assert_eq!(r.stato, 200);
            assert_eq!(r.client.as_deref(), Some("claude-code 2.1.0"));
            r.corpo.unwrap()["result"].clone()
        };
        // Una versione servita torna uguale.
        for v in VERSIONI_CON_INITIALIZE {
            assert_eq!(inizializza(v)["protocolVersion"], v);
        }
        // Una non servita (vecchia, futura, o quella senza stato chiesta con
        // `initialize`) riceve la più recente fra quelle con `initialize`.
        for v in [
            "2024-11-05",
            "2025-03-26",
            "2099-01-01",
            VERSIONE_MODERNA,
            "",
        ] {
            assert_eq!(inizializza(v)["protocolVersion"], "2025-11-25", "{v}");
        }
        let r = inizializza("2025-11-25");
        // Solo le capacità realizzate.
        assert_eq!(r["capabilities"], json!({ "tools": {} }));
        assert_eq!(r["serverInfo"]["name"], "obdio");
        assert_eq!(r["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
        let istruzioni = r["instructions"].as_str().unwrap();
        assert!(istruzioni.contains("NOT available over MCP"));
        assert!(istruzioni.contains("READ-ONLY"));
        // Nelle revisioni con `initialize` il risultato non ha `resultType`.
        assert!(r.get("resultType").is_none());

        // Intestazione con una versione non servita: 400.
        let h = Intestazioni {
            versione: Some("2025-03-26"),
            ..NESSUNA
        };
        let r = manda(
            &ctx,
            &h,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        );
        assert_eq!(errore(&r), (400, -32022));
        let dati = &r.corpo.as_ref().unwrap()["error"]["data"];
        assert_eq!(dati["requested"], "2025-03-26");
        assert_eq!(
            dati["supported"],
            json!(["2026-07-28", "2025-11-25", "2025-06-18"])
        );
        // Con una versione servita, o senza intestazione, la richiesta passa.
        for versione in [Some("2025-06-18"), Some("2025-11-25"), None] {
            let h = Intestazioni {
                versione,
                ..NESSUNA
            };
            let r = manda(
                &ctx,
                &h,
                json!({ "jsonrpc": "2.0", "id": "a", "method": "ping" }),
            );
            assert_eq!(r.stato, 200);
            assert_eq!(
                r.corpo.unwrap(),
                json!({ "jsonrpc": "2.0", "id": "a", "result": {} })
            );
        }
    }

    #[test]
    fn era_senza_stato() {
        let (ctx, _rx) = contesto(prova::popolato(), Livello::SolaLettura);
        let r = manda(
            &ctx,
            &intestazioni("server/discover", None),
            moderno("server/discover", json!({})),
        );
        assert_eq!(r.stato, 200);
        assert_eq!(r.client.as_deref(), Some("prova 1.2"));
        let ris = &r.corpo.as_ref().unwrap()["result"];
        assert_eq!(ris["resultType"], "complete");
        assert_eq!(
            ris["supportedVersions"],
            json!(["2026-07-28", "2025-11-25", "2025-06-18"])
        );
        assert_eq!(ris["capabilities"], json!({ "tools": {} }));
        assert_eq!(ris["_meta"][META_SERVER]["name"], "obdio");
        assert_eq!(ris["cacheScope"], "private");
        assert!(ris["ttlMs"].as_u64().is_some());
        assert!(
            ris["instructions"]
                .as_str()
                .unwrap()
                .contains("NOT available")
        );

        let r = manda(
            &ctx,
            &intestazioni("tools/list", None),
            moderno("tools/list", json!({})),
        );
        let ris = &r.corpo.as_ref().unwrap()["result"];
        assert_eq!(ris["resultType"], "complete");
        assert_eq!(ris["tools"].as_array().unwrap().len(), 8);
        assert!(ris["ttlMs"].as_u64().is_some());

        let r = manda(
            &ctx,
            &intestazioni("tools/call", Some("get_status")),
            moderno("tools/call", json!({ "name": "get_status" })),
        );
        assert_eq!(r.stato, 200);
        let ris = &r.corpo.as_ref().unwrap()["result"];
        assert_eq!(ris["resultType"], "complete");
        assert_eq!(ris["isError"], false);
        assert_eq!(ris["structuredContent"]["origin"], "demo_simulator");
        // Il testo è lo stesso oggetto, serializzato.
        let testo: Value =
            serde_json::from_str(ris["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(testo["connection"], "connected");
        // I risultati di `tools/call` non sono da tenere in memoria.
        assert!(ris.get("ttlMs").is_none());

        // Il nome con la sentinella Base64 vale quanto quello in chiaro.
        let r = manda(
            &ctx,
            &intestazioni("tools/call", Some("=?base64?Z2V0X3N0YXR1cw==?=")),
            moderno("tools/call", json!({ "name": "get_status" })),
        );
        assert_eq!(r.stato, 200);

        // Metodo non realizzato: 404 con -32601.
        let r = manda(
            &ctx,
            &intestazioni("resources/list", None),
            moderno("resources/list", json!({})),
        );
        assert_eq!(errore(&r), (404, -32601));
        // `initialize` non appartiene a questa era: resta servito alla vecchia
        // maniera anche se arriva con l'intestazione nuova.
        let r = manda(
            &ctx,
            &intestazioni("initialize", None),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "protocolVersion": "2026-07-28" } }),
        );
        assert_eq!(r.corpo.unwrap()["result"]["protocolVersion"], "2025-11-25");
    }

    #[test]
    fn era_senza_stato_metadati_e_intestazioni() {
        let (ctx, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        let h = intestazioni("tools/list", None);
        // Versione non servita nel corpo.
        let mut c = moderno("tools/list", json!({}));
        c["params"]["_meta"][META_VERSIONE] = json!("2099-01-01");
        assert_eq!(errore(&manda(&ctx, &h, c)), (400, -32022));
        // Capacità del client mancanti.
        let mut c = moderno("tools/list", json!({}));
        c["params"]["_meta"]
            .as_object_mut()
            .unwrap()
            .remove(META_CAPACITA);
        assert_eq!(errore(&manda(&ctx, &h, c)), (400, -32602));
        // Intestazione nuova ma corpo senza metadati.
        let c = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        assert_eq!(errore(&manda(&ctx, &h, c)), (400, -32602));
        // Intestazioni mancanti o in disaccordo con il corpo: -32020.
        for h in [
            NESSUNA,
            Intestazioni {
                versione: Some("2025-11-25"),
                metodo: Some("tools/list"),
                nome: None,
            },
            Intestazioni {
                versione: Some(VERSIONE_MODERNA),
                metodo: None,
                nome: None,
            },
            intestazioni("tools/call", None),
        ] {
            let r = manda(&ctx, &h, moderno("tools/list", json!({})));
            assert_eq!(errore(&r), (400, -32020), "{h:?}");
        }
        for nome in [None, Some("get_diagnosis"), Some("=?base64?@@@?=")] {
            let r = manda(
                &ctx,
                &intestazioni("tools/call", nome),
                moderno("tools/call", json!({ "name": "get_status" })),
            );
            assert_eq!(errore(&r), (400, -32020), "{nome:?}");
        }
        // Il client non dichiarato non è un errore.
        let mut c = moderno("ping", json!({}));
        c["params"]["_meta"]
            .as_object_mut()
            .unwrap()
            .remove(META_CLIENT);
        let r = manda(&ctx, &intestazioni("ping", None), c);
        assert_eq!(r.stato, 200);
        assert_eq!(r.client, None);
    }

    #[test]
    fn strumenti_secondo_il_livello() {
        let chiama = |ctx: &Contesto, nome: &str, argomenti: Value| {
            let r = manda(
                ctx,
                &NESSUNA,
                json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": { "name": nome, "arguments": argomenti } }),
            );
            r.corpo.unwrap()
        };
        let elenco = |ctx: &Contesto| {
            let r = manda(
                ctx,
                &NESSUNA,
                json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            );
            r.corpo.unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let (lettura, rx) = contesto(prova::popolato(), Livello::SolaLettura);
        assert_eq!(elenco(&lettura).len(), 8);
        assert!(!elenco(&lettura).contains(&"start_reading".to_string()));
        // Un'azione in sola lettura: errore di esecuzione, spiegato al modello.
        let r = chiama(&lettura, "start_reading", json!({ "keys": ["giri"] }));
        assert_eq!(r["result"]["isError"], true);
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("read-only")
        );
        assert!(r["result"].get("structuredContent").is_none());
        assert!(rx.try_recv().is_err());
        // Argomenti sbagliati: errore di esecuzione, non di protocollo.
        let r = chiama(&lettura, "get_sensor_history", json!({ "key": "boh" }));
        assert_eq!(r["result"]["isError"], true);
        // La cancellazione dei codici non esiste, con nessun livello.
        let (azioni, rx) = contesto(prova::popolato(), Livello::LetturaEAzioni);
        assert_eq!(elenco(&azioni).len(), 14);
        for ctx in [&lettura, &azioni] {
            for nome in ["clear_dtc", "erase_codes", "cancella_dtc"] {
                let r = chiama(ctx, nome, json!({}));
                assert_eq!(r["error"]["code"], -32602, "{nome}");
                assert!(
                    r["error"]["message"]
                        .as_str()
                        .unwrap()
                        .contains("Unknown tool")
                );
            }
            assert!(
                !elenco(ctx)
                    .iter()
                    .any(|n| n.contains("erase") || n.contains("clear") || n.contains("cancel"))
            );
        }
        assert!(rx.try_recv().is_err());
        assert!(istruzioni(Livello::LetturaEAzioni).contains("NOT available over MCP"));
        assert!(!istruzioni(Livello::LetturaEAzioni).contains("READ-ONLY"));
    }

    #[test]
    fn sentinella_base64_e_nome_del_client() {
        assert_eq!(decodifica_nome("get_status").as_deref(), Some("get_status"));
        assert_eq!(
            decodifica_nome("=?base64?SGVsbG8sIOS4lueVjA==?=").as_deref(),
            Some("Hello, 世界")
        );
        assert_eq!(
            decodifica_nome("=?base64?IHBhZGRlZCA=?=").as_deref(),
            Some(" padded ")
        );
        assert_eq!(decodifica_nome("=?base64?!!?="), None);
        assert_eq!(decodifica_nome("=?base64?/w==?="), None);

        assert_eq!(
            nome_client(Some(&json!({ "name": "Claude", "version": "1.0" }))).as_deref(),
            Some("Claude 1.0")
        );
        assert_eq!(
            nome_client(Some(&json!({ "name": "solo" }))).as_deref(),
            Some("solo")
        );
        // Caratteri di controllo tolti, lunghezza limitata.
        let lungo = nome_client(Some(
            &json!({ "name": format!("a\r\n{}", "b".repeat(500)), "version": "1" }),
        ))
        .unwrap();
        assert!(lungo.chars().count() <= 62 && !lungo.contains('\n'));
        assert_eq!(nome_client(Some(&json!({ "name": "" }))), None);
        assert_eq!(nome_client(Some(&json!("x"))), None);
        assert_eq!(nome_client(None), None);
        assert!(descrizione_breve().contains("2026-07-28"));
    }
}
