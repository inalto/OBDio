//! Gli strumenti («tools») offerti all'assistente AI.
//!
//! Nomi, descrizioni e campi dei risultati sono in inglese: li legge un
//! modello linguistico, non l'utente. Le chiavi dei sensori restano quelle
//! stabili dell'app (`giri`, `velocita`, …) e i nomi leggibili quelli
//! italiani dell'interfaccia.
//!
//! Regole che valgono per ogni strumento:
//! - ogni risultato dice da dove vengono i dati (`origin`): veicolo,
//!   simulatore o riproduzione di una sessione registrata;
//! - un valore assente è `null` con il suo stato, mai zero;
//! - i risultati hanno dimensione limitata;
//! - gli strumenti di sola lettura leggono l'archivio o i file delle
//!   sessioni e non parlano mai con l'adattatore;
//! - le azioni passano da [`Azione`]: la cancellazione dei codici guasto non
//!   c'è e non può esserci.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::archivio::{Archivio, DiagnosiLetta, Interno, Punto, Statistiche};
use super::{Azione, Livello};
use crate::diagnosis::{Lettura, StatoDtc};
use crate::model::{Origine, Stato};
use crate::obd::dtc::{self, LinguaDescrizione};
use crate::obd::pids;
use crate::recording::{self, SessionData};
use crate::worker::StatoConn;

/// Nota riportata dove il VIN potrebbe essere atteso.
const NOTA_VIN: &str = "The VIN is never exposed over MCP, by design.";
/// Nota su ciò che un assistente non può fare.
pub const NOTA_CANCELLAZIONE: &str = "Erasing trouble codes (OBD service 04) is NOT available \
     over MCP, by design: it requires a person at the OBDio screen. Do not look for a tool that \
     does it; tell the user to use the Diagnosi page of the app.";

const SECONDI_STORIA_PREDEFINITI: u64 = 60;
const SECONDI_STORIA_MAX: u64 = 3600;
const PUNTI_PREDEFINITI: u64 = 120;
const PUNTI_MAX: u64 = 500;
const CHIAVI_MAX: usize = 100;
const SESSIONI_PREDEFINITE: u64 = 10;
const SESSIONI_MAX: u64 = 25;
/// Oltre questa dimensione una sessione non viene aperta per il riepilogo.
const SESSIONE_MAX_BYTE: u64 = 64 * 1024 * 1024;
const MARCATORI_MAX: usize = 100;
const PARAMETRI_MAX: usize = 200;
const LUNGHEZZA_MARCATORE: usize = 80;

const ATTESA_DIAGNOSI: Duration = Duration::from_secs(45);
const ATTESA_BREVE: Duration = Duration::from_secs(4);
const ATTESA_CHIUSURA_REGISTRAZIONE: Duration = Duration::from_secs(15);

/// Ciò che serve agli strumenti per rispondere.
pub struct Contesto {
    pub archivio: Arc<Archivio>,
    /// Verso il thread di lavoro: solo [`Azione`], mai un comando qualsiasi.
    pub azioni: Sender<Azione>,
    pub livello: Livello,
    pub cartella_sessioni: PathBuf,
    /// Vero quando il server sta chiudendo: le attese terminano subito.
    pub stop: Arc<AtomicBool>,
    /// Una sola azione alla volta.
    pub azione_in_corso: Mutex<()>,
}

/// Esito della chiamata di uno strumento.
#[derive(Clone, Debug, PartialEq)]
pub enum Chiamata {
    /// Oggetto JSON del risultato.
    Risultato(Value),
    /// Errore di esecuzione, da riportare al modello (`isError: true`).
    Errore(String),
    /// Lo strumento non esiste.
    Sconosciuto,
}

type Esito = Result<Value, String>;
type Argomenti = Map<String, Value>;

struct Definizione {
    nome: &'static str,
    titolo: &'static str,
    descrizione: &'static str,
    /// Vero se cambia lo stato dell'app o interroga la centralina.
    azione: bool,
    idempotente: bool,
    schema: fn() -> Value,
    esegui: fn(&Contesto, &Argomenti) -> Esito,
}

fn senza_argomenti() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

const DEFINIZIONI: &[Definizione] = &[
    Definizione {
        nome: "get_status",
        titolo: "OBDio status",
        descrizione: "Connection state of the OBDio diagnostic app and where its data comes \
            from. ALWAYS check `origin` before interpreting any data: `vehicle` is a real car, \
            `demo_simulator` is synthetic data from the built-in simulator (not a real car), \
            `replay` is a previously recorded session being played back (not current readings). \
            Also reports adapter, protocol, responding ECUs, whether live reading and recording \
            are active, and the replay position.",
        azione: false,
        idempotente: true,
        schema: senza_argomenti,
        esegui: get_status,
    },
    Definizione {
        nome: "list_sensors",
        titolo: "List sensors",
        descrizione: "Parameters (OBD-II service 01 PIDs and values computed by the app) that \
            the connected ECU declares as supported: stable key, human-readable name (Italian), \
            unit, group, and whether each one is currently being read. Use the keys with \
            get_live_values, get_sensor_history and start_reading.",
        azione: false,
        idempotente: true,
        schema: senza_argomenti,
        esegui: list_sensors,
    },
    Definizione {
        nome: "get_live_values",
        titolo: "Latest sensor values",
        descrizione: "Latest reading of each sensor read since the connection started: value \
            (null when the read failed - never 0 as a placeholder), unit, status, age in \
            seconds, ECU, plus min/max/mean of the valid readings since connection. Only \
            sensors that are being read have fresh values: check `age_s` and `reading_active`. \
            Optional `keys` limits the answer to those sensors.",
        azione: false,
        idempotente: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "keys": {
                        "type": "array",
                        "items": { "type": "string" },
                        "maxItems": CHIAVI_MAX,
                        "description": "Sensor keys from list_sensors. Omit for all sensors \
                            that have data."
                    }
                },
                "additionalProperties": false
            })
        },
        esegui: get_live_values,
    },
    Definizione {
        nome: "get_sensor_history",
        titolo: "Recent samples of one sensor",
        descrizione: "Recent samples of one sensor over the last N seconds (the app keeps the \
            last 2000 samples per sensor). If there are more samples than `max_points` they \
            are downsampled into time buckets (mean, min, max, sample and missing counts) and \
            the result says so. Failed reads are kept as null with their status.",
        azione: false,
        idempotente: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Sensor key from list_sensors." },
                    "seconds": {
                        "type": "integer", "minimum": 1, "maximum": SECONDI_STORIA_MAX,
                        "default": SECONDI_STORIA_PREDEFINITI,
                        "description": "Length of the window, counted back from the latest \
                            sample."
                    },
                    "max_points": {
                        "type": "integer", "minimum": 10, "maximum": PUNTI_MAX,
                        "default": PUNTI_PREDEFINITI,
                        "description": "Maximum number of points returned."
                    }
                },
                "required": ["key"],
                "additionalProperties": false
            })
        },
        esegui: get_sensor_history,
    },
    Definizione {
        nome: "get_diagnosis",
        titolo: "Last diagnosis report",
        descrizione: "The last diagnosis read by the app, without querying the car again: \
            trouble codes (DTCs) with description and states (confirmed, pending, permanent), \
            MIL lamp, readiness monitors, freeze frame, Mode 06 catalyst monitor results (raw \
            and scaled), fuel type, OBD standard, and when it was read. Says clearly if no \
            diagnosis has been read yet. The report lists what the ECU says; it does not \
            establish which component is faulty. The VIN is never included.",
        azione: false,
        idempotente: true,
        schema: senza_argomenti,
        esegui: get_diagnosis,
    },
    Definizione {
        nome: "lookup_dtc",
        titolo: "Look up a trouble code",
        descrizione: "Description of a diagnostic trouble code (for example P0420) from the \
            database embedded in the app, and whether the code is generic (SAE J2012, same \
            meaning on every make) or manufacturer-specific (meaning not included: it depends \
            on the make). Works without a connection.",
        azione: false,
        idempotente: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "pattern": "^[PCBUpcbu][0-3][0-9A-Fa-f]{3}$",
                        "description": "Five-character code such as P0420, C1234, U0100."
                    }
                },
                "required": ["code"],
                "additionalProperties": false
            })
        },
        esegui: lookup_dtc,
    },
    Definizione {
        nome: "list_sessions",
        titolo: "List recorded sessions",
        descrizione: "Sessions recorded or imported by the app and stored on disk, newest \
            first: file name, date, whether the data came from a vehicle or from the demo \
            simulator, duration, sample count, parameters, markers. Use get_session_summary \
            with a `name` from this list for per-parameter statistics.",
        azione: false,
        idempotente: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer", "minimum": 1, "maximum": SESSIONI_MAX,
                        "default": SESSIONI_PREDEFINITE
                    },
                    "offset": { "type": "integer", "minimum": 0, "default": 0 }
                },
                "additionalProperties": false
            })
        },
        esegui: list_sessions,
    },
    Definizione {
        nome: "get_session_summary",
        titolo: "Summary of a recorded session",
        descrizione: "Summary of one recorded session: date, source (vehicle or demo \
            simulator), duration, markers, the trouble codes stored with it, and for every \
            parameter the number of samples, failed reads, minimum, maximum and mean.",
        azione: false,
        idempotente: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "File name exactly as returned by list_sessions."
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            })
        },
        esegui: get_session_summary,
    },
    Definizione {
        nome: "read_diagnosis",
        titolo: "Read the diagnosis from the car now",
        descrizione: "ACTION. Asks the ECU for a fresh diagnosis (trouble codes, MIL, \
            readiness, freeze frame, Mode 06) using read-only OBD services, then returns it in \
            the same shape as get_diagnosis. It does not change anything in the vehicle, but \
            it takes several seconds, pauses live reading meanwhile and updates what the app \
            shows. Needs a live connection (not a replay).",
        azione: true,
        idempotente: true,
        schema: senza_argomenti,
        esegui: read_diagnosis,
    },
    Definizione {
        nome: "start_reading",
        titolo: "Start reading sensors",
        descrizione: "ACTION. Starts (or replaces) the continuous reading of the given sensor \
            keys; values then appear in get_live_values and get_sensor_history. Fewer keys \
            means a faster refresh of each one. The person using the app can change or stop \
            the selection at any time from the Sensori and Cruscotto pages.",
        azione: true,
        idempotente: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "keys": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "maxItems": CHIAVI_MAX,
                        "description": "Sensor keys from list_sensors."
                    }
                },
                "required": ["keys"],
                "additionalProperties": false
            })
        },
        esegui: start_reading,
    },
    Definizione {
        nome: "stop_reading",
        titolo: "Stop reading sensors",
        descrizione: "ACTION. Stops the continuous reading of sensors. Values already read \
            stay available and simply get older.",
        azione: true,
        idempotente: true,
        schema: senza_argomenti,
        esegui: stop_reading,
    },
    Definizione {
        nome: "add_marker",
        titolo: "Add a marker",
        descrizione: "ACTION. Adds a time marker with a short label at the current moment (for \
            example \"2500 rpm steady\"). It appears on the live graphs and, if a recording is \
            active, is saved in the session file.",
        azione: true,
        idempotente: false,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string", "minLength": 1, "maxLength": LUNGHEZZA_MARCATORE,
                        "description": "Label of the marker."
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            })
        },
        esegui: add_marker,
    },
    Definizione {
        nome: "start_recording",
        titolo: "Start recording a session",
        descrizione: "ACTION. Starts recording the readings to a session file on disk. Only \
            sensors that are being read are recorded: call start_reading first. Does nothing \
            if a recording is already active.",
        azione: true,
        idempotente: true,
        schema: senza_argomenti,
        esegui: start_recording,
    },
    Definizione {
        nome: "stop_recording",
        titolo: "Stop recording",
        descrizione: "ACTION. Stops the recording in progress and closes the session file (a \
            CSV copy is written next to it). The session then appears in list_sessions.",
        azione: true,
        idempotente: true,
        schema: senza_argomenti,
        esegui: stop_recording,
    },
];

/// Nomi di tutti gli strumenti esistenti, azioni comprese.
pub fn nomi() -> Vec<&'static str> {
    DEFINIZIONI.iter().map(|d| d.nome).collect()
}

/// Vero se lo strumento esiste ed è un'azione.
pub fn e_azione(nome: &str) -> bool {
    DEFINIZIONI.iter().any(|d| d.nome == nome && d.azione)
}

/// Definizioni degli strumenti visibili con il livello dato, nell'ordine
/// fisso di [`DEFINIZIONI`].
pub fn elenco(livello: Livello) -> Vec<Value> {
    DEFINIZIONI
        .iter()
        .filter(|d| !d.azione || livello == Livello::LetturaEAzioni)
        .map(|d| {
            json!({
                "name": d.nome,
                "title": d.titolo,
                "description": d.descrizione,
                "inputSchema": (d.schema)(),
                "annotations": {
                    "title": d.titolo,
                    "readOnlyHint": !d.azione,
                    "destructiveHint": false,
                    "idempotentHint": d.idempotente,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

/// Esegue lo strumento `nome` con gli argomenti dati.
pub fn chiama(ctx: &Contesto, nome: &str, argomenti: &Argomenti) -> Chiamata {
    let Some(d) = DEFINIZIONI.iter().find(|d| d.nome == nome) else {
        return Chiamata::Sconosciuto;
    };
    if d.azione && ctx.livello != Livello::LetturaEAzioni {
        return Chiamata::Errore(format!(
            "The tool `{nome}` is an action, and the OBDio MCP server is set to read-only by \
             its user. Nothing was done. If the user wants you to perform actions, they must \
             choose \"Lettura e azioni\" in OBDio, page Impostazioni, card \"Server MCP per \
             assistenti AI\"."
        ));
    }
    if let Err(e) = controlla_argomenti(&(d.schema)(), argomenti) {
        return Chiamata::Errore(e);
    }
    let esito = if d.azione {
        // Una sola azione alla volta: le attese non si accavallano.
        match ctx.azione_in_corso.try_lock() {
            Ok(_guardia) => (d.esegui)(ctx, argomenti),
            Err(std::sync::TryLockError::Poisoned(g)) => {
                let _guardia = g.into_inner();
                (d.esegui)(ctx, argomenti)
            }
            Err(std::sync::TryLockError::WouldBlock) => Err(
                "Another action requested over MCP is still in progress. Wait for it to \
                 finish, then retry."
                    .into(),
            ),
        }
    } else {
        (d.esegui)(ctx, argomenti)
    };
    match esito {
        Ok(v) => Chiamata::Risultato(v),
        Err(e) => Chiamata::Errore(e),
    }
}

/// Rifiuta gli argomenti che lo schema non prevede e segnala quelli
/// obbligatori mancanti: un modello che sbaglia nome riceve subito l'elenco
/// giusto.
fn controlla_argomenti(schema: &Value, a: &Argomenti) -> Result<(), String> {
    let ammessi: Vec<&str> = schema["properties"]
        .as_object()
        .map(|p| p.keys().map(String::as_str).collect())
        .unwrap_or_default();
    if let Some(ignoto) = a.keys().find(|k| !ammessi.contains(&k.as_str())) {
        return Err(if ammessi.is_empty() {
            format!("Unknown argument `{ignoto}`: this tool takes no arguments.")
        } else {
            format!(
                "Unknown argument `{ignoto}`. Allowed arguments: {}.",
                ammessi.join(", ")
            )
        });
    }
    for r in schema["required"].as_array().into_iter().flatten() {
        if let Some(nome) = r.as_str()
            && a.get(nome).is_none_or(Value::is_null)
        {
            return Err(format!("Missing required argument `{nome}`."));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pezzi comuni
// ---------------------------------------------------------------------------

/// Codice dell'origine per l'assistente.
pub fn origine_codice(o: Origine) -> &'static str {
    match o {
        Origine::Veicolo => "vehicle",
        Origine::Demo => "demo_simulator",
        Origine::Riproduzione => "replay",
    }
}

fn origine_nota(o: Option<Origine>) -> &'static str {
    match o {
        Some(Origine::Veicolo) => "Data read from a real vehicle ECU.",
        Some(Origine::Demo) => {
            "SIMULATED data from OBDio's built-in demo simulator. It does NOT come from a real \
             vehicle: do not draw conclusions about any car from it."
        }
        Some(Origine::Riproduzione) => {
            "REPLAY of a previously recorded session. These are NOT current readings of a \
             vehicle."
        }
        None => "Not connected: there is no live data source.",
    }
}

/// Campi `origin` e `origin_note` comuni ai risultati.
fn con_origine(o: Option<Origine>, mut v: Value) -> Value {
    if let Some(m) = v.as_object_mut() {
        let mut nuovo = Map::new();
        nuovo.insert("origin".into(), json!(o.map(origine_codice)));
        nuovo.insert("origin_note".into(), json!(origine_nota(o)));
        nuovo.append(m);
        *m = nuovo;
    }
    v
}

fn stato_codice(s: Stato) -> &'static str {
    match s {
        Stato::Ok => "ok",
        Stato::NonSupportato => "not_supported",
        Stato::RispostaMancante => "no_response",
        Stato::ErroreComunicazione => "communication_error",
    }
}

fn stato_conn_codice(s: &StatoConn) -> &'static str {
    match s {
        StatoConn::Scollegato => "disconnected",
        StatoConn::Scansione => "scanning_bluetooth",
        StatoConn::Connessione(_) => "connecting",
        StatoConn::Connesso => "connected",
        StatoConn::Diagnosi => "reading_diagnosis",
        StatoConn::Cancellazione => "user_is_erasing_codes",
        StatoConn::Disconnessione => "disconnecting",
    }
}

fn arrotonda(v: f64, decimali: usize) -> f64 {
    let f = 10f64.powi(decimali.min(9) as i32);
    let r = (v * f).round() / f;
    if r.is_finite() { r } else { v }
}

fn decimali_di(key: &str) -> usize {
    pids::per_key(key).map(|p| p.decimali).unwrap_or(2)
}

fn nome_di(key: &str) -> String {
    pids::per_key(key)
        .map(|p| p.nome.clone())
        .unwrap_or_else(|| key.to_string())
}

fn statistiche_json(key: &str, s: &Statistiche) -> Value {
    let d = decimali_di(key) + 2;
    json!({
        "min": s.min,
        "max": s.max,
        "mean": s.media().map(|m| arrotonda(m, d)),
        "valid_samples": s.conteggio,
    })
}

fn intero(a: &Argomenti, nome: &str, predefinito: u64, min: u64, max: u64) -> Result<u64, String> {
    match a.get(nome) {
        None | Some(Value::Null) => Ok(predefinito),
        Some(v) => match v.as_u64() {
            Some(n) if (min..=max).contains(&n) => Ok(n),
            _ => Err(format!(
                "Invalid `{nome}`: expected an integer from {min} to {max}, got {v}."
            )),
        },
    }
}

fn testo<'a>(a: &'a Argomenti, nome: &str) -> Result<&'a str, String> {
    a.get(nome)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Invalid `{nome}`: expected a string."))
}

fn elenco_testi(a: &Argomenti, nome: &str) -> Result<Option<Vec<String>>, String> {
    let Some(v) = a.get(nome).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let errore = || format!("Invalid `{nome}`: expected an array of strings.");
    let lista = v.as_array().ok_or_else(errore)?;
    if lista.len() > CHIAVI_MAX {
        return Err(format!(
            "Invalid `{nome}`: at most {CHIAVI_MAX} items are accepted."
        ));
    }
    lista
        .iter()
        .map(|x| x.as_str().map(|s| s.trim().to_string()).ok_or_else(errore))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Messaggio per una chiave che non esiste fra i sensori disponibili.
fn chiave_ignota(key: &str) -> String {
    format!(
        "Unknown or unavailable sensor key `{key}`. Call list_sensors for the keys available \
         on this connection."
    )
}

// ---------------------------------------------------------------------------
// Sola lettura
// ---------------------------------------------------------------------------

fn get_status(ctx: &Contesto, _a: &Argomenti) -> Esito {
    Ok(ctx.archivio.leggi(|i| {
        let c = i.collegamento.as_ref();
        let riproduzione = c.and_then(|c| c.riproduzione.as_ref()).map(|r| {
            let s = i.riproduzione.as_ref();
            json!({
                "file": r.file,
                "recorded_at": r.registrata_il,
                "recorded_from": if r.sessione_demo { "demo_simulator" } else { "vehicle" },
                "imported_from": r.importata,
                "position_s": s.map(|s| arrotonda(s.posizione, 1)),
                "duration_s": s.map(|s| arrotonda(s.durata, 1)),
                "paused": s.map(|s| s.in_pausa),
                "speed": s.map(|s| s.velocita),
                "loop": s.map(|s| s.ripeti),
                "finished": s.map(|s| s.finita),
            })
        });
        let mut chiavi: Vec<&str> = i
            .parametri()
            .into_iter()
            .filter(|p| i.in_lettura(p))
            .map(|p| p.key.as_str())
            .collect();
        chiavi.sort_unstable();
        con_origine(
            i.origine(),
            json!({
                "app": "OBDio",
                "version": env!("CARGO_PKG_VERSION"),
                "connection": stato_conn_codice(&i.stato),
                "connected": c.is_some(),
                "link": c.map(|c| c.collegamento.as_str()),
                "adapter": c.map(|c| c.adattatore.as_str()),
                "protocol": c.map(|c| c.protocollo.as_str()),
                "ecus": c.map(|c| c.ecu.clone()).unwrap_or_default(),
                "warnings": c.map(|c| c.avvisi.clone()).unwrap_or_default(),
                "reading_active": i.lettura || (i.in_riproduzione()
                    && i.riproduzione.as_ref().is_some_and(|r| !r.in_pausa && !r.finita)),
                "reading_keys": chiavi,
                "t_now_s": arrotonda(i.t_ultimo, 2),
                "recording": {
                    "active": i.registrazione.is_some(),
                    "file": i.registrazione.as_ref().map(|r| r.file.as_str()),
                    "started_at_t_s": i.registrazione.as_ref().map(|r| arrotonda(r.t_inizio, 2)),
                    "last_closed_file": i.ultima_registrazione.as_ref().map(|u| u.0.as_str()),
                    "last_closed_samples": i.ultima_registrazione.as_ref().map(|u| u.1),
                },
                "replay": riproduzione,
                "diagnosis_available": i.diagnosi.is_some(),
                "diagnosis_read_at": i.diagnosi.as_ref().map(|d| d.referto.letto_il.as_str()),
                "last_error": i.errore.as_ref().map(|(t, m)| json!({ "title": t, "message": m })),
                "mcp_permission_level": match ctx.livello {
                    Livello::SolaLettura => "read_only",
                    Livello::LetturaEAzioni => "read_and_actions",
                },
                "erase_codes_over_mcp": "never_available",
                "language_note": "Sensor keys, sensor names, monitor names and error messages \
                    of the app are in Italian.",
            }),
        )
    }))
}

fn list_sensors(ctx: &Contesto, _a: &Argomenti) -> Esito {
    Ok(ctx.archivio.leggi(|i| {
        let sensori: Vec<Value> = i
            .parametri()
            .into_iter()
            .map(|p| {
                json!({
                    "key": p.key,
                    "name": p.nome,
                    "unit": p.unita,
                    "group": p.gruppo.nome(),
                    "pid": (!p.derivato && !p.libero).then(|| format!("{:02X}", p.pid)),
                    "computed_by_app": p.derivato,
                    "being_read": i.in_lettura(p),
                    "has_data": i.sensori.contains_key(&p.key),
                })
            })
            .collect();
        let nota = if i.collegamento.is_none() {
            "Not connected: only sensors with data left in memory from the last connection \
             are listed."
        } else if i.in_riproduzione() {
            "Replay: the sensors present in the recorded session."
        } else {
            "Sensors the ECU declares as supported. Declared does not guarantee an answer: \
             a failed read shows as null with its status."
        };
        con_origine(
            i.origine(),
            json!({
                "connected": i.collegamento.is_some(),
                "reading_active": i.lettura,
                "count": sensori.len(),
                "sensors": sensori,
                "note": nota,
            }),
        )
    }))
}

fn get_live_values(ctx: &Contesto, a: &Argomenti) -> Esito {
    let filtro = elenco_testi(a, "keys")?;
    Ok(ctx.archivio.leggi(|i| {
        let mut valori = Map::new();
        let mut senza_dati: Vec<String> = Vec::new();
        let mut ignote: Vec<String> = Vec::new();
        let disponibili: BTreeSet<&str> = i.parametri().iter().map(|p| p.key.as_str()).collect();
        let chiavi: Vec<String> = match &filtro {
            Some(f) => f.clone(),
            None => i.sensori.keys().cloned().collect(),
        };
        for k in chiavi {
            let Some(s) = i.sensori.get(&k) else {
                if disponibili.contains(k.as_str()) {
                    senza_dati.push(k);
                } else {
                    ignote.push(k);
                }
                continue;
            };
            let mut v = json!({
                "name": nome_di(&k),
                "value": s.ultimo.v,
                "unit": s.unita,
                "status": stato_codice(s.ultimo.stato),
                "age_s": arrotonda(s.arrivo.elapsed().as_secs_f64(), 1),
                "t_s": arrotonda(s.ultimo.t, 2),
                "timestamp": s.ts,
                "ecu": (!s.ecu.is_empty()).then_some(s.ecu.as_str()),
                "samples": s.letture,
                "failed_reads": s.mancanti,
            });
            if let (Some(m), Some(st)) = (
                v.as_object_mut(),
                statistiche_json(&k, &s.stat).as_object_mut(),
            ) {
                m.append(st);
            }
            valori.insert(k, v);
        }
        con_origine(
            i.origine(),
            json!({
                "connected": i.collegamento.is_some(),
                "reading_active": i.lettura,
                "t_now_s": arrotonda(i.t_ultimo, 2),
                "values": valori,
                "keys_without_data_yet": senza_dati,
                "unknown_keys": ignote,
                "note": "A null value is a read that failed: see `status`. Statistics (min, \
                    max, mean) cover the valid readings since the connection started. When not \
                    connected or not reading, values are the last ones read: check `age_s`.",
            }),
        )
    }))
}

/// Un secchio della storia ridotta.
#[derive(Default)]
struct Secchio {
    t_da: f64,
    t_a: f64,
    stat: Statistiche,
    letture: u64,
    mancanti: u64,
}

/// Riduce `punti` ad al più `max` secchi di uguale durata.
fn riduci(punti: &[Punto], max: usize) -> Vec<Value> {
    let (Some(primo), Some(ultimo)) = (punti.first(), punti.last()) else {
        return Vec::new();
    };
    let durata = (ultimo.t - primo.t).max(0.0);
    let mut secchi: Vec<Option<Secchio>> = (0..max).map(|_| None).collect();
    for p in punti {
        let indice = if durata > 0.0 {
            (((p.t - primo.t) / durata) * max as f64).floor() as usize
        } else {
            0
        }
        .min(max - 1);
        let s = secchi[indice].get_or_insert_with(|| Secchio {
            t_da: p.t,
            t_a: p.t,
            ..Default::default()
        });
        s.t_da = s.t_da.min(p.t);
        s.t_a = s.t_a.max(p.t);
        s.letture += 1;
        match p.v {
            Some(v) => s.stat.aggiungi(v),
            None => s.mancanti += 1,
        }
    }
    secchi
        .into_iter()
        .flatten()
        .map(|s| {
            json!({
                "t_from_s": arrotonda(s.t_da, 2),
                "t_to_s": arrotonda(s.t_a, 2),
                "value": s.stat.media().map(|m| arrotonda(m, 4)),
                "min": s.stat.min,
                "max": s.stat.max,
                "samples": s.letture,
                "failed_reads": s.mancanti,
            })
        })
        .collect()
}

fn get_sensor_history(ctx: &Contesto, a: &Argomenti) -> Esito {
    let key = testo(a, "key")?.trim().to_string();
    let secondi = intero(
        a,
        "seconds",
        SECONDI_STORIA_PREDEFINITI,
        1,
        SECONDI_STORIA_MAX,
    )?;
    let max = intero(a, "max_points", PUNTI_PREDEFINITI, 10, PUNTI_MAX)? as usize;
    // Sotto il mutex si copia soltanto; il resto si calcola fuori.
    let trovato = ctx.archivio.leggi(|i| {
        let s = i.sensori.get(&key)?;
        let da = i.t_ultimo - secondi as f64;
        let punti: Vec<Punto> = s.storia.iter().filter(|p| p.t >= da).copied().collect();
        Some((
            punti,
            s.unita.clone(),
            s.ecu.clone(),
            s.storia.len(),
            i.origine(),
            i.t_ultimo,
        ))
    });
    let Some((punti, unita, ecu, in_memoria, origine, t_ora)) = trovato else {
        let disponibile = ctx
            .archivio
            .leggi(|i| i.parametri().iter().any(|p| p.key == key));
        return Err(if disponibile {
            format!(
                "No samples yet for `{key}`: it is available but has not been read since the \
                 connection started. Start reading it (start_reading, or the Sensori page of \
                 the app) and retry."
            )
        } else {
            chiave_ignota(&key)
        });
    };
    let mut stat = Statistiche::default();
    let mut mancanti = 0u64;
    for p in &punti {
        match p.v {
            Some(v) => stat.aggiungi(v),
            None => mancanti += 1,
        }
    }
    let ridotta = punti.len() > max;
    let serie: Vec<Value> = if ridotta {
        riduci(&punti, max)
    } else {
        punti
            .iter()
            .map(|p| {
                json!({
                    "t_s": arrotonda(p.t, 2),
                    "value": p.v,
                    "status": stato_codice(p.stato),
                })
            })
            .collect()
    };
    let mut riepilogo = statistiche_json(&key, &stat);
    if let Some(m) = riepilogo.as_object_mut() {
        m.insert("failed_reads".into(), json!(mancanti));
    }
    Ok(con_origine(
        origine,
        json!({
            "key": key,
            "name": nome_di(&key),
            "unit": unita,
            "ecu": (!ecu.is_empty()).then_some(ecu),
            "window_s": secondi,
            "t_now_s": arrotonda(t_ora, 2),
            "raw_samples_in_window": punti.len(),
            "samples_kept_in_memory": in_memoria,
            "downsampled": ridotta,
            "downsampling": if ridotta {
                json!(format!(
                    "{} raw samples reduced to at most {max} equal-duration time buckets; \
                     `value` is the mean of the valid samples in the bucket (null if none), \
                     with min, max, sample count and failed reads.",
                    punti.len()
                ))
            } else {
                json!("none: every raw sample in the window is returned")
            },
            "window_statistics": riepilogo,
            "points": serie,
            "note": "t_s is seconds since the connection started (or the position in a \
                replay). A null value is a failed read.",
        }),
    ))
}

fn lettura_json(l: &Lettura) -> Value {
    json!({
        "status": stato_codice(l.stato),
        "detail": (!l.dettaglio.is_empty()).then_some(l.dettaglio.as_str()),
    })
}

fn stato_dtc_codice(s: StatoDtc) -> &'static str {
    match s {
        StatoDtc::Confermato => "confirmed",
        StatoDtc::InSospeso => "pending",
        StatoDtc::Permanente => "permanent",
    }
}

/// La diagnosi nella forma data all'assistente. Il VIN non c'è: nemmeno
/// l'archivio lo conserva.
fn diagnosi_json(d: &DiagnosiLetta) -> Value {
    let r = &d.referto;
    let dtc: Vec<Value> = r
        .dtc
        .iter()
        .map(|x| {
            json!({
                "code": x.codice,
                "description": x.descrizione,
                "ecu": x.ecu,
                "states": x.stati.iter().map(|s| stato_dtc_codice(*s)).collect::<Vec<_>>(),
                "generic": dtc::e_generico(&x.codice),
            })
        })
        .collect();
    let mil: Vec<Value> = r
        .prontezza
        .iter()
        .map(|p| {
            json!({
                "ecu": p.ecu,
                "mil_on": p.mil_accesa,
                "dtc_count_declared": p.numero_dtc,
            })
        })
        .collect();
    let prontezza: Vec<Value> = r
        .prontezza
        .iter()
        .map(|p| {
            json!({
                "ecu": p.ecu,
                "diesel": p.diesel,
                "monitors": p.monitor.iter().filter(|m| m.disponibile).map(|m| json!({
                    "name": m.nome,
                    "complete": m.completato,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let fermo: Vec<Value> = r
        .fermo
        .righe
        .iter()
        .map(|x| {
            json!({
                "key": x.key,
                "name": x.nome,
                "value": x.valore.filter(|v| v.is_finite() && x.stato == Stato::Ok),
                "text": x.testo,
                "unit": x.unita,
                "status": stato_codice(x.stato),
            })
        })
        .collect();
    let mode06: Vec<Value> = r
        .mode06
        .test
        .iter()
        .map(|t| {
            json!({
                "ecu": t.ecu,
                "mid": format!("{:02X}", t.mid),
                "tid": format!("{:02X}", t.tid),
                "unit_and_scaling_id": format!("{:02X}", t.uas_id),
                "raw": {
                    "value": t.grezzo_valore,
                    "min": t.grezzo_minimo,
                    "max": t.grezzo_massimo,
                },
                "scaled": t.scalato.as_ref().map(|s| json!({
                    "value": s.valore,
                    "min": s.minimo,
                    "max": s.massimo,
                    "unit": s.unita,
                })),
            })
        })
        .collect();
    let pid: Map<String, Value> = r
        .pid_supportati
        .iter()
        .map(|(ecu, lista)| {
            (
                ecu.clone(),
                json!(lista.iter().map(|p| format!("{p:02X}")).collect::<Vec<_>>()),
            )
        })
        .collect();
    con_origine(
        Some(d.origine),
        json!({
            "available": true,
            "read_at": r.letto_il,
            "adapter": r.adattatore,
            "protocol": r.protocollo,
            "ecus": r.ecu,
            "connector_voltage": r.tensione,
            "fuel_type": r.tipo_carburante,
            "obd_standard": r.norma_obd,
            "vin": Value::Null,
            "vin_note": NOTA_VIN,
            "mil": mil,
            "mil_and_readiness_read": lettura_json(&r.lettura_prontezza),
            "dtc_count": dtc.len(),
            "dtcs": dtc,
            "dtc_reads": {
                "confirmed": lettura_json(&r.lettura_confermati),
                "pending": lettura_json(&r.lettura_in_sospeso),
                "permanent": lettura_json(&r.lettura_permanenti),
            },
            "readiness": prontezza,
            "freeze_frame": {
                "present": r.fermo.presente,
                "read": lettura_json(&r.fermo.lettura),
                "dtc": r.fermo.dtc,
                "ecu": (!r.fermo.ecu.is_empty()).then_some(r.fermo.ecu.as_str()),
                "values": fermo,
                "note": "Stored when the fault was detected: not live data.",
            },
            "mode06_catalyst_monitor": {
                "read": lettura_json(&r.mode06.lettura),
                "tests": mode06,
                "note": "Raw values as reported by the ECU; `scaled` is present only when \
                    the unit-and-scaling ID is in the app's table. The app does not judge \
                    whether a test passed.",
            },
            "supported_pids": pid,
            "land_rover_td5": r.td5.as_ref().map(|t| json!({
                "fault_map_read": lettura_json(&t.lettura_guasti),
                "ecu_type": t.tipo_centralina,
                "map_variant": t.variante_mappa,
                "battery_voltage_v": t.tensione_batteria.filter(|v| v.is_finite()),
                "adapter_strategy": t.strategia,
                "note": "Land Rover Td5 ECU, read-only mode never tested on a real adapter or \
                    vehicle. Fault codes are the Td5's own numbering (byte-bit of the fault \
                    map), not OBD-II P-codes; names from the Td5OpenDiag table. Clearing \
                    codes is not available for this ECU, by design.",
            })),
            "note": "This report lists what the ECU reported. It does not establish which \
                component is faulty. Descriptions and monitor names may be in Italian or \
                English.",
        }),
    )
}

fn get_diagnosis(ctx: &Contesto, _a: &Argomenti) -> Esito {
    Ok(ctx.archivio.leggi(|i| match &i.diagnosi {
        Some(d) => {
            let mut v = diagnosi_json(d);
            if let Some(m) = v.as_object_mut() {
                m.insert("connected_now".into(), json!(i.collegamento.is_some()));
            }
            v
        }
        None => con_origine(
            i.origine(),
            json!({
                "available": false,
                "connected_now": i.collegamento.is_some(),
                "message": if i.collegamento.is_some() {
                    "No diagnosis has been read on this connection yet. Ask the user to \
                     press \"Leggi diagnosi\" in OBDio, or call read_diagnosis if actions are \
                     enabled."
                } else {
                    "No diagnosis available: OBDio is not connected and none has been read. \
                     The user must connect the adapter (or the demo) in OBDio first."
                },
            }),
        ),
    }))
}

fn lookup_dtc(_ctx: &Contesto, a: &Argomenti) -> Esito {
    let codice = testo(a, "code")?.trim().to_uppercase();
    let b = codice.as_bytes();
    let valido = b.len() == 5
        && matches!(b[0], b'P' | b'C' | b'B' | b'U')
        && matches!(b[1], b'0'..=b'3')
        && b[2..].iter().all(u8::is_ascii_hexdigit);
    if !valido {
        return Err(format!(
            "Invalid code `{codice}`: expected a letter P, C, B or U, a digit 0-3 and three \
             hexadecimal digits, for example P0420."
        ));
    }
    let d = dtc::descrivi(&codice);
    Ok(json!({
        "origin": "embedded_database",
        "origin_note": "From the DTC list embedded in OBDio, not read from any vehicle.",
        "code": codice,
        "found": d.lingua.is_some(),
        "description": d.testo,
        "description_language": match d.lingua {
            Some(LinguaDescrizione::Italiano) => json!("it"),
            Some(LinguaDescrizione::Inglese) => json!("en"),
            None => Value::Null,
        },
        "kind": if d.generico { "generic" } else { "manufacturer_specific" },
        "generic": d.generico,
        "system": match b[0] {
            b'P' => "powertrain",
            b'C' => "chassis",
            b'B' => "body",
            _ => "network",
        },
        "note": if d.generico {
            "Generic code (SAE J2012): same meaning on every make."
        } else {
            "Manufacturer-specific code: its meaning depends on the make and is not included \
             in the app."
        },
    }))
}

// ---------------------------------------------------------------------------
// Sessioni registrate
// ---------------------------------------------------------------------------

fn nome_sessione(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Durata (dal primo all'ultimo campione) in secondi.
fn durata(s: &SessionData) -> Option<f64> {
    let mut it = s.campioni.iter().map(|c| c.t).filter(|t| t.is_finite());
    let primo = it.next()?;
    let (min, max) = it.fold((primo, primo), |(a, b), t| (a.min(t), b.max(t)));
    Some(max - min)
}

fn intestazione_json(nome: &str, s: &SessionData) -> Value {
    let i = &s.intestazione;
    let chiavi: BTreeSet<&str> = s.campioni.iter().map(|c| c.key.as_str()).collect();
    json!({
        "name": nome,
        "recorded_at": i.creata,
        "recorded_from": origine_codice(i.origine()),
        "imported_from": i.importata.as_ref().map(|x| json!({
            "file": x.file,
            "format": x.formato,
            "imported_at": x.il,
        })),
        "adapter": i.adattatore,
        "protocol": i.protocollo,
        "ecus": i.ecu,
        "duration_s": durata(s).map(|d| arrotonda(d, 1)),
        "samples": s.campioni.len(),
        "parameter_count": chiavi.len(),
        "parameter_keys": chiavi.iter().take(60).collect::<Vec<_>>(),
        "marker_count": s.marcatori.len(),
        "has_diagnosis": s.diagnosi.is_some(),
    })
}

fn list_sessions(ctx: &Contesto, a: &Argomenti) -> Esito {
    let limite = intero(a, "limit", SESSIONI_PREDEFINITE, 1, SESSIONI_MAX)? as usize;
    let scarto = intero(a, "offset", 0, 0, u32::MAX as u64)? as usize;
    let tutte = recording::elenca(&ctx.cartella_sessioni);
    let sessioni: Vec<Value> = tutte
        .iter()
        .skip(scarto)
        .take(limite)
        .map(|p| {
            let nome = nome_sessione(p);
            let byte = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            if byte > SESSIONE_MAX_BYTE {
                return json!({
                    "name": nome,
                    "size_bytes": byte,
                    "unreadable": "too large to summarize over MCP",
                });
            }
            match recording::carica(p) {
                Ok(s) => {
                    let mut v = intestazione_json(&nome, &s);
                    if let Some(m) = v.as_object_mut() {
                        m.insert("size_bytes".into(), json!(byte));
                    }
                    v
                }
                Err(e) => json!({ "name": nome, "size_bytes": byte, "unreadable": e }),
            }
        })
        .collect();
    Ok(json!({
        "origin": "recorded_sessions_on_disk",
        "origin_note": "Sessions stored on disk, not live data. `recorded_from` says whether \
            each one came from a real vehicle or from the demo simulator.",
        "total": tutte.len(),
        "offset": scarto,
        "returned": sessioni.len(),
        "sessions": sessioni,
    }))
}

fn get_session_summary(ctx: &Contesto, a: &Argomenti) -> Esito {
    let nome = testo(a, "name")?.trim();
    // Il nome deve coincidere con un file dell'elenco: nessun percorso viene
    // costruito con ciò che arriva dalla rete.
    let Some(percorso) = recording::elenca(&ctx.cartella_sessioni)
        .into_iter()
        .find(|p| nome_sessione(p) == nome)
    else {
        return Err(format!(
            "No recorded session named `{nome}`. Call list_sessions and use a `name` exactly \
             as returned."
        ));
    };
    let byte = std::fs::metadata(&percorso).map(|m| m.len()).unwrap_or(0);
    if byte > SESSIONE_MAX_BYTE {
        return Err(format!(
            "The session `{nome}` is too large ({byte} bytes) to summarize over MCP."
        ));
    }
    let s = recording::carica(&percorso)
        .map_err(|e| format!("The session `{nome}` could not be read: {e}"))?;

    struct Riga {
        unita: String,
        letture: u64,
        mancanti: u64,
        stat: Statistiche,
    }
    let mut righe: std::collections::BTreeMap<&str, Riga> = Default::default();
    for c in &s.campioni {
        let r = righe.entry(c.key.as_str()).or_insert_with(|| Riga {
            unita: c.unita.clone(),
            letture: 0,
            mancanti: 0,
            stat: Statistiche::default(),
        });
        r.letture += 1;
        match c.valore.filter(|v| v.is_finite() && c.stato == Stato::Ok) {
            Some(v) => r.stat.aggiungi(v),
            None => r.mancanti += 1,
        }
    }
    let totale_parametri = righe.len();
    let parametri: Vec<Value> = righe
        .iter()
        .take(PARAMETRI_MAX)
        .map(|(k, r)| {
            let mut v = json!({
                "key": k,
                "name": recording::nome_parametro(&s, k),
                "unit": r.unita,
                "samples": r.letture,
                "failed_reads": r.mancanti,
            });
            if let (Some(m), Some(st)) = (
                v.as_object_mut(),
                statistiche_json(k, &r.stat).as_object_mut(),
            ) {
                m.append(st);
            }
            v
        })
        .collect();
    let marcatori: Vec<Value> = s
        .marcatori
        .iter()
        .take(MARCATORI_MAX)
        .map(|m| json!({ "t_s": arrotonda(m.t, 2), "label": m.etichetta, "timestamp": m.ts }))
        .collect();
    let diagnosi = s.diagnosi.as_ref().map(|d| {
        json!({
            "read_at": d.letto_il,
            "dtcs": d.dtc.iter().map(|x| json!({
                "code": x.codice,
                "description": x.descrizione,
                "ecu": x.ecu,
                "states": x.stati.iter().map(|s| stato_dtc_codice(*s)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "mil_on": d.prontezza.iter().any(|p| p.mil_accesa),
            "freeze_frame_dtc": d.fermo.dtc,
            "vin_note": NOTA_VIN,
        })
    });
    let mut v = intestazione_json(nome, &s);
    if let Some(m) = v.as_object_mut() {
        m.remove("parameter_keys");
        m.insert("origin".into(), json!("recorded_session_on_disk"));
        m.insert(
            "origin_note".into(),
            json!(match s.intestazione.origine() {
                Origine::Demo =>
                    "Recorded session with SIMULATED data from the demo simulator: not a real \
                     vehicle.",
                _ => "Recorded session: past readings from a vehicle, not current data.",
            }),
        );
        m.insert("size_bytes".into(), json!(byte));
        m.insert("discarded_lines".into(), json!(s.righe_scartate));
        m.insert("parameters".into(), json!(parametri));
        m.insert(
            "parameters_truncated".into(),
            json!(totale_parametri > PARAMETRI_MAX),
        );
        m.insert("markers".into(), json!(marcatori));
        m.insert(
            "markers_truncated".into(),
            json!(s.marcatori.len() > MARCATORI_MAX),
        );
        m.insert("diagnosis".into(), json!(diagnosi));
        m.insert(
            "note".into(),
            json!(
                "Statistics cover the valid readings; failed reads are counted apart and \
                 never treated as zero. t_s is seconds from the start of the recording."
            ),
        );
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// Azioni
// ---------------------------------------------------------------------------

/// Verifica che ci sia un collegamento dal vivo (non una riproduzione).
fn richiedi_dal_vivo(i: &Interno, in_riproduzione: &str) -> Result<(), String> {
    match i.origine() {
        None => Err(
            "OBDio is not connected. The user must connect the adapter (or start the demo) \
             from the Connessione page of the app: connecting is not available over MCP."
                .into(),
        ),
        Some(Origine::Riproduzione) => Err(in_riproduzione.to_string()),
        Some(_) => Ok(()),
    }
}

fn invia(ctx: &Contesto, a: Azione) -> Result<(), String> {
    ctx.azioni
        .send(a)
        .map_err(|_| "OBDio is shutting down: the action was not performed.".to_string())
}

fn errore_app(e: &Option<(String, String)>) -> String {
    match e {
        Some((titolo, messaggio)) => {
            format!("OBDio reported an error (in Italian): {titolo}. {messaggio}")
        }
        None => "OBDio reported an error.".into(),
    }
}

const SCADUTA: &str = "OBDio did not confirm the action in time. Call get_status to see the \
     current state before retrying.";

fn read_diagnosis(ctx: &Contesto, _a: &Argomenti) -> Esito {
    let (n0, e0) = ctx.archivio.leggi(|i| {
        richiedi_dal_vivo(
            i,
            "A recorded session is being replayed: there is no ECU to query. get_diagnosis \
             returns the diagnosis stored in the session, if any.",
        )?;
        if i.stato != StatoConn::Connesso {
            return Err(format!(
                "OBDio is busy ({}). Retry in a few seconds.",
                stato_conn_codice(&i.stato)
            ));
        }
        Ok((i.n_diagnosi, i.n_errori))
    })?;
    invia(ctx, Azione::LeggiDiagnosi)?;
    let esito = ctx.archivio.attendi(ATTESA_DIAGNOSI, &ctx.stop, |i| {
        if i.n_diagnosi > n0 {
            let d = i.diagnosi.as_ref()?;
            let mut v = diagnosi_json(d);
            if let Some(m) = v.as_object_mut() {
                m.insert("connected_now".into(), json!(i.collegamento.is_some()));
                m.insert("freshly_read".into(), json!(true));
            }
            Some(Ok(v))
        } else if i.n_errori > e0 {
            Some(Err(errore_app(&i.errore)))
        } else if i.collegamento.is_none() {
            Some(Err(
                "The connection was lost while reading the diagnosis.".to_string()
            ))
        } else {
            None
        }
    });
    esito.unwrap_or_else(|| Err(SCADUTA.into()))
}

fn start_reading(ctx: &Contesto, a: &Argomenti) -> Esito {
    let richieste = elenco_testi(a, "keys")?
        .filter(|v| !v.is_empty())
        .ok_or("Invalid `keys`: give at least one sensor key from list_sensors.")?;
    let (chiavi, ignorate, e0) = ctx.archivio.leggi(|i| {
        richiedi_dal_vivo(
            i,
            "A recorded session is being replayed: all its sensors already flow by \
             themselves, there is nothing to start.",
        )?;
        let disponibili = i.parametri();
        let mut chiavi: Vec<String> = Vec::new();
        let mut ignorate: Vec<Value> = Vec::new();
        for k in &richieste {
            match disponibili.iter().find(|p| p.key == *k) {
                Some(p) if p.derivato => {
                    // Un calcolato si ottiene leggendo la sua sorgente.
                    if let Some(s) = pids::sorgente_derivato(&p.key)
                        && !chiavi.iter().any(|x| x == s)
                    {
                        chiavi.push(s.to_string());
                    }
                    if !chiavi.contains(k) {
                        chiavi.push(k.clone());
                    }
                }
                Some(_) if !chiavi.contains(k) => chiavi.push(k.clone()),
                Some(_) => {}
                None => ignorate.push(json!({
                    "key": k,
                    "reason": "unknown, or not supported by the connected ECU",
                })),
            }
        }
        Ok::<_, String>((chiavi, ignorate, i.n_errori))
    })?;
    if chiavi.is_empty() {
        return Err(
            "None of the requested keys is available on this connection, so nothing was \
             started. Call list_sensors for the valid keys."
                .into(),
        );
    }
    let attese: BTreeSet<String> = chiavi.iter().cloned().collect();
    invia(ctx, Azione::AvviaLettura(chiavi.clone()))?;
    let esito = ctx.archivio.attendi(ATTESA_BREVE, &ctx.stop, |i| {
        if i.lettura && i.chiavi_lettura == attese {
            Some(Ok(i.origine()))
        } else if i.n_errori > e0 {
            Some(Err(errore_app(&i.errore)))
        } else if i.collegamento.is_none() {
            Some(Err("The connection was lost.".to_string()))
        } else {
            None
        }
    });
    let origine = esito.unwrap_or_else(|| Err(SCADUTA.into()))?;
    Ok(con_origine(
        origine,
        json!({
            "reading_active": true,
            "keys": chiavi,
            "ignored_keys": ignorate,
            "note": "Values arrive over the next seconds: call get_live_values. The person \
                using the app can change or stop the selection from its pages.",
        }),
    ))
}

fn stop_reading(ctx: &Contesto, _a: &Argomenti) -> Esito {
    let attiva = ctx.archivio.leggi(|i| {
        richiedi_dal_vivo(
            i,
            "A recorded session is being replayed: only the user can pause or close it, from \
             the transport bar of the app.",
        )?;
        Ok::<_, String>(i.lettura)
    })?;
    if attiva {
        invia(ctx, Azione::FermaLettura)?;
        ctx.archivio
            .attendi(ATTESA_BREVE, &ctx.stop, |i| (!i.lettura).then_some(()))
            .ok_or(SCADUTA)?;
    }
    Ok(con_origine(
        ctx.archivio.leggi(|i| i.origine()),
        json!({
            "reading_active": false,
            "was_active": attiva,
        }),
    ))
}

fn add_marker(ctx: &Contesto, a: &Argomenti) -> Esito {
    let etichetta = testo(a, "text")?.trim().to_string();
    let lunghezza = etichetta.chars().count();
    if lunghezza == 0 || lunghezza > LUNGHEZZA_MARCATORE {
        return Err(format!(
            "Invalid `text`: the label must have 1 to {LUNGHEZZA_MARCATORE} characters (got \
             {lunghezza})."
        ));
    }
    if etichetta.chars().any(char::is_control) {
        return Err("Invalid `text`: the label must be a single line of plain text.".into());
    }
    let (n0, e0) = ctx.archivio.leggi(|i| {
        richiedi_dal_vivo(
            i,
            "A recorded session is being replayed: markers cannot be added to it.",
        )?;
        Ok::<_, String>((i.n_marcatori, i.n_errori))
    })?;
    invia(ctx, Azione::Marcatore(etichetta.clone()))?;
    let esito = ctx.archivio.attendi(ATTESA_BREVE, &ctx.stop, |i| {
        if i.n_marcatori > n0 {
            let m = i.marcatori.iter().rev().find(|m| m.etichetta == etichetta);
            Some(Ok(con_origine(
                i.origine(),
                json!({
                    "added": true,
                    "label": etichetta,
                    "t_s": m.map(|m| arrotonda(m.t, 2)),
                    "timestamp": m.map(|m| m.ts.as_str()),
                    "saved_in_recording": i.registrazione.is_some(),
                    "note": if i.registrazione.is_some() {
                        "Saved in the session being recorded."
                    } else {
                        "No recording is active: the marker is shown on the live graphs but \
                         not saved to disk."
                    },
                }),
            )))
        } else if i.n_errori > e0 {
            Some(Err(errore_app(&i.errore)))
        } else {
            None
        }
    });
    esito.unwrap_or_else(|| Err(SCADUTA.into()))
}

fn start_recording(ctx: &Contesto, _a: &Argomenti) -> Esito {
    let (gia, e0) = ctx.archivio.leggi(|i| {
        richiedi_dal_vivo(
            i,
            "A recorded session is being replayed: it is already recorded, there is nothing \
             to record.",
        )?;
        Ok::<_, String>((i.registrazione.is_some(), i.n_errori))
    })?;
    if !gia {
        invia(ctx, Azione::AvviaRegistrazione)?;
    }
    let esito = ctx.archivio.attendi(ATTESA_BREVE, &ctx.stop, |i| {
        if let Some(r) = &i.registrazione {
            Some(Ok(con_origine(
                i.origine(),
                json!({
                    "recording_active": true,
                    "already_active": gia,
                    "file": r.file,
                    "reading_active": i.lettura,
                    "note": if i.lettura {
                        "The sensors being read are now recorded."
                    } else {
                        "Recording started, but no sensor is being read: call start_reading, \
                         otherwise the session stays empty."
                    },
                }),
            )))
        } else if i.n_errori > e0 {
            Some(Err(errore_app(&i.errore)))
        } else if i.collegamento.is_none() {
            Some(Err("The connection was lost.".to_string()))
        } else {
            None
        }
    });
    esito.unwrap_or_else(|| Err(SCADUTA.into()))
}

fn stop_recording(ctx: &Contesto, _a: &Argomenti) -> Esito {
    let (attiva, n0, e0, origine) = ctx.archivio.leggi(|i| {
        (
            i.registrazione.is_some(),
            i.n_registrazioni_chiuse,
            i.n_errori,
            i.origine(),
        )
    });
    if !attiva {
        return Ok(con_origine(
            origine,
            json!({
                "recording_active": false,
                "was_active": false,
                "note": "No recording was in progress.",
            }),
        ));
    }
    invia(ctx, Azione::FermaRegistrazione)?;
    let esito = ctx
        .archivio
        .attendi(ATTESA_CHIUSURA_REGISTRAZIONE, &ctx.stop, |i| {
            if i.n_registrazioni_chiuse > n0 {
                let (file, campioni) = i.ultima_registrazione.clone().unwrap_or_default();
                Some(Ok(con_origine(
                    origine,
                    json!({
                        "recording_active": false,
                        "was_active": true,
                        "file": file,
                        "samples": campioni,
                        "note": "The session is now listed by list_sessions.",
                    }),
                )))
            } else if i.n_errori > e0 {
                Some(Err(errore_app(&i.errore)))
            } else {
                None
            }
        });
    esito.unwrap_or_else(|| Err(SCADUTA.into()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mcp::archivio::prova;
    use crate::model::Sample;
    use crate::worker::Evento;
    use std::sync::mpsc::{self, Receiver};

    pub(crate) fn contesto(archivio: Archivio, livello: Livello) -> (Contesto, Receiver<Azione>) {
        let (tx, rx) = mpsc::channel();
        (
            Contesto {
                archivio: Arc::new(archivio),
                azioni: tx,
                livello,
                cartella_sessioni: std::env::temp_dir().join("obdio_mcp_nessuna_sessione"),
                stop: Arc::new(AtomicBool::new(false)),
                azione_in_corso: Mutex::new(()),
            },
            rx,
        )
    }

    fn ok(ctx: &Contesto, nome: &str, argomenti: Value) -> Value {
        match chiama(ctx, nome, argomenti.as_object().unwrap()) {
            Chiamata::Risultato(v) => v,
            altro => panic!("{nome}: atteso un risultato, avuto {altro:?}"),
        }
    }

    fn errore(ctx: &Contesto, nome: &str, argomenti: Value) -> String {
        match chiama(ctx, nome, argomenti.as_object().unwrap()) {
            Chiamata::Errore(e) => e,
            altro => panic!("{nome}: atteso un errore, avuto {altro:?}"),
        }
    }

    #[test]
    fn elenco_secondo_il_livello_e_annotazioni_veritiere() {
        let lettura = elenco(Livello::SolaLettura);
        let tutto = elenco(Livello::LetturaEAzioni);
        let nomi_di = |v: &[Value]| -> Vec<String> {
            v.iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(
            nomi_di(&lettura),
            [
                "get_status",
                "list_sensors",
                "get_live_values",
                "get_sensor_history",
                "get_diagnosis",
                "lookup_dtc",
                "list_sessions",
                "get_session_summary"
            ]
        );
        assert_eq!(
            nomi_di(&tutto)[8..],
            [
                "read_diagnosis",
                "start_reading",
                "stop_reading",
                "add_marker",
                "start_recording",
                "stop_recording"
            ]
        );
        assert_eq!(tutto.len(), nomi().len());
        for t in &tutto {
            let nome = t["name"].as_str().unwrap();
            // Nomi ammessi dalla specifica: lettere, cifre, «_», «-», «.».
            assert!(
                nome.len() <= 128
                    && nome
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
            );
            assert_eq!(t["inputSchema"]["type"], "object", "{nome}");
            assert_eq!(t["inputSchema"]["additionalProperties"], false, "{nome}");
            assert!(t["description"].as_str().unwrap().len() > 40, "{nome}");
            let a = &t["annotations"];
            assert_eq!(a["readOnlyHint"], !e_azione(nome), "{nome}");
            // Nessuno strumento distrugge qualcosa o esce dal PC.
            assert_eq!(a["destructiveHint"], false, "{nome}");
            assert_eq!(a["openWorldHint"], false, "{nome}");
            if e_azione(nome) {
                assert!(
                    t["description"].as_str().unwrap().starts_with("ACTION."),
                    "{nome}"
                );
            }
        }
    }

    #[test]
    fn nessuno_strumento_cancella_i_codici() {
        for n in nomi() {
            for vietato in ["clear", "erase", "delete", "reset", "cancel"] {
                assert!(!n.contains(vietato), "{n}");
            }
        }
        let (ctx, rx) = contesto(prova::popolato(), Livello::LetturaEAzioni);
        for n in [
            "clear_dtc",
            "clear_codes",
            "erase_codes",
            "erase_dtc",
            "reset_mil",
            "cancella_dtc",
            "CancellaDtc",
        ] {
            assert_eq!(chiama(&ctx, n, &Map::new()), Chiamata::Sconosciuto, "{n}");
        }
        assert!(rx.try_recv().is_err());
        // E lo dice a chi chiede lo stato.
        let s = ok(&ctx, "get_status", json!({}));
        assert_eq!(s["erase_codes_over_mcp"], "never_available");
    }

    #[test]
    fn stato_su_archivio_vuoto_e_popolato() {
        let (vuoto, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        let s = ok(&vuoto, "get_status", json!({}));
        assert_eq!(s["connection"], "disconnected");
        assert_eq!(s["connected"], false);
        assert!(s["origin"].is_null());
        assert!(s["adapter"].is_null());
        assert_eq!(s["reading_active"], false);
        assert_eq!(s["recording"]["active"], false);
        assert!(s["replay"].is_null());
        assert_eq!(s["mcp_permission_level"], "read_only");

        let (pieno, _rx) = contesto(prova::popolato(), Livello::LetturaEAzioni);
        let s = ok(&pieno, "get_status", json!({}));
        assert_eq!(s["connection"], "connected");
        assert_eq!(s["origin"], "demo_simulator");
        assert!(s["origin_note"].as_str().unwrap().contains("SIMULATED"));
        assert_eq!(s["ecus"][0], "7E8");
        assert_eq!(s["reading_active"], true);
        assert!(
            s["reading_keys"]
                .as_array()
                .unwrap()
                .contains(&json!("giri"))
        );
        assert_eq!(s["diagnosis_available"], true);
        assert_eq!(s["mcp_permission_level"], "read_and_actions");
    }

    #[test]
    fn sensori_e_valori() {
        let (vuoto, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        let l = ok(&vuoto, "list_sensors", json!({}));
        assert_eq!(l["count"], 0);
        assert_eq!(l["connected"], false);
        let v = ok(&vuoto, "get_live_values", json!({}));
        assert!(v["values"].as_object().unwrap().is_empty());
        assert!(v["origin"].is_null());

        let (ctx, _rx) = contesto(prova::popolato(), Livello::SolaLettura);
        let l = ok(&ctx, "list_sensors", json!({}));
        let giri = l["sensors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["key"] == "giri")
            .expect("giri");
        assert_eq!(giri["unit"], "giri/min");
        assert_eq!(giri["group"], "Motore");
        assert_eq!(giri["pid"], "0C");
        assert_eq!(giri["being_read"], true);
        let refrigerante = l["sensors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["key"] == "temp_refrigerante")
            .expect("temp_refrigerante");
        assert_eq!(refrigerante["being_read"], false);
        assert_eq!(refrigerante["has_data"], false);

        let v = ok(&ctx, "get_live_values", json!({}));
        assert_eq!(v["origin"], "demo_simulator");
        let g = &v["values"]["giri"];
        assert_eq!(g["value"], 890.0);
        assert_eq!(g["status"], "ok");
        assert_eq!(g["unit"], "giri/min");
        assert_eq!(g["ecu"], "7E8");
        assert_eq!(g["min"], 800.0);
        assert_eq!(g["max"], 890.0);
        assert_eq!(g["mean"], 845.0);
        assert!(g["age_s"].as_f64().unwrap() < 5.0);
        // Una velocità nulla letta davvero è zero; una lettura fallita è null.
        assert_eq!(v["values"]["velocita"]["value"], 0.0);
        let sonda = &v["values"]["o2s2_tensione"];
        assert!(sonda["value"].is_null(), "{sonda}");
        assert_eq!(sonda["status"], "no_response");
        assert_eq!(sonda["failed_reads"], 1);
        assert_eq!(sonda["min"], 0.7);

        // Filtro: chiavi senza dati e chiavi sconosciute sono distinte.
        let f = ok(
            &ctx,
            "get_live_values",
            json!({ "keys": ["giri", "temp_refrigerante", "inesistente"] }),
        );
        assert_eq!(f["values"].as_object().unwrap().len(), 1);
        assert_eq!(f["keys_without_data_yet"], json!(["temp_refrigerante"]));
        assert_eq!(f["unknown_keys"], json!(["inesistente"]));
        assert!(errore(&ctx, "get_live_values", json!({ "keys": "giri" })).contains("array"));
    }

    #[test]
    fn storia_limitata_e_ridotta() {
        let a = Archivio::default();
        a.osserva(&Evento::Connesso(Box::new(prova::info_demo())));
        let mut v: Vec<Sample> = (0..1000)
            .map(|i| prova::campione("giri", i as f64 * 0.1, Some(1000.0 + i as f64), Stato::Ok))
            .collect();
        v.push(prova::campione(
            "giri",
            100.0,
            None,
            Stato::ErroreComunicazione,
        ));
        a.osserva(&Evento::Campioni(v));
        let (ctx, _rx) = contesto(a, Livello::SolaLettura);

        // Finestra breve: tutti i campioni grezzi, il mancante è null.
        let breve = ok(
            &ctx,
            "get_sensor_history",
            json!({ "key": "giri", "seconds": 5 }),
        );
        assert_eq!(breve["downsampled"], false);
        let punti = breve["points"].as_array().unwrap();
        assert_eq!(punti.len(), breve["raw_samples_in_window"]);
        assert!(punti.len() <= 52, "{}", punti.len());
        let ultimo = punti.last().unwrap();
        assert!(ultimo["value"].is_null());
        assert_eq!(ultimo["status"], "communication_error");
        assert_eq!(breve["window_statistics"]["failed_reads"], 1);

        // Finestra lunga: ridotta, e lo dice.
        let lunga = ok(
            &ctx,
            "get_sensor_history",
            json!({ "key": "giri", "seconds": 3600, "max_points": 50 }),
        );
        assert_eq!(lunga["downsampled"], true);
        assert_eq!(lunga["raw_samples_in_window"], 1001);
        let secchi = lunga["points"].as_array().unwrap();
        assert!(secchi.len() <= 50 && secchi.len() >= 40, "{}", secchi.len());
        assert!(lunga["downsampling"].as_str().unwrap().contains("1001"));
        let totale: u64 = secchi.iter().map(|s| s["samples"].as_u64().unwrap()).sum();
        assert_eq!(totale, 1001);
        let mancanti: u64 = secchi
            .iter()
            .map(|s| s["failed_reads"].as_u64().unwrap())
            .sum();
        assert_eq!(mancanti, 1);
        assert_eq!(secchi[0]["min"], 1000.0);
        assert_eq!(lunga["window_statistics"]["max"], 1999.0);

        // Limiti degli argomenti.
        for a in [
            json!({ "key": "giri", "seconds": 0 }),
            json!({ "key": "giri", "seconds": 999_999 }),
            json!({ "key": "giri", "max_points": 5000 }),
            json!({ "key": "giri", "max_points": -3 }),
            json!({ "key": "giri", "altro": 1 }),
            json!({}),
        ] {
            errore(&ctx, "get_sensor_history", a);
        }
        assert!(
            errore(&ctx, "get_sensor_history", json!({ "key": "velocita" }))
                .contains("No samples yet")
        );
        assert!(
            errore(&ctx, "get_sensor_history", json!({ "key": "boh" })).contains("list_sensors")
        );
    }

    #[test]
    fn un_secchio_senza_valori_validi_e_null() {
        let punti: Vec<Punto> = (0..40)
            .map(|i| Punto {
                t: i as f64,
                v: (i < 20).then_some(5.0),
                stato: if i < 20 {
                    Stato::Ok
                } else {
                    Stato::RispostaMancante
                },
            })
            .collect();
        let secchi = riduci(&punti, 10);
        assert_eq!(secchi.len(), 10);
        assert_eq!(secchi[0]["value"], 5.0);
        let ultimo = secchi.last().unwrap();
        assert!(ultimo["value"].is_null(), "{ultimo}");
        assert!(ultimo["min"].is_null());
        assert_eq!(ultimo["failed_reads"], ultimo["samples"]);
        assert!(riduci(&[], 10).is_empty());
        // Tutti i campioni allo stesso istante: un solo secchio.
        assert_eq!(riduci(&punti[..1].repeat(30), 10).len(), 1);
    }

    #[test]
    fn diagnosi_senza_vin() {
        let (vuoto, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        let d = ok(&vuoto, "get_diagnosis", json!({}));
        assert_eq!(d["available"], false);
        assert!(d["message"].as_str().unwrap().contains("not connected"));

        let (ctx, _rx) = contesto(prova::popolato(), Livello::SolaLettura);
        let d = ok(&ctx, "get_diagnosis", json!({}));
        assert_eq!(d["available"], true);
        assert_eq!(d["origin"], "demo_simulator");
        assert_eq!(d["dtc_count"], 1);
        assert_eq!(d["dtcs"][0]["code"], "P0420");
        assert_eq!(d["dtcs"][0]["states"], json!(["confirmed", "permanent"]));
        assert_eq!(d["dtcs"][0]["generic"], true);
        assert_eq!(d["read_at"], "2026-01-02T03:04:05.000+01:00");
        assert!(d["vin"].is_null());
        assert_eq!(d["freeze_frame"]["present"], false);
        // Il referto di prova porta il VIN della demo: non deve uscire.
        assert!(!d.to_string().contains("DEMO0000000000000"));
        for nome in nomi() {
            if e_azione(nome) || nome.contains("session") {
                continue;
            }
            let argomenti = match nome {
                "get_sensor_history" => json!({ "key": "giri" }),
                "lookup_dtc" => json!({ "code": "P0420" }),
                _ => json!({}),
            };
            let testo = ok(&ctx, nome, argomenti).to_string();
            assert!(!testo.contains("DEMO0000000000000"), "{nome}");
            assert!(testo.contains("\"origin\""), "{nome}");
        }
    }

    #[test]
    fn codici_guasto_dal_database() {
        let (ctx, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        let p = ok(&ctx, "lookup_dtc", json!({ "code": " p0420 " }));
        assert_eq!(p["code"], "P0420");
        assert_eq!(p["found"], true);
        assert_eq!(p["kind"], "generic");
        assert_eq!(p["system"], "powertrain");
        assert!(p["description"].as_str().unwrap().len() > 5);
        let c = ok(&ctx, "lookup_dtc", json!({ "code": "P1234" }));
        assert_eq!(c["kind"], "manufacturer_specific");
        assert_eq!(c["found"], false);
        assert!(c["description_language"].is_null());
        for brutto in ["", "P04", "X0420", "P9420", "P04ZZ", "P04200"] {
            assert!(
                errore(&ctx, "lookup_dtc", json!({ "code": brutto })).contains("Invalid code"),
                "{brutto}"
            );
        }
        assert!(errore(&ctx, "lookup_dtc", json!({})).contains("Missing required"));
        assert!(errore(&ctx, "lookup_dtc", json!({ "code": 420 })).contains("string"));
    }

    #[test]
    fn azioni_rifiutate_in_sola_lettura() {
        let (ctx, rx) = contesto(prova::popolato(), Livello::SolaLettura);
        for nome in nomi().into_iter().filter(|n| e_azione(n)) {
            let e = errore(&ctx, nome, json!({}));
            assert!(e.contains("read-only"), "{nome}: {e}");
        }
        // Nulla è arrivato al thread di lavoro.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn azioni_validano_prima_di_agire() {
        // Scollegato: nessuna azione parte.
        let (ctx, rx) = contesto(Archivio::default(), Livello::LetturaEAzioni);
        for (nome, argomenti) in [
            ("read_diagnosis", json!({})),
            ("start_reading", json!({ "keys": ["giri"] })),
            ("stop_reading", json!({})),
            ("add_marker", json!({ "text": "x" })),
            ("start_recording", json!({})),
        ] {
            let e = errore(&ctx, nome, argomenti);
            assert!(e.contains("not connected"), "{nome}: {e}");
        }
        let fermo = ok(&ctx, "stop_recording", json!({}));
        assert_eq!(fermo["was_active"], false);
        assert!(rx.try_recv().is_err());

        // Collegato: argomenti sbagliati non partono.
        let (ctx, rx) = contesto(prova::popolato(), Livello::LetturaEAzioni);
        assert!(errore(&ctx, "start_reading", json!({ "keys": [] })).contains("at least one"));
        assert!(
            errore(
                &ctx,
                "start_reading",
                json!({ "keys": ["temp_olio", "boh"] })
            )
            .contains("None of the requested keys")
        );
        assert!(errore(&ctx, "add_marker", json!({ "text": "   " })).contains("1 to 80"));
        assert!(errore(&ctx, "add_marker", json!({ "text": "x".repeat(81) })).contains("1 to 80"));
        assert!(errore(&ctx, "add_marker", json!({ "text": "a\nb" })).contains("single line"));
        assert!(errore(&ctx, "stop_reading", json!({ "forza": true })).contains("no arguments"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn azioni_inviate_e_confermate_dall_archivio() {
        let (ctx, rx) = contesto(prova::popolato(), Livello::LetturaEAzioni);
        let ctx = Arc::new(ctx);
        // Un finto thread di lavoro: riceve l'azione e aggiorna l'archivio.
        let archivio = ctx.archivio.clone();
        let lavoro = std::thread::spawn(move || {
            let mut viste = Vec::new();
            while let Ok(a) = rx.recv_timeout(Duration::from_secs(5)) {
                match &a {
                    Azione::AvviaLettura(chiavi) => {
                        archivio.chiavi_lettura(chiavi, &[]);
                        archivio.osserva(&Evento::Polling(true));
                    }
                    Azione::FermaLettura => archivio.osserva(&Evento::Polling(false)),
                    Azione::Marcatore(t) => {
                        archivio.osserva(&Evento::Marcatore(crate::model::Marker {
                            t: 12.0,
                            ts: "2026-01-01T00:00:00+01:00".into(),
                            etichetta: t.clone(),
                        }))
                    }
                    Azione::LeggiDiagnosi => {
                        archivio.osserva(&Evento::Diagnosi(Box::new(prova::referto_demo())));
                    }
                    Azione::AvviaRegistrazione => {
                        archivio.osserva(&Evento::RegistrazioneAvviata {
                            percorso: PathBuf::from("C:/dati/utente/sessione_1.obdio.jsonl"),
                            t_inizio: 12.0,
                        });
                    }
                    Azione::FermaRegistrazione => {
                        archivio.osserva(&Evento::RegistrazioneFermata {
                            percorso: PathBuf::from("C:/dati/utente/sessione_1.obdio.jsonl"),
                            csv: None,
                            campioni: 33,
                        });
                    }
                }
                let fine = a == Azione::FermaRegistrazione;
                viste.push(a);
                if fine {
                    break;
                }
            }
            viste
        });

        let r = ok(
            &ctx,
            "start_reading",
            json!({ "keys": ["giri", "distanza_parziale", "boh", "giri"] }),
        );
        // La distanza parziale porta con sé la velocità da cui si calcola.
        assert_eq!(r["keys"], json!(["giri", "velocita", "distanza_parziale"]));
        assert_eq!(r["ignored_keys"][0]["key"], "boh");
        assert_eq!(r["origin"], "demo_simulator");
        let m = ok(&ctx, "add_marker", json!({ "text": " 2500 giri " }));
        assert_eq!(m["label"], "2500 giri");
        assert_eq!(m["t_s"], 12.0);
        assert_eq!(m["saved_in_recording"], false);
        let d = ok(&ctx, "read_diagnosis", json!({}));
        assert_eq!(d["freshly_read"], true);
        assert_eq!(d["dtcs"][0]["code"], "P0420");
        assert!(!d.to_string().contains("DEMO0000000000000"));
        let reg = ok(&ctx, "start_recording", json!({}));
        // Solo il nome del file: la cartella (con il nome utente) non esce.
        assert_eq!(reg["file"], "sessione_1.obdio.jsonl");
        assert_eq!(reg["already_active"], false);
        assert!(!reg.to_string().contains("utente"));
        let s = ok(&ctx, "stop_reading", json!({}));
        assert_eq!(s["was_active"], true);
        let fine = ok(&ctx, "stop_recording", json!({}));
        assert_eq!(fine["samples"], 33);
        assert_eq!(fine["file"], "sessione_1.obdio.jsonl");

        let viste = lavoro.join().unwrap();
        assert_eq!(
            viste,
            vec![
                Azione::AvviaLettura(vec![
                    "giri".into(),
                    "velocita".into(),
                    "distanza_parziale".into()
                ]),
                Azione::Marcatore("2500 giri".into()),
                Azione::LeggiDiagnosi,
                Azione::AvviaRegistrazione,
                Azione::FermaLettura,
                Azione::FermaRegistrazione,
            ]
        );
    }

    #[test]
    fn sessioni_su_disco() {
        use crate::recording::{Intestazione, Recorder};
        let dir = std::env::temp_dir().join(format!("obdio_mcp_sessioni_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (mut ctx, _rx) = contesto(Archivio::default(), Livello::SolaLettura);
        ctx.cartella_sessioni = dir.clone();

        let vuoto = ok(&ctx, "list_sessions", json!({}));
        assert_eq!(vuoto["total"], 0);
        assert!(vuoto["sessions"].as_array().unwrap().is_empty());

        let intestazione = Intestazione {
            versione: 1,
            creata: "2026-03-04T10:00:00.000+01:00".into(),
            demo: true,
            adattatore: "ELM327 v1.5 (demo)".into(),
            ecu: vec!["7E8".into()],
            ..Default::default()
        };
        let mut r =
            Recorder::avvia(&dir, &intestazione, Some(&prova::referto_demo()), 0.0).unwrap();
        for i in 0..20 {
            let t = i as f64 * 0.5;
            r.campione(&prova::campione(
                "giri",
                t,
                Some(900.0 + i as f64),
                Stato::Ok,
            ))
            .unwrap();
            let muta = i >= 15;
            r.campione(&prova::campione(
                "o2s2_tensione",
                t,
                (!muta).then_some(0.6),
                if muta {
                    Stato::RispostaMancante
                } else {
                    Stato::Ok
                },
            ))
            .unwrap();
        }
        r.marcatore(&crate::model::Marker {
            t: 3.0,
            ts: "2026-03-04T10:00:03.000+01:00".into(),
            etichetta: "Minimo".into(),
        })
        .unwrap();
        let percorso = r.chiudi().unwrap();
        let nome = percorso.file_name().unwrap().to_string_lossy().to_string();

        let elenco = ok(&ctx, "list_sessions", json!({ "limit": 5 }));
        assert_eq!(elenco["total"], 1);
        let s = &elenco["sessions"][0];
        assert_eq!(s["name"], nome.as_str());
        assert_eq!(s["recorded_from"], "demo_simulator");
        assert_eq!(s["samples"], 40);
        assert_eq!(s["duration_s"], 9.5);
        assert_eq!(s["marker_count"], 1);
        assert_eq!(s["has_diagnosis"], true);
        assert_eq!(s["parameter_keys"], json!(["giri", "o2s2_tensione"]));
        assert_eq!(
            ok(&ctx, "list_sessions", json!({ "offset": 1 }))["returned"],
            0
        );

        let r = ok(&ctx, "get_session_summary", json!({ "name": nome }));
        assert_eq!(r["origin"], "recorded_session_on_disk");
        assert!(r["origin_note"].as_str().unwrap().contains("SIMULATED"));
        let parametri = r["parameters"].as_array().unwrap();
        assert_eq!(parametri.len(), 2);
        let giri = &parametri[0];
        assert_eq!(giri["key"], "giri");
        assert_eq!(giri["samples"], 20);
        assert_eq!(giri["min"], 900.0);
        assert_eq!(giri["max"], 919.0);
        assert_eq!(giri["mean"], 909.5);
        let sonda = &parametri[1];
        assert_eq!(sonda["failed_reads"], 5);
        assert_eq!(sonda["valid_samples"], 15);
        // Le letture fallite non abbassano il minimo a zero.
        assert_eq!(sonda["min"], 0.6);
        assert_eq!(r["markers"][0]["label"], "Minimo");
        assert_eq!(r["diagnosis"]["dtcs"][0]["code"], "P0420");
        assert!(!r.to_string().contains("DEMO0000000000000"));

        // Solo i nomi dell'elenco: niente percorsi.
        for n in [
            "../segreto.obdio.jsonl",
            "C:\\Windows\\win.ini",
            "inesistente.obdio.jsonl",
            "",
        ] {
            assert!(
                errore(&ctx, "get_session_summary", json!({ "name": n }))
                    .contains("No recorded session"),
                "{n}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
