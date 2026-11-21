//! Pubblicazione in tempo reale su un broker MQTT (3.1.1, TCP senza cifratura).
//!
//! Tutto ciò che esce da qui passa da una coda che **non blocca mai** il
//! chiamante (il thread di lavoro che fa il polling): se la coda è piena o la
//! connessione è assente, i messaggi dei sensori vengono scartati e contati;
//! stato, diagnosi e configurazioni di discovery sono trattenuti in memoria e
//! rinviati a ogni riconnessione.
//!
//! # Argomenti
//!
//! Con prefisso `obdio` e veicolo `auto` (entrambi configurabili):
//!
//! | Argomento | Contenuto |
//! |---|---|
//! | `obdio/auto/stato` | Trattenuto. `{"stato":"online","origine":"veicolo"\|"demo_sintetico","app":"OBDio","versione":"…"}` con un collegamento attivo, altrimenti `{"stato":"offline"}`. L'ultima volontà del client è `{"stato":"offline"}`; alla chiusura pulita lo stesso messaggio è pubblicato esplicitamente. |
//! | `obdio/auto/sensori/<chiave>` | Il solo numero, in testo con il punto decimale. Non trattenuto. Solo letture valide, al massimo una ogni `intervallo_sensore_ms` per chiave. |
//! | `obdio/auto/istantanea` | JSON con l'ultimo stato di ogni parametro, al massimo ogni `intervallo_istantanea_ms`: `{"ts":…,"origine":…,"valori":{chiave:{"valore":numero\|null,"unita":…,"stato":…,"ecu":…}}}`. Un valore assente è `null`, mai zero. |
//! | `obdio/auto/diagnosi` | Trattenuto. La diagnosi letta (senza VIN) a ogni lettura. |
//! | `obdio/auto/evento` | Non trattenuto. Marcatori e cancellazione dei codici guasto. |
//!
//! Discovery per Home Assistant (facoltativa): una configurazione trattenuta
//! per sensore in `<prefisso_discovery>/sensor/obdio_<veicolo>_<chiave>/config`.
//!
//! Il VIN non viene mai pubblicato.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config;
use crate::diagnosis::DiagnosisReport;
use crate::elm::{EsitoCancellazione, RispostaCancellazione};
use crate::model::{Marker, Sample, Stato, adesso_iso};
use crate::obd::pids;

/// Nome del file di configurazione nella cartella di configurazione.
pub const FILE_CONFIGURAZIONE: &str = "mqtt.json";

/// Posti nella coda verso il broker. Oltre, i messaggi dei sensori si scartano.
const CAPACITA_CODA: usize = 256;
/// Dimensione massima di un pacchetto (la diagnosi completa supera i 10 KB
/// predefiniti della libreria).
const PACCHETTO_MAX: usize = 256 * 1024;
const ATTESA_MIN: Duration = Duration::from_secs(1);
const ATTESA_MAX: Duration = Duration::from_secs(30);
/// Tempo massimo di attesa del thread di rete alla chiusura.
const ATTESA_CHIUSURA: Duration = Duration::from_secs(2);
const INTERVALLO_MAX_MS: u64 = 3_600_000;

// ---------------------------------------------------------------------------
// Configurazione
// ---------------------------------------------------------------------------

/// Impostazioni della pubblicazione. La password finisce su disco solo se
/// `ricorda_password` è vero (e in chiaro).
#[derive(Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct MqttConfig {
    pub attivo: bool,
    pub host: String,
    pub porta: u16,
    pub utente: String,
    pub password: String,
    pub ricorda_password: bool,
    /// Vuoto: ne viene generato uno a ogni avvio.
    pub id_client: String,
    pub prefisso: String,
    pub veicolo: String,
    /// 0 o 1.
    pub qos: u8,
    /// Tempo minimo fra due pubblicazioni dello stesso sensore.
    pub intervallo_sensore_ms: u64,
    pub intervallo_istantanea_ms: u64,
    pub discovery_home_assistant: bool,
    pub prefisso_discovery: String,
}

impl Default for MqttConfig {
    fn default() -> Self {
        MqttConfig {
            attivo: false,
            host: String::new(),
            porta: 1883,
            utente: String::new(),
            password: String::new(),
            ricorda_password: false,
            id_client: String::new(),
            prefisso: "obdio".into(),
            veicolo: "veicolo".into(),
            qos: 0,
            intervallo_sensore_ms: 500,
            intervallo_istantanea_ms: 1000,
            discovery_home_assistant: false,
            prefisso_discovery: "homeassistant".into(),
        }
    }
}

/// Specchio di [`MqttConfig`] per la scrittura su disco.
#[derive(Serialize)]
struct ConfigSuDisco<'a> {
    attivo: bool,
    host: &'a str,
    porta: u16,
    utente: &'a str,
    /// Assente se l'utente non ha scelto di ricordarla.
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
    ricorda_password: bool,
    id_client: &'a str,
    prefisso: &'a str,
    veicolo: &'a str,
    qos: u8,
    intervallo_sensore_ms: u64,
    intervallo_istantanea_ms: u64,
    discovery_home_assistant: bool,
    prefisso_discovery: &'a str,
}

impl Serialize for MqttConfig {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ConfigSuDisco {
            attivo: self.attivo,
            host: &self.host,
            porta: self.porta,
            utente: &self.utente,
            password: self.ricorda_password.then_some(self.password.as_str()),
            ricorda_password: self.ricorda_password,
            id_client: &self.id_client,
            prefisso: &self.prefisso,
            veicolo: &self.veicolo,
            qos: self.qos,
            intervallo_sensore_ms: self.intervallo_sensore_ms,
            intervallo_istantanea_ms: self.intervallo_istantanea_ms,
            discovery_home_assistant: self.discovery_home_assistant,
            prefisso_discovery: &self.prefisso_discovery,
        }
        .serialize(s)
    }
}

impl fmt::Debug for MqttConfig {
    /// La password non compare mai nei log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttConfig")
            .field("attivo", &self.attivo)
            .field("host", &self.host)
            .field("porta", &self.porta)
            .field("utente", &self.utente)
            .field(
                "password",
                &if self.password.is_empty() {
                    ""
                } else {
                    "********"
                },
            )
            .field("ricorda_password", &self.ricorda_password)
            .field("prefisso", &self.prefisso)
            .field("veicolo", &self.veicolo)
            .field("qos", &self.qos)
            .finish_non_exhaustive()
    }
}

/// Campo del modulo a cui si riferisce un problema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Campo {
    Host,
    Porta,
    IdClient,
    Prefisso,
    Qos,
    IntervalloSensore,
    IntervalloIstantanea,
    PrefissoDiscovery,
}

/// Un valore non accettabile, con il messaggio da mostrare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problema {
    pub campo: Campo,
    pub messaggio: String,
}

impl MqttConfig {
    /// Legge `mqtt.json`; se manca restituisce le impostazioni predefinite.
    pub fn carica(cartella: &Path) -> Result<Self, String> {
        Ok(config::carica(cartella, FILE_CONFIGURAZIONE)?.unwrap_or_default())
    }

    pub fn salva(&self, cartella: &Path) -> Result<(), String> {
        config::salva(cartella, FILE_CONFIGURAZIONE, self).map(|_| ())
    }

    /// Nome del veicolo ripulito (`a-z`, `0-9`, `_`, `-`).
    pub fn veicolo_pulito(&self) -> String {
        pulisci_segmento(&self.veicolo, "veicolo")
    }

    /// Valori da correggere prima di collegarsi; vuoto se la configurazione è valida.
    pub fn problemi(&self) -> Vec<Problema> {
        let mut v = Vec::new();
        let mut aggiungi = |campo, messaggio: String| v.push(Problema { campo, messaggio });
        let host = self.host.trim();
        if host.is_empty() {
            aggiungi(
                Campo::Host,
                "Indica l'indirizzo del broker (per esempio 192.168.1.10 o broker.local).".into(),
            );
        } else if host.chars().any(|c| c.is_whitespace() || c == '/') {
            aggiungi(
                Campo::Host,
                "L'indirizzo non deve contenere spazi né barre: scrivi solo il nome o l'IP, \
                 senza «mqtt://»."
                    .into(),
            );
        }
        if self.porta == 0 {
            aggiungi(
                Campo::Porta,
                "La porta deve essere un numero da 1 a 65535 (di solito 1883).".into(),
            );
        }
        if let Err(m) = valida_prefisso(&self.prefisso, "prefisso") {
            aggiungi(Campo::Prefisso, m);
        }
        if self.discovery_home_assistant
            && let Err(m) = valida_prefisso(&self.prefisso_discovery, "prefisso di discovery")
        {
            aggiungi(Campo::PrefissoDiscovery, m);
        }
        if self.qos > 1 {
            aggiungi(
                Campo::Qos,
                "La qualità del servizio può essere 0 o 1.".into(),
            );
        }
        let id = self.id_client.as_str();
        if id.chars().any(|c| c.is_whitespace() || c.is_control()) || id.len() > 128 {
            aggiungi(
                Campo::IdClient,
                "L'identificativo del client non deve contenere spazi e può avere al massimo 128 \
                 caratteri."
                    .into(),
            );
        }
        if self.intervallo_sensore_ms > INTERVALLO_MAX_MS {
            aggiungi(
                Campo::IntervalloSensore,
                "L'intervallo per sensore può arrivare al massimo a un'ora (3.600.000 ms).".into(),
            );
        }
        if !(100..=INTERVALLO_MAX_MS).contains(&self.intervallo_istantanea_ms) {
            aggiungi(
                Campo::IntervalloIstantanea,
                "L'intervallo dell'istantanea va da 100 ms a un'ora (3.600.000 ms).".into(),
            );
        }
        v
    }

    fn id_client_effettivo(&self) -> String {
        let id = self.id_client.trim();
        if !id.is_empty() {
            return id.to_string();
        }
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
            ^ std::process::id().rotate_left(16);
        format!("obdio-{}-{:08x}", self.veicolo_pulito(), n)
    }
}

/// Ripulisce un segmento di argomento: minuscole e solo `a-z 0-9 _ -`; il
/// resto diventa `_`. Se non resta nulla, `predefinito`.
pub fn pulisci_segmento(s: &str, predefinito: &str) -> String {
    let p: String = s
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if p.trim_matches('_').is_empty() {
        predefinito.to_string()
    } else {
        p
    }
}

/// Un prefisso può avere più livelli («casa/auto») ma niente caratteri jolly.
fn valida_prefisso(s: &str, nome: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("Il {nome} non può essere vuoto."));
    }
    if s.chars().any(|c| c == '+' || c == '#') {
        return Err(format!("Il {nome} non può contenere «+» o «#»."));
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("Il {nome} non può contenere spazi."));
    }
    if s.starts_with('/') || s.ends_with('/') || s.contains("//") {
        return Err(format!(
            "Il {nome} non deve iniziare o finire con «/» né contenere «//»."
        ));
    }
    if s.starts_with('$') {
        return Err(format!("Il {nome} non può iniziare con «$»."));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Argomenti
// ---------------------------------------------------------------------------

/// Costruisce gli argomenti a partire dalla configurazione.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Argomenti {
    base: String,
    discovery: String,
    veicolo: String,
}

impl Argomenti {
    pub fn new(c: &MqttConfig) -> Self {
        Argomenti {
            base: format!(
                "{}/{}",
                c.prefisso.trim().trim_matches('/'),
                c.veicolo_pulito()
            ),
            discovery: c.prefisso_discovery.trim().trim_matches('/').to_string(),
            veicolo: c.veicolo_pulito(),
        }
    }

    pub fn stato(&self) -> String {
        format!("{}/stato", self.base)
    }

    pub fn sensore(&self, chiave: &str) -> String {
        format!(
            "{}/sensori/{}",
            self.base,
            pulisci_segmento(chiave, "sconosciuto")
        )
    }

    /// Come [`Argomenti::sensore`] con `<chiave>` al posto della chiave (per l'anteprima).
    pub fn modello_sensore(&self) -> String {
        format!("{}/sensori/<chiave>", self.base)
    }

    /// Come [`Argomenti::discovery_sensore`] con `<chiave>` (per l'anteprima).
    pub fn modello_discovery(&self) -> String {
        format!(
            "{}/sensor/obdio_{}_<chiave>/config",
            self.discovery, self.veicolo
        )
    }

    pub fn istantanea(&self) -> String {
        format!("{}/istantanea", self.base)
    }

    pub fn diagnosi(&self) -> String {
        format!("{}/diagnosi", self.base)
    }

    pub fn evento(&self) -> String {
        format!("{}/evento", self.base)
    }

    /// Identificativo unico dell'entità in Home Assistant.
    pub fn id_unico(&self, chiave: &str) -> String {
        format!(
            "obdio_{}_{}",
            self.veicolo,
            pulisci_segmento(chiave, "sconosciuto")
        )
    }

    pub fn discovery_sensore(&self, chiave: &str) -> String {
        format!("{}/sensor/{}/config", self.discovery, self.id_unico(chiave))
    }

    /// Identificativo del dispositivo in Home Assistant.
    pub fn id_dispositivo(&self) -> String {
        format!("obdio_{}", self.veicolo)
    }
}

// ---------------------------------------------------------------------------
// Pezzi puri
// ---------------------------------------------------------------------------

/// Da dove arrivano i dati pubblicati.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origine {
    Veicolo,
    DemoSintetico,
}

impl Origine {
    pub fn codice(self) -> &'static str {
        match self {
            Origine::Veicolo => "veicolo",
            Origine::DemoSintetico => "demo_sintetico",
        }
    }
}

/// Limita la frequenza per chiave: ammette al massimo un messaggio ogni
/// `intervallo`.
#[derive(Clone, Debug)]
pub struct Limitatore {
    intervallo: Duration,
    ultimo: BTreeMap<String, Instant>,
}

impl Limitatore {
    pub fn new(intervallo: Duration) -> Self {
        Limitatore {
            intervallo,
            ultimo: BTreeMap::new(),
        }
    }

    /// Vero se `chiave` può essere pubblicata all'istante `ora` (e in tal caso
    /// lo registra).
    pub fn ammetti(&mut self, chiave: &str, ora: Instant) -> bool {
        if let Some(u) = self.ultimo.get(chiave)
            && ora.saturating_duration_since(*u) < self.intervallo
        {
            return false;
        }
        self.ultimo.insert(chiave.to_string(), ora);
        true
    }
}

/// Messaggio di stato: `online` con l'origine dei dati, oppure `offline`.
pub fn payload_stato(origine: Option<Origine>) -> String {
    match origine {
        Some(o) => json!({
            "stato": "online",
            "origine": o.codice(),
            "app": "OBDio",
            "versione": env!("CARGO_PKG_VERSION"),
        })
        .to_string(),
        None => payload_offline(),
    }
}

pub fn payload_offline() -> String {
    json!({ "stato": "offline" }).to_string()
}

/// Il numero in testo con il punto decimale; `None` se non è un numero finito.
pub fn payload_sensore(v: f64) -> Option<String> {
    v.is_finite().then(|| format!("{v}"))
}

/// Vero se il campione porta un valore da pubblicare come sensore.
fn valore_valido(c: &Sample) -> Option<f64> {
    c.valore.filter(|v| v.is_finite() && c.stato == Stato::Ok)
}

/// JSON dell'istantanea. Un valore assente è `null`, con lo stato che ne dice il motivo.
pub fn istantanea_json(
    ts: &str,
    origine: Option<Origine>,
    ultimi: &BTreeMap<String, Sample>,
) -> Value {
    let mut valori = serde_json::Map::new();
    for (k, c) in ultimi {
        valori.insert(
            k.clone(),
            json!({
                "valore": valore_valido(c),
                "unita": c.unita,
                "stato": c.stato.codice(),
                "ecu": c.ecu,
            }),
        );
    }
    json!({
        "ts": ts,
        "origine": origine.map(Origine::codice),
        "valori": valori,
    })
}

/// Classe del dispositivo per Home Assistant, solo dove non c'è ambiguità.
fn classe_dispositivo(unita: &str) -> Option<&'static str> {
    match unita {
        "°C" => Some("temperature"),
        "V" => Some("voltage"),
        "kPa" => Some("pressure"),
        "km/h" => Some("speed"),
        "km" => Some("distance"),
        _ => None,
    }
}

/// Configurazione di discovery di un sensore numerico.
///
/// La disponibilità legge il campo `stato` del JSON pubblicato su `…/stato`
/// (`availability_template`).
pub fn discovery_json(a: &Argomenti, chiave: &str, nome: &str, unita: &str) -> Value {
    let mut v = json!({
        "name": nome,
        "unique_id": a.id_unico(chiave),
        "has_entity_name": true,
        "state_topic": a.sensore(chiave),
        "availability_topic": a.stato(),
        "availability_template": "{{ value_json.stato }}",
        "payload_available": "online",
        "payload_not_available": "offline",
        "state_class": "measurement",
        "device": {
            "identifiers": [a.id_dispositivo()],
            "name": format!("OBDio {}", a.veicolo),
            "manufacturer": "OBDio",
            "sw_version": env!("CARGO_PKG_VERSION"),
        },
    });
    let o = v.as_object_mut().expect("oggetto");
    if !unita.is_empty() {
        o.insert("unit_of_measurement".into(), json!(unita));
    }
    if let Some(c) = classe_dispositivo(unita) {
        o.insert("device_class".into(), json!(c));
    }
    v
}

fn evento_marcatore_json(m: &Marker, origine: Option<Origine>) -> Value {
    json!({
        "ts": m.ts,
        "tipo": "marcatore",
        "etichetta": m.etichetta,
        "origine": origine.map(Origine::codice),
    })
}

fn evento_cancellazione_json(
    esito: &EsitoCancellazione,
    eseguita_il: &str,
    origine: Option<Origine>,
) -> Value {
    let (codice, dettaglio) = match esito {
        EsitoCancellazione::Risposte(v) => {
            let testo = v
                .iter()
                .map(|e| format!("{}: {}", e.ecu, e.testo()))
                .collect::<Vec<_>>()
                .join("; ");
            let accettata = v
                .iter()
                .any(|e| e.risposta == RispostaCancellazione::Accettata);
            (if accettata { "accettata" } else { "rifiutata" }, testo)
        }
        EsitoCancellazione::NessunaRisposta => (
            "nessuna_risposta",
            "nessuna centralina ha risposto".to_string(),
        ),
        EsitoCancellazione::ErroreComunicazione(d) => ("errore", d.clone()),
    };
    json!({
        "ts": eseguita_il,
        "tipo": "codici_cancellati",
        "esito": codice,
        "dettaglio": dettaglio,
        "origine": origine.map(Origine::codice),
    })
}

// ---------------------------------------------------------------------------
// Stato e coda
// ---------------------------------------------------------------------------

/// Stato della connessione con il broker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatoMqtt {
    Disattivato,
    Connessione,
    Connesso,
    Errore(String),
}

impl StatoMqtt {
    pub fn testo(&self) -> String {
        match self {
            StatoMqtt::Disattivato => "Disattivato".into(),
            StatoMqtt::Connessione => "Connessione in corso".into(),
            StatoMqtt::Connesso => "Connesso".into(),
            StatoMqtt::Errore(e) => format!("Errore: {e}"),
        }
    }
}

/// Fotografia dello stato, economica da copiare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatoMqttInfo {
    pub stato: StatoMqtt,
    /// Messaggi accettati dalla coda di invio.
    pub pubblicati: u64,
    /// Messaggi dei sensori non accodati (coda piena o broker non raggiungibile).
    pub scartati: u64,
    pub ultimo_errore: Option<String>,
}

impl StatoMqttInfo {
    pub fn disattivato() -> Self {
        StatoMqttInfo {
            stato: StatoMqtt::Disattivato,
            pubblicati: 0,
            scartati: 0,
            ultimo_errore: None,
        }
    }
}

/// Dove vanno i messaggi. Non deve mai bloccare: se non c'è posto, `Err`.
pub trait Coda: Send + Sync {
    fn invia(
        &self,
        argomento: &str,
        payload: &[u8],
        qos: u8,
        trattenuto: bool,
    ) -> Result<(), String>;

    /// Chiede la disconnessione pulita (dopo i messaggi già accodati).
    fn chiudi(&self) {}
}

struct Nucleo {
    coda: Box<dyn Coda>,
    argomenti: Argomenti,
    qos: u8,
    stato: Mutex<StatoMqtt>,
    pubblicati: AtomicU64,
    scartati: AtomicU64,
    ultimo_errore: Mutex<Option<String>>,
    /// Messaggi trattenuti da rinviare a ogni (ri)connessione, in ordine.
    trattenuti: Mutex<Vec<(String, Vec<u8>)>>,
    da_rinviare: AtomicBool,
    stop: AtomicBool,
    scadenza_stop: Mutex<Option<Instant>>,
}

fn blocca<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Nucleo {
    fn imposta_stato(&self, s: StatoMqtt) {
        *blocca(&self.stato) = s;
    }

    fn imposta_errore(&self, e: String) {
        *blocca(&self.ultimo_errore) = Some(e.clone());
        self.imposta_stato(StatoMqtt::Errore(e));
    }

    fn connesso(&self) -> bool {
        *blocca(&self.stato) == StatoMqtt::Connesso
    }

    fn fermato(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Fermo da oltre il tempo concesso alla chiusura.
    fn scaduto(&self) -> bool {
        self.fermato() && blocca(&self.scadenza_stop).is_some_and(|s| Instant::now() >= s)
    }

    /// Messaggio a perdere (sensori, istantanea, eventi).
    fn invia_volatile(&self, argomento: &str, payload: &[u8]) {
        if !self.connesso() {
            self.scartati.fetch_add(1, Ordering::Relaxed);
            return;
        }
        match self.coda.invia(argomento, payload, self.qos, false) {
            Ok(()) => {
                self.pubblicati.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.scartati.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Messaggio trattenuto: ricordato e rinviato a ogni riconnessione.
    fn invia_trattenuto(&self, argomento: &str, payload: Vec<u8>) {
        let mut cache = blocca(&self.trattenuti);
        match cache.iter_mut().find(|(a, _)| a == argomento) {
            Some(voce) => voce.1 = payload.clone(),
            None => cache.push((argomento.to_string(), payload.clone())),
        }
        if !self.connesso() {
            return;
        }
        match self.coda.invia(argomento, &payload, self.qos, true) {
            Ok(()) => {
                self.pubblicati.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => self.da_rinviare.store(true, Ordering::Relaxed),
        }
    }

    /// Rinvia tutti i messaggi trattenuti.
    fn ripubblica(&self) {
        let cache = blocca(&self.trattenuti);
        self.da_rinviare.store(false, Ordering::Relaxed);
        for (a, p) in cache.iter() {
            match self.coda.invia(a, p, self.qos, true) {
                Ok(()) => {
                    self.pubblicati.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    self.da_rinviare.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    }

    /// Attesa interrompibile dalla chiusura.
    fn dormi(&self, durata: Duration) {
        let fine = Instant::now() + durata;
        while !self.fermato() {
            let resto = fine.saturating_duration_since(Instant::now());
            if resto.is_zero() {
                break;
            }
            std::thread::sleep(resto.min(Duration::from_millis(50)));
        }
    }
}

struct Logica {
    limite_sensori: Limitatore,
    limite_istantanea: Limitatore,
    ultimi: BTreeMap<String, Sample>,
    annunciati: BTreeSet<String>,
    origine: Option<Origine>,
    discovery: bool,
}

/// Il pubblicatore: riceve i dati dal thread di lavoro e li passa alla coda.
/// Tutti i metodi sono non bloccanti.
pub struct MqttPublisher {
    nucleo: Arc<Nucleo>,
    logica: Mutex<Logica>,
    thread: Mutex<Option<(JoinHandle<()>, mpsc::Receiver<()>)>>,
}

impl MqttPublisher {
    /// Si collega al broker in un thread proprio. `Err` se la configurazione
    /// non è valida.
    pub fn avvia(config: &MqttConfig) -> Result<Self, String> {
        use rumqttc::{Client, LastWill, MqttOptions};

        if let Some(p) = config.problemi().first() {
            return Err(p.messaggio.clone());
        }
        let argomenti = Argomenti::new(config);
        let qos = config.qos.min(1);
        let mut opzioni = MqttOptions::new(
            config.id_client_effettivo(),
            config.host.trim(),
            config.porta,
        );
        opzioni
            .set_keep_alive(Duration::from_secs(15))
            .set_clean_session(true)
            .set_max_packet_size(PACCHETTO_MAX, PACCHETTO_MAX)
            .set_last_will(LastWill::new(
                argomenti.stato(),
                payload_offline(),
                qos_di(qos),
                true,
            ));
        if !config.utente.is_empty() {
            opzioni.set_credentials(config.utente.clone(), config.password.clone());
        }
        let (client, mut connessione) = Client::new(opzioni, CAPACITA_CODA);
        let nucleo = Arc::new(Nucleo::new(
            Box::new(CodaRumqttc { client }),
            argomenti,
            qos,
            StatoMqtt::Connessione,
        ));
        let n = nucleo.clone();
        let (fine_tx, fine_rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("obdio-mqtt".into())
            .spawn(move || {
                pilota(&mut connessione, &n);
                let _ = fine_tx.send(());
            })
            .map_err(|e| format!("impossibile avviare il thread MQTT: {e}"))?;
        let p = Self::da_nucleo(config, nucleo, Some((handle, fine_rx)));
        p.pubblica_stato();
        Ok(p)
    }

    /// Con una coda propria, già «connessa» (verifiche).
    pub fn con_coda(config: &MqttConfig, coda: Box<dyn Coda>) -> Self {
        let nucleo = Arc::new(Nucleo::new(
            coda,
            Argomenti::new(config),
            config.qos.min(1),
            StatoMqtt::Connesso,
        ));
        let p = Self::da_nucleo(config, nucleo, None);
        p.pubblica_stato();
        p
    }

    fn da_nucleo(
        config: &MqttConfig,
        nucleo: Arc<Nucleo>,
        thread: Option<(JoinHandle<()>, mpsc::Receiver<()>)>,
    ) -> Self {
        MqttPublisher {
            nucleo,
            logica: Mutex::new(Logica {
                limite_sensori: Limitatore::new(Duration::from_millis(
                    config.intervallo_sensore_ms,
                )),
                limite_istantanea: Limitatore::new(Duration::from_millis(
                    config.intervallo_istantanea_ms,
                )),
                ultimi: BTreeMap::new(),
                annunciati: BTreeSet::new(),
                origine: None,
                discovery: config.discovery_home_assistant,
            }),
            thread: Mutex::new(thread),
        }
    }

    pub fn argomenti(&self) -> &Argomenti {
        &self.nucleo.argomenti
    }

    /// Stato corrente e contatori.
    pub fn stato(&self) -> StatoMqttInfo {
        StatoMqttInfo {
            stato: blocca(&self.nucleo.stato).clone(),
            pubblicati: self.nucleo.pubblicati.load(Ordering::Relaxed),
            scartati: self.nucleo.scartati.load(Ordering::Relaxed),
            ultimo_errore: blocca(&self.nucleo.ultimo_errore).clone(),
        }
    }

    fn pubblica_stato(&self) {
        let origine = blocca(&self.logica).origine;
        self.pubblica_stato_con(origine);
    }

    fn pubblica_stato_con(&self, origine: Option<Origine>) {
        self.nucleo.invia_trattenuto(
            &self.nucleo.argomenti.stato(),
            payload_stato(origine).into_bytes(),
        );
    }

    /// Un collegamento con il veicolo (o con il simulatore) è attivo.
    pub fn collegato(&self, demo: bool) {
        if self.nucleo.fermato() {
            return;
        }
        let origine = if demo {
            Origine::DemoSintetico
        } else {
            Origine::Veicolo
        };
        {
            let mut l = blocca(&self.logica);
            l.origine = Some(origine);
            l.ultimi.clear();
        }
        self.pubblica_stato_con(Some(origine));
    }

    /// Il collegamento è terminato: i sensori non sono più disponibili.
    pub fn scollegato(&self) {
        if self.nucleo.fermato() {
            return;
        }
        {
            let mut l = blocca(&self.logica);
            l.origine = None;
            l.ultimi.clear();
        }
        self.pubblica_stato_con(None);
    }

    /// Un gruppo di letture. Pubblica i soli valori validi (limitati per
    /// sensore) e, a intervalli, l'istantanea con anche i valori assenti.
    pub fn campioni(&self, campioni: &[Sample]) {
        if self.nucleo.fermato() {
            return;
        }
        let ora = Instant::now();
        let mut l = blocca(&self.logica);
        let a = &self.nucleo.argomenti;
        for c in campioni {
            l.ultimi.insert(c.key.clone(), c.clone());
            let Some(v) = valore_valido(c) else {
                continue;
            };
            if l.discovery && l.annunciati.insert(c.key.clone()) {
                let (nome, unita) = match pids::per_key(&c.key) {
                    Some(p) => (p.nome.clone(), p.unita.to_string()),
                    None => (c.key.clone(), c.unita.clone()),
                };
                self.nucleo.invia_trattenuto(
                    &a.discovery_sensore(&c.key),
                    discovery_json(a, &c.key, &nome, &unita)
                        .to_string()
                        .into_bytes(),
                );
            }
            if l.limite_sensori.ammetti(&c.key, ora)
                && let Some(testo) = payload_sensore(v)
            {
                self.nucleo
                    .invia_volatile(&a.sensore(&c.key), testo.as_bytes());
            }
        }
        if !l.ultimi.is_empty() && l.limite_istantanea.ammetti("", ora) {
            let json = istantanea_json(&adesso_iso(), l.origine, &l.ultimi);
            self.nucleo
                .invia_volatile(&a.istantanea(), json.to_string().as_bytes());
        }
    }

    /// Diagnosi appena letta. Il VIN non fa parte del referto serializzato.
    pub fn diagnosi(&self, report: &DiagnosisReport) {
        if self.nucleo.fermato() {
            return;
        }
        if let Ok(testo) = serde_json::to_vec(report) {
            self.nucleo
                .invia_trattenuto(&self.nucleo.argomenti.diagnosi(), testo);
        }
    }

    pub fn marcatore(&self, m: &Marker) {
        if self.nucleo.fermato() {
            return;
        }
        let origine = blocca(&self.logica).origine;
        self.nucleo.invia_volatile(
            &self.nucleo.argomenti.evento(),
            evento_marcatore_json(m, origine).to_string().as_bytes(),
        );
    }

    pub fn cancellazione(&self, esito: &EsitoCancellazione, eseguita_il: &str) {
        if self.nucleo.fermato() {
            return;
        }
        let origine = blocca(&self.logica).origine;
        self.nucleo.invia_volatile(
            &self.nucleo.argomenti.evento(),
            evento_cancellazione_json(esito, eseguita_il, origine)
                .to_string()
                .as_bytes(),
        );
    }

    /// Chiusura pulita: pubblica `offline`, si disconnette e attende il thread
    /// per un tempo limitato (non blocca mai l'uscita se il broker non risponde).
    pub fn ferma(&self) {
        if self.nucleo.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        *blocca(&self.nucleo.scadenza_stop) = Some(Instant::now() + ATTESA_CHIUSURA);
        self.nucleo.invia_trattenuto(
            &self.nucleo.argomenti.stato(),
            payload_offline().into_bytes(),
        );
        self.nucleo.coda.chiudi();
        if let Some((handle, fine)) = blocca(&self.thread).take() {
            match fine.recv_timeout(ATTESA_CHIUSURA) {
                Err(mpsc::RecvTimeoutError::Timeout) => drop(handle),
                _ => {
                    let _ = handle.join();
                }
            }
        }
        self.nucleo.imposta_stato(StatoMqtt::Disattivato);
    }
}

impl Drop for MqttPublisher {
    fn drop(&mut self) {
        self.ferma();
    }
}

impl Nucleo {
    fn new(coda: Box<dyn Coda>, argomenti: Argomenti, qos: u8, stato: StatoMqtt) -> Self {
        Nucleo {
            coda,
            argomenti,
            qos,
            stato: Mutex::new(stato),
            pubblicati: AtomicU64::new(0),
            scartati: AtomicU64::new(0),
            ultimo_errore: Mutex::new(None),
            trattenuti: Mutex::new(Vec::new()),
            da_rinviare: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            scadenza_stop: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Libreria di rete (rumqttc)
// ---------------------------------------------------------------------------

fn qos_di(qos: u8) -> rumqttc::QoS {
    if qos >= 1 {
        rumqttc::QoS::AtLeastOnce
    } else {
        rumqttc::QoS::AtMostOnce
    }
}

struct CodaRumqttc {
    client: rumqttc::Client,
}

impl Coda for CodaRumqttc {
    fn invia(
        &self,
        argomento: &str,
        payload: &[u8],
        qos: u8,
        trattenuto: bool,
    ) -> Result<(), String> {
        self.client
            .try_publish(argomento, qos_di(qos), trattenuto, payload)
            .map_err(|e| e.to_string())
    }

    fn chiudi(&self) {
        let _ = self.client.try_disconnect();
    }
}

fn traduci_errore(e: &rumqttc::ConnectionError) -> String {
    use rumqttc::ConnectionError as E;
    match e {
        E::Io(io) => match io.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                "connessione rifiutata: il broker non è in ascolto su quell'indirizzo e porta"
                    .to_string()
            }
            std::io::ErrorKind::TimedOut => "tempo scaduto nel collegamento al broker".to_string(),
            _ => format!("connessione non riuscita o interrotta ({io})"),
        },
        E::NetworkTimeout | E::FlushTimeout => "il broker non risponde (tempo scaduto)".into(),
        E::ConnectionRefused(codice) => {
            format!("il broker ha rifiutato l'accesso ({codice:?}): controlla utente e password")
        }
        E::NotConnAck(_) => "risposta inattesa del broker alla connessione".into(),
        altro => format!("{altro}"),
    }
}

/// Fa avanzare la connessione: ogni `Err` è una connessione fallita o caduta;
/// continuare il ciclo la riapre, con una pausa crescente da 1 a 30 secondi.
fn pilota(connessione: &mut rumqttc::Connection, n: &Nucleo) {
    use rumqttc::{Event, Outgoing, Packet};

    let mut attesa = ATTESA_MIN;
    for evento in connessione.iter() {
        match evento {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                attesa = ATTESA_MIN;
                n.imposta_stato(StatoMqtt::Connesso);
                n.ripubblica();
            }
            Ok(Event::Outgoing(Outgoing::Disconnect)) => break,
            Ok(_) => {
                if n.da_rinviare.load(Ordering::Relaxed) {
                    n.ripubblica();
                }
                if n.scaduto() {
                    break;
                }
            }
            Err(e) => {
                if n.fermato() {
                    break;
                }
                n.imposta_errore(traduci_errore(&e));
                n.dormi(attesa);
                attesa = (attesa * 2).min(ATTESA_MAX);
                if n.fermato() {
                    break;
                }
                n.imposta_stato(StatoMqtt::Connessione);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnosis::DiagnosisReport;
    use crate::elm::EsitoCentralina;

    /// Messaggio registrato dalla coda finta: (argomento, payload, qos, trattenuto).
    type Msg = (String, String, u8, bool);

    #[derive(Clone, Default)]
    struct Finta {
        msg: Arc<Mutex<Vec<Msg>>>,
        piena: Arc<AtomicBool>,
        chiusa: Arc<AtomicBool>,
    }

    impl Coda for Finta {
        fn invia(&self, a: &str, p: &[u8], qos: u8, r: bool) -> Result<(), String> {
            if self.piena.load(Ordering::Relaxed) {
                return Err("coda piena".into());
            }
            blocca(&self.msg).push((a.into(), String::from_utf8_lossy(p).into(), qos, r));
            Ok(())
        }
        fn chiudi(&self) {
            self.chiusa.store(true, Ordering::Relaxed);
        }
    }

    impl Finta {
        fn tutti(&self) -> Vec<Msg> {
            blocca(&self.msg).clone()
        }
        fn su(&self, argomento: &str) -> Vec<Msg> {
            self.tutti()
                .into_iter()
                .filter(|m| m.0 == argomento)
                .collect()
        }
    }

    fn config() -> MqttConfig {
        MqttConfig {
            attivo: true,
            host: "broker.local".into(),
            veicolo: "Auto di Mario!".into(),
            intervallo_sensore_ms: 0,
            ..Default::default()
        }
    }

    fn campione(key: &str, valore: Option<f64>, stato: Stato) -> Sample {
        Sample {
            t: 1.0,
            ts: "2026-01-01T10:00:00.000+01:00".into(),
            ecu: "7E8".into(),
            key: key.into(),
            pid: 0x0C,
            valore,
            unita: "giri/min".into(),
            stato,
        }
    }

    #[test]
    fn segmenti_ripuliti() {
        assert_eq!(pulisci_segmento("Auto di Mario!", "v"), "auto_di_mario_");
        assert_eq!(pulisci_segmento("Fiat-500_X", "v"), "fiat-500_x");
        assert_eq!(pulisci_segmento("  ", "veicolo"), "veicolo");
        assert_eq!(pulisci_segmento("///", "veicolo"), "veicolo");
        assert_eq!(pulisci_segmento("à+#", "x"), "x");
    }

    #[test]
    fn argomenti_costruiti() {
        let mut c = config();
        c.prefisso = "casa/obd".into();
        let a = Argomenti::new(&c);
        assert_eq!(a.stato(), "casa/obd/auto_di_mario_/stato");
        assert_eq!(
            a.sensore("o2s1_lambda"),
            "casa/obd/auto_di_mario_/sensori/o2s1_lambda"
        );
        assert_eq!(a.sensore("A/B+#"), "casa/obd/auto_di_mario_/sensori/a_b__");
        assert_eq!(a.istantanea(), "casa/obd/auto_di_mario_/istantanea");
        assert_eq!(a.diagnosi(), "casa/obd/auto_di_mario_/diagnosi");
        assert_eq!(a.evento(), "casa/obd/auto_di_mario_/evento");
        assert_eq!(
            a.discovery_sensore("giri"),
            "homeassistant/sensor/obdio_auto_di_mario__giri/config"
        );
    }

    #[test]
    fn limitatore_per_chiave() {
        let t0 = Instant::now();
        let mut l = Limitatore::new(Duration::from_millis(500));
        assert!(l.ammetti("giri", t0));
        assert!(!l.ammetti("giri", t0 + Duration::from_millis(499)));
        // Un'altra chiave non è influenzata.
        assert!(l.ammetti("velocita", t0 + Duration::from_millis(10)));
        assert!(l.ammetti("giri", t0 + Duration::from_millis(500)));
        // Il rifiuto non sposta l'ultimo istante: 500 ms dall'ultima pubblicazione.
        assert!(!l.ammetti("giri", t0 + Duration::from_millis(999)));
        assert!(l.ammetti("giri", t0 + Duration::from_millis(1000)));
        let mut zero = Limitatore::new(Duration::ZERO);
        assert!(zero.ammetti("k", t0) && zero.ammetti("k", t0));
    }

    #[test]
    fn istantanea_con_valori_assenti_a_null() {
        let mut ultimi = BTreeMap::new();
        ultimi.insert("giri".into(), campione("giri", Some(2128.0), Stato::Ok));
        ultimi.insert(
            "o2s2_tensione".into(),
            campione("o2s2_tensione", None, Stato::RispostaMancante),
        );
        let j = istantanea_json(
            "2026-01-01T10:00:00+01:00",
            Some(Origine::DemoSintetico),
            &ultimi,
        );
        assert_eq!(j["origine"], "demo_sintetico");
        assert_eq!(j["valori"]["giri"]["valore"], 2128.0);
        assert_eq!(j["valori"]["giri"]["stato"], "ok");
        assert_eq!(j["valori"]["giri"]["ecu"], "7E8");
        assert!(j["valori"]["o2s2_tensione"]["valore"].is_null());
        assert_eq!(j["valori"]["o2s2_tensione"]["stato"], "risposta_mancante");
        // Nemmeno uno zero.
        assert!(!j.to_string().contains("\"valore\":0"));
    }

    #[test]
    fn payload_sensore_solo_numeri_finiti() {
        assert_eq!(payload_sensore(2128.0).unwrap(), "2128");
        assert_eq!(payload_sensore(0.78125).unwrap(), "0.78125");
        assert_eq!(payload_sensore(-3.5).unwrap(), "-3.5");
        assert_eq!(payload_sensore(f64::NAN), None);
        assert_eq!(payload_sensore(f64::INFINITY), None);
    }

    #[test]
    fn payload_stato_online_e_offline() {
        let v: Value = serde_json::from_str(&payload_stato(Some(Origine::Veicolo))).unwrap();
        assert_eq!(v["stato"], "online");
        assert_eq!(v["origine"], "veicolo");
        assert_eq!(v["app"], "OBDio");
        assert!(v["versione"].is_string());
        assert_eq!(payload_stato(None), r#"{"stato":"offline"}"#);
    }

    #[test]
    fn discovery_completa() {
        let a = Argomenti::new(&config());
        let v = discovery_json(&a, "temp_refrigerante", "Temperatura refrigerante", "°C");
        assert_eq!(v["unique_id"], "obdio_auto_di_mario__temp_refrigerante");
        assert_eq!(
            v["state_topic"],
            "obdio/auto_di_mario_/sensori/temp_refrigerante"
        );
        assert_eq!(v["unit_of_measurement"], "°C");
        assert_eq!(v["device_class"], "temperature");
        assert_eq!(v["state_class"], "measurement");
        assert_eq!(v["availability_topic"], "obdio/auto_di_mario_/stato");
        assert_eq!(v["availability_template"], "{{ value_json.stato }}");
        assert_eq!(v["payload_available"], "online");
        assert_eq!(v["device"]["identifiers"][0], "obdio_auto_di_mario_");
        assert_eq!(v["device"]["manufacturer"], "OBDio");
        // Nessuna classe dove sarebbe ambigua.
        let giri = discovery_json(&a, "giri", "Giri motore", "giri/min");
        assert!(giri.get("device_class").is_none());
        for (u, c) in [
            ("V", "voltage"),
            ("kPa", "pressure"),
            ("km/h", "speed"),
            ("km", "distance"),
        ] {
            assert_eq!(discovery_json(&a, "x", "x", u)["device_class"], c);
        }
        assert!(discovery_json(&a, "x", "x", "")["unit_of_measurement"].is_null());
    }

    #[test]
    fn configurazione_password_su_disco_solo_se_richiesto() {
        let mut c = config();
        c.utente = "mario".into();
        c.password = "segretissima".into();
        let senza = serde_json::to_string(&c).unwrap();
        assert!(!senza.contains("segretissima"), "{senza}");
        assert!(!senza.contains("\"password\""), "{senza}");
        assert!(senza.contains("\"ricorda_password\":false"));
        c.ricorda_password = true;
        let con = serde_json::to_string(&c).unwrap();
        assert!(con.contains("\"password\":\"segretissima\""), "{con}");
        let letta: MqttConfig = serde_json::from_str(&con).unwrap();
        assert_eq!(letta, c);
        // Debug non la mostra.
        assert!(!format!("{c:?}").contains("segretissima"));
    }

    #[test]
    fn configurazione_su_file_e_valori_mancanti() {
        let dir = std::env::temp_dir().join(format!("obdio_mqtt_cfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(MqttConfig::carica(&dir).unwrap(), MqttConfig::default());
        let mut c = config();
        c.password = "x".into();
        c.salva(&dir).unwrap();
        let testo = std::fs::read_to_string(dir.join(FILE_CONFIGURAZIONE)).unwrap();
        assert!(!testo.contains("\"x\""));
        let letta = MqttConfig::carica(&dir).unwrap();
        assert_eq!(letta.host, "broker.local");
        assert_eq!(letta.password, "");
        // File parziale: il resto prende i valori predefiniti.
        std::fs::write(
            dir.join(FILE_CONFIGURAZIONE),
            r#"{"host":"h","porta":8883}"#,
        )
        .unwrap();
        let parziale = MqttConfig::carica(&dir).unwrap();
        assert_eq!((parziale.host.as_str(), parziale.porta), ("h", 8883));
        assert_eq!(parziale.prefisso, "obdio");
        assert_eq!(parziale.intervallo_istantanea_ms, 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validazione_in_italiano() {
        assert!(config().problemi().is_empty());
        let campi = |c: &MqttConfig| c.problemi().iter().map(|p| p.campo).collect::<Vec<_>>();
        let mut c = config();
        c.host = "  ".into();
        assert_eq!(campi(&c), vec![Campo::Host]);
        assert!(c.problemi()[0].messaggio.contains("indirizzo del broker"));
        c.host = "mqtt://x".into();
        assert_eq!(campi(&c), vec![Campo::Host]);
        c = config();
        c.porta = 0;
        assert_eq!(campi(&c), vec![Campo::Porta]);
        for brutto in [
            "",
            "a/+/b",
            "casa/#",
            "/casa",
            "casa/",
            "a//b",
            "con spazio",
            "$SYS/x",
        ] {
            let mut c = config();
            c.prefisso = brutto.into();
            assert_eq!(campi(&c), vec![Campo::Prefisso], "{brutto:?}");
        }
        c = config();
        c.prefisso = "casa/auto".into();
        assert!(c.problemi().is_empty());
        c.qos = 2;
        assert_eq!(campi(&c), vec![Campo::Qos]);
        c = config();
        c.intervallo_istantanea_ms = 10;
        assert_eq!(campi(&c), vec![Campo::IntervalloIstantanea]);
        c = config();
        c.prefisso_discovery = "a#".into();
        assert!(
            c.problemi().is_empty(),
            "controllato solo se la discovery è attiva"
        );
        c.discovery_home_assistant = true;
        assert_eq!(campi(&c), vec![Campo::PrefissoDiscovery]);
        assert!(MqttPublisher::avvia(&MqttConfig::default()).is_err());
    }

    #[test]
    fn pubblica_solo_valori_validi_con_limite() {
        let f = Finta::default();
        let mut c = config();
        c.intervallo_sensore_ms = 60_000;
        let p = MqttPublisher::con_coda(&c, Box::new(f.clone()));
        p.collegato(true);
        p.campioni(&[
            campione("giri", Some(2128.0), Stato::Ok),
            campione("o2s2_tensione", None, Stato::RispostaMancante),
            campione("velocita", None, Stato::ErroreComunicazione),
            campione("map", Some(f64::NAN), Stato::Ok),
        ]);
        p.campioni(&[campione("giri", Some(2200.0), Stato::Ok)]);
        let a = p.argomenti().clone();
        // Un solo messaggio per «giri» (il secondo è oltre il limite), nessuno per i valori assenti.
        let giri = f.su(&a.sensore("giri"));
        assert_eq!(giri.len(), 1);
        assert_eq!(giri[0].1, "2128");
        assert!(!giri[0].3, "i sensori non sono trattenuti");
        assert!(f.su(&a.sensore("o2s2_tensione")).is_empty());
        assert!(f.su(&a.sensore("velocita")).is_empty());
        assert!(f.su(&a.sensore("map")).is_empty());
        // L'istantanea (una sola nell'intervallo) riporta gli assenti come null.
        let ist = f.su(&a.istantanea());
        assert_eq!(ist.len(), 1);
        let j: Value = serde_json::from_str(&ist[0].1).unwrap();
        assert_eq!(j["origine"], "demo_sintetico");
        assert!(j["valori"]["o2s2_tensione"]["valore"].is_null());
        assert_eq!(j["valori"]["velocita"]["stato"], "errore_comunicazione");
        // Nessun payload è zero per un valore assente.
        assert!(f.tutti().iter().all(|m| m.1 != "0"));
    }

    #[test]
    fn stato_origine_e_ultima_volonta_alla_chiusura() {
        let f = Finta::default();
        let p = MqttPublisher::con_coda(&config(), Box::new(f.clone()));
        let st = p.argomenti().stato();
        p.collegato(false);
        p.scollegato();
        p.collegato(true);
        p.ferma();
        let v: Vec<(String, bool)> = f.su(&st).into_iter().map(|m| (m.1, m.3)).collect();
        assert!(v.iter().all(|(_, r)| *r), "lo stato è sempre trattenuto");
        let stati: Vec<Value> = v
            .iter()
            .map(|(t, _)| serde_json::from_str(t).unwrap())
            .collect();
        let nomi: Vec<(&str, &str)> = stati
            .iter()
            .map(|s| {
                (
                    s["stato"].as_str().unwrap(),
                    s["origine"].as_str().unwrap_or("-"),
                )
            })
            .collect();
        assert_eq!(
            nomi,
            vec![
                ("offline", "-"),
                ("online", "veicolo"),
                ("offline", "-"),
                ("online", "demo_sintetico"),
                ("offline", "-"),
            ]
        );
        assert!(f.chiusa.load(Ordering::Relaxed));
        assert_eq!(p.stato().stato, StatoMqtt::Disattivato);
        // Dopo la chiusura non esce più nulla.
        let n = f.tutti().len();
        p.campioni(&[campione("giri", Some(1.0), Stato::Ok)]);
        p.collegato(true);
        assert_eq!(f.tutti().len(), n);
    }

    #[test]
    fn coda_piena_scarta_e_conta_senza_bloccare() {
        let f = Finta::default();
        let p = MqttPublisher::con_coda(&config(), Box::new(f.clone()));
        f.piena.store(true, Ordering::Relaxed);
        p.campioni(&[
            campione("giri", Some(1.0), Stato::Ok),
            campione("velocita", Some(2.0), Stato::Ok),
        ]);
        let s = p.stato();
        // Due sensori più l'istantanea.
        assert_eq!(s.scartati, 3);
        let prima = s.pubblicati;
        f.piena.store(false, Ordering::Relaxed);
        p.campioni(&[campione("giri", Some(3.0), Stato::Ok)]);
        assert!(p.stato().pubblicati > prima);
    }

    #[test]
    fn senza_connessione_si_scarta_e_si_rinvia_il_trattenuto() {
        let f = Finta::default();
        let c = config();
        let nucleo = Arc::new(Nucleo::new(
            Box::new(f.clone()),
            Argomenti::new(&c),
            0,
            StatoMqtt::Connessione,
        ));
        let p = MqttPublisher::da_nucleo(&c, nucleo.clone(), None);
        p.pubblica_stato();
        p.collegato(false);
        p.campioni(&[campione("giri", Some(1.0), Stato::Ok)]);
        assert!(
            f.tutti().is_empty(),
            "nulla esce finché il broker non risponde"
        );
        assert!(p.stato().scartati >= 1);
        // Alla connessione parte l'ultimo stato trattenuto, una volta sola.
        nucleo.imposta_stato(StatoMqtt::Connesso);
        nucleo.ripubblica();
        let st = f.su(&p.argomenti().stato());
        assert_eq!(st.len(), 1);
        assert!(st[0].1.contains("\"online\"") && st[0].3);
        // Coda piena durante il rinvio: si ritenta.
        f.piena.store(true, Ordering::Relaxed);
        nucleo.ripubblica();
        assert!(nucleo.da_rinviare.load(Ordering::Relaxed));
        f.piena.store(false, Ordering::Relaxed);
        nucleo.ripubblica();
        assert!(!nucleo.da_rinviare.load(Ordering::Relaxed));
    }

    #[test]
    fn diagnosi_trattenuta_senza_vin() {
        let f = Finta::default();
        let p = MqttPublisher::con_coda(&config(), Box::new(f.clone()));
        let report = DiagnosisReport {
            vin: Some("VF3ABCDEFGHIJKLMN".into()),
            protocollo: "ISO 15765-4".into(),
            ..Default::default()
        };
        p.diagnosi(&report);
        let m = f.su(&p.argomenti().diagnosi());
        assert_eq!(m.len(), 1);
        assert!(m[0].3);
        assert!(m[0].1.contains("ISO 15765-4"));
        assert!(!m[0].1.contains("VF3ABCDEFGHIJKLMN"));
        assert!(!m[0].1.to_lowercase().contains("\"vin\""));
    }

    #[test]
    fn discovery_una_volta_per_chiave_e_trattenuta() {
        let f = Finta::default();
        let mut c = config();
        c.discovery_home_assistant = true;
        let p = MqttPublisher::con_coda(&c, Box::new(f.clone()));
        let a = p.argomenti().clone();
        for _ in 0..3 {
            p.campioni(&[campione("giri", Some(1.0), Stato::Ok)]);
        }
        p.campioni(&[campione("velocita", None, Stato::RispostaMancante)]);
        let d = f.su(&a.discovery_sensore("giri"));
        assert_eq!(d.len(), 1);
        assert!(d[0].3, "la discovery è trattenuta");
        assert!(f.su(&a.discovery_sensore("velocita")).is_empty());
        let v: Value = serde_json::from_str(&d[0].1).unwrap();
        assert_eq!(v["name"], "Giri motore");
    }

    #[test]
    fn eventi_marcatore_e_cancellazione() {
        let f = Finta::default();
        let p = MqttPublisher::con_coda(&config(), Box::new(f.clone()));
        p.collegato(false);
        p.marcatore(&Marker {
            t: 3.0,
            ts: "2026-01-01T10:00:00+01:00".into(),
            etichetta: "Minimo".into(),
        });
        p.cancellazione(
            &EsitoCancellazione::Risposte(vec![EsitoCentralina {
                ecu: "7E8".into(),
                risposta: RispostaCancellazione::Accettata,
            }]),
            "2026-01-01T10:01:00+01:00",
        );
        let e = f.su(&p.argomenti().evento());
        assert_eq!(e.len(), 2);
        assert!(e.iter().all(|m| !m.3));
        let m: Value = serde_json::from_str(&e[0].1).unwrap();
        assert_eq!(
            (m["tipo"].as_str(), m["etichetta"].as_str()),
            (Some("marcatore"), Some("Minimo"))
        );
        let c: Value = serde_json::from_str(&e[1].1).unwrap();
        assert_eq!(c["tipo"], "codici_cancellati");
        assert_eq!(c["esito"], "accettata");
        assert_eq!(c["origine"], "veicolo");
    }

    #[test]
    fn qos_della_configurazione() {
        let f = Finta::default();
        let mut c = config();
        c.qos = 1;
        let p = MqttPublisher::con_coda(&c, Box::new(f.clone()));
        p.campioni(&[campione("giri", Some(1.0), Stato::Ok)]);
        assert!(f.tutti().iter().all(|m| m.2 == 1));
    }
}
