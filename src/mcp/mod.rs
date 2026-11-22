//! Server MCP (Model Context Protocol) per assistenti AI.
//!
//! Mentre l'app è aperta, e solo se l'utente lo attiva, un assistente AI
//! (Claude Desktop, Claude Code, …) può collegarsi a
//! `http://127.0.0.1:<porta>/mcp` e leggere ciò che l'app sta mostrando:
//! stato del collegamento, sensori, diagnosi, sessioni registrate. Con il
//! livello «Lettura e azioni» può anche avviare la lettura, registrare,
//! aggiungere marcatori e chiedere una nuova diagnosi.
//!
//! # Strati
//!
//! - [`archivio`]: stato condiviso che il thread di lavoro aggiorna a ogni
//!   evento; il server legge solo da qui e non tocca mai l'adattatore;
//! - [`strumenti`]: gli strumenti («tools») offerti all'assistente;
//! - [`protocollo`]: JSON-RPC e MCP (revisione 2026-07-28, senza stato, e le
//!   revisioni 2025-06-18 e 2025-11-25 con `initialize`);
//! - [`http`]: il trasporto «Streamable HTTP» su `127.0.0.1`, con token,
//!   controllo di `Origin` e `Host`, limiti di dimensione e di tempo.
//!
//! # Sicurezza
//!
//! Spento finché l'utente non lo accende; in ascolto solo su `127.0.0.1`;
//! ogni richiesta porta un token casuale di 256 bit; il VIN non esce mai; la
//! cancellazione dei codici guasto (servizio 04) **non è raggiungibile**: le
//! azioni passano dal tipo [`Azione`], che non ha una variante per farlo.
//! Vedi `docs/MCP.md`.

pub mod archivio;
pub mod http;
pub mod protocollo;
pub mod strumenti;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::worker::{Comando, Evento};

pub use archivio::Archivio;

/// Nome del file di configurazione nella cartella di configurazione.
pub const FILE_CONFIGURAZIONE: &str = "mcp.json";
/// Porta proposta la prima volta.
pub const PORTA_PREDEFINITA: u16 = 8765;
/// Percorso dell'unico punto d'accesso.
pub const PERCORSO: &str = "/mcp";
/// Byte casuali del token (256 bit), scritti in esadecimale.
const BYTE_TOKEN: usize = 32;
/// Lunghezza minima accettata per un token scritto a mano nel file (128 bit
/// in esadecimale).
pub const LUNGHEZZA_MINIMA_TOKEN: usize = 32;
/// Intervallo minimo fra due stati comunicati all'interfaccia.
const INTERVALLO_STATO: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Configurazione
// ---------------------------------------------------------------------------

/// Che cosa può fare l'assistente.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Livello {
    /// Solo strumenti che leggono ciò che l'app ha già in memoria o su disco.
    #[default]
    SolaLettura,
    /// Anche avviare la lettura, registrare, marcare, rileggere la diagnosi.
    LetturaEAzioni,
}

impl Livello {
    pub fn etichetta(self) -> &'static str {
        match self {
            Livello::SolaLettura => "Sola lettura",
            Livello::LetturaEAzioni => "Lettura e azioni",
        }
    }
}

/// Impostazioni del server. Il token sta nel file in chiaro: chi può leggere
/// il file può collegarsi (vedi `docs/MCP.md`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub attivo: bool,
    /// Porta su `127.0.0.1`. Zero: una porta libera scelta dal sistema (solo
    /// per le verifiche; l'interfaccia non lo propone).
    pub porta: u16,
    /// Vuoto finché il server non viene attivato la prima volta.
    pub token: String,
    pub livello: Livello,
}

impl Default for McpConfig {
    fn default() -> Self {
        McpConfig {
            attivo: false,
            porta: PORTA_PREDEFINITA,
            token: String::new(),
            livello: Livello::SolaLettura,
        }
    }
}

impl fmt::Debug for McpConfig {
    /// Il token non compare mai nei log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpConfig")
            .field("attivo", &self.attivo)
            .field("porta", &self.porta)
            .field(
                "token",
                &if self.token.is_empty() {
                    ""
                } else {
                    "********"
                },
            )
            .field("livello", &self.livello)
            .finish()
    }
}

impl McpConfig {
    /// Legge `mcp.json`; se manca restituisce le impostazioni predefinite.
    pub fn carica(cartella: &Path) -> Result<Self, String> {
        Ok(config::carica(cartella, FILE_CONFIGURAZIONE)?.unwrap_or_default())
    }

    pub fn salva(&self, cartella: &Path) -> Result<(), String> {
        config::salva(cartella, FILE_CONFIGURAZIONE, self).map(|_| ())
    }

    /// Genera il token se manca. `Ok(true)` se ne è stato creato uno nuovo.
    pub fn assicura_token(&mut self) -> Result<bool, String> {
        if token_accettabile(&self.token) {
            return Ok(false);
        }
        self.token = nuovo_token()?;
        Ok(true)
    }

    /// Indirizzo da dare al client, con la porta indicata.
    pub fn indirizzo_con(porta: u16) -> String {
        format!("http://127.0.0.1:{porta}{PERCORSO}")
    }

    /// Indirizzo da dare al client secondo la configurazione.
    pub fn indirizzo(&self) -> String {
        Self::indirizzo_con(self.porta)
    }

    /// Messaggio da mostrare se la porta non è utilizzabile dall'interfaccia.
    pub fn problema_porta(&self) -> Option<String> {
        (self.porta < 1024).then(|| {
            "La porta deve essere un numero da 1024 a 65535 (quella proposta è 8765).".to_string()
        })
    }
}

/// Vero se il token è abbastanza lungo e fatto solo di caratteri che possono
/// stare in un'intestazione HTTP senza ambiguità.
pub fn token_accettabile(token: &str) -> bool {
    token.len() >= LUNGHEZZA_MINIMA_TOKEN
        && token.len() <= 256
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
}

/// Token casuale di 256 bit dal generatore del sistema operativo, in
/// esadecimale (64 caratteri).
pub fn nuovo_token() -> Result<String, String> {
    let mut byte = [0u8; BYTE_TOKEN];
    getrandom::fill(&mut byte)
        .map_err(|e| format!("il sistema non ha fornito numeri casuali per il token: {e}"))?;
    Ok(byte.iter().map(|b| format!("{b:02x}")).collect())
}

/// Token con i soli ultimi quattro caratteri in chiaro.
pub fn token_mascherato(token: &str) -> String {
    if token.is_empty() {
        return "non ancora generato".into();
    }
    let coda: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}{coda}", "•".repeat(12))
}

/// Blocco JSON per la configurazione di un client MCP (per esempio
/// `.mcp.json` di Claude Code).
pub fn frammento_json(indirizzo: &str, token: &str) -> String {
    // Scritto a mano per tenere l'ordine in cui lo si legge: tipo, indirizzo,
    // intestazioni.
    let testo = |s: &str| serde_json::Value::from(s).to_string();
    [
        "{".to_string(),
        "  \"mcpServers\": {".to_string(),
        "    \"obdio\": {".to_string(),
        "      \"type\": \"http\",".to_string(),
        format!("      \"url\": {},", testo(indirizzo)),
        "      \"headers\": {".to_string(),
        format!(
            "        \"Authorization\": {}",
            testo(&format!("Bearer {token}"))
        ),
        "      }".to_string(),
        "    }".to_string(),
        "  }".to_string(),
        "}".to_string(),
    ]
    .join("\n")
}

/// Riga di comando equivalente per Claude Code.
pub fn comando_claude(indirizzo: &str, token: &str) -> String {
    format!(
        "claude mcp add --transport http obdio {indirizzo} --header \"Authorization: Bearer \
         {token}\""
    )
}

// ---------------------------------------------------------------------------
// Stato
// ---------------------------------------------------------------------------

/// Stato del server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatoMcp {
    Disattivato,
    /// In ascolto su `127.0.0.1` alla porta indicata (quella effettiva).
    InAscolto(u16),
    Errore(String),
}

impl StatoMcp {
    pub fn testo(&self) -> String {
        match self {
            StatoMcp::Disattivato => "Disattivato".into(),
            StatoMcp::InAscolto(p) => format!("In ascolto su 127.0.0.1:{p}"),
            StatoMcp::Errore(e) => format!("Errore: {e}"),
        }
    }
}

/// Fotografia dello stato, economica da copiare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatoMcpInfo {
    pub stato: StatoMcp,
    /// Richieste autenticate servite.
    pub richieste: u64,
    /// Richieste respinte prima di essere lette (token, `Origin`, `Host`).
    pub rifiutate: u64,
    /// Nome e versione dell'ultimo client che si è presentato.
    pub ultimo_client: Option<String>,
    pub ultimo_errore: Option<String>,
}

impl StatoMcpInfo {
    pub fn disattivato() -> Self {
        StatoMcpInfo {
            stato: StatoMcp::Disattivato,
            richieste: 0,
            rifiutate: 0,
            ultimo_client: None,
            ultimo_errore: None,
        }
    }

    fn errore(e: String) -> Self {
        StatoMcpInfo {
            stato: StatoMcp::Errore(e.clone()),
            ultimo_errore: Some(e),
            ..Self::disattivato()
        }
    }
}

// ---------------------------------------------------------------------------
// Azioni
// ---------------------------------------------------------------------------

/// Le sole azioni che un assistente può chiedere al thread di lavoro.
///
/// È un tipo a parte, e non [`Comando`], di proposito: qui **non esiste** la
/// cancellazione dei codici guasto, né collegare o scollegare l'adattatore,
/// né uscire dall'app. Ciò che non è in questo elenco non è raggiungibile via
/// MCP, qualunque cosa arrivi dalla rete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Azione {
    LeggiDiagnosi,
    AvviaLettura(Vec<String>),
    FermaLettura,
    Marcatore(String),
    AvviaRegistrazione,
    FermaRegistrazione,
}

impl Azione {
    /// Il comando corrispondente per il thread di lavoro.
    pub fn in_comando(self) -> Comando {
        match self {
            Azione::LeggiDiagnosi => Comando::LeggiDiagnosi,
            Azione::AvviaLettura(chiavi) => Comando::AvviaPolling {
                chiavi,
                prioritarie: Vec::new(),
            },
            Azione::FermaLettura => Comando::FermaPolling,
            Azione::Marcatore(testo) => Comando::Marcatore(testo),
            Azione::AvviaRegistrazione => Comando::AvviaRegistrazione,
            Azione::FermaRegistrazione => Comando::FermaRegistrazione,
        }
    }
}

// ---------------------------------------------------------------------------
// Lato thread di lavoro
// ---------------------------------------------------------------------------

/// Ciò che il thread di lavoro possiede del server MCP: l'archivio (sempre
/// aggiornato, anche a server spento, così attivandolo a metà sessione i
/// dati ci sono già), la coda delle azioni e il server, quando è acceso.
pub struct Ospite {
    archivio: Arc<Archivio>,
    cartella_sessioni: PathBuf,
    tx: Sender<Azione>,
    rx: Receiver<Azione>,
    server: Option<http::Server>,
    /// Ultimo stato comunicato all'interfaccia e quando.
    inviato: Option<(StatoMcpInfo, Instant)>,
}

impl Ospite {
    pub fn new(cartella_sessioni: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        Ospite {
            archivio: Arc::new(Archivio::default()),
            cartella_sessioni,
            tx,
            rx,
            server: None,
            inviato: None,
        }
    }

    pub fn archivio(&self) -> &Arc<Archivio> {
        &self.archivio
    }

    /// Aggiorna l'archivio con un evento del thread di lavoro. Non blocca:
    /// una sezione critica breve, nessun I/O.
    pub fn osserva(&self, e: &Evento) {
        self.archivio.osserva(e);
    }

    /// Vero se il server è in ascolto.
    pub fn attivo(&self) -> bool {
        self.server.is_some()
    }

    /// Avvia, riavvia o ferma il server secondo la configurazione. Attende la
    /// chiusura del precedente per un tempo limitato.
    pub fn configura(&mut self, config: &McpConfig) -> StatoMcpInfo {
        self.ferma();
        self.inviato = None;
        if !config.attivo {
            return StatoMcpInfo::disattivato();
        }
        match http::Server::avvia(
            config,
            self.archivio.clone(),
            self.tx.clone(),
            self.cartella_sessioni.clone(),
        ) {
            Ok(s) => {
                let info = s.stato();
                self.server = Some(s);
                self.inviato = Some((info.clone(), Instant::now()));
                info
            }
            Err(e) => StatoMcpInfo::errore(e),
        }
    }

    /// Lo stato da comunicare all'interfaccia, se è cambiato (al più due
    /// volte al secondo).
    pub fn stato_cambiato(&mut self) -> Option<StatoMcpInfo> {
        let info = self.server.as_ref()?.stato();
        let da_inviare = match &self.inviato {
            None => true,
            Some((u, t)) => *u != info && t.elapsed() >= INTERVALLO_STATO,
        };
        if da_inviare {
            self.inviato = Some((info.clone(), Instant::now()));
            Some(info)
        } else {
            None
        }
    }

    /// Prossima azione chiesta da un assistente, già tradotta in comando.
    pub fn prossima_azione(&self) -> Option<Comando> {
        self.rx.try_recv().ok().map(Azione::in_comando)
    }

    /// Spegne il server (attesa limitata, non resta mai appeso) e scarta le
    /// azioni ancora in coda.
    pub fn ferma(&mut self) {
        if let Some(s) = self.server.take() {
            s.ferma();
        }
        while self.rx.try_recv().is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurazione_andata_e_ritorno() {
        let dir = std::env::temp_dir().join(format!("obdio_mcp_config_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Senza file: spento, sola lettura, nessun token.
        let predefinita = McpConfig::carica(&dir).unwrap();
        assert_eq!(predefinita, McpConfig::default());
        assert!(!predefinita.attivo);
        assert_eq!(predefinita.livello, Livello::SolaLettura);
        assert!(predefinita.token.is_empty());

        let mut c = McpConfig {
            attivo: true,
            porta: 9001,
            livello: Livello::LetturaEAzioni,
            ..Default::default()
        };
        assert!(c.assicura_token().unwrap());
        // Un token valido non viene sostituito.
        let token = c.token.clone();
        assert!(!c.assicura_token().unwrap());
        assert_eq!(c.token, token);
        c.salva(&dir).unwrap();
        assert_eq!(McpConfig::carica(&dir).unwrap(), c);
        let testo = std::fs::read_to_string(dir.join(FILE_CONFIGURAZIONE)).unwrap();
        assert!(
            testo.contains("\"livello\": \"lettura_e_azioni\""),
            "{testo}"
        );

        // Un file di una versione futura con campi in più o in meno si legge.
        std::fs::write(dir.join(FILE_CONFIGURAZIONE), r#"{"attivo":true}"#).unwrap();
        let parziale = McpConfig::carica(&dir).unwrap();
        assert!(parziale.attivo);
        assert_eq!(parziale.porta, PORTA_PREDEFINITA);
        assert_eq!(parziale.livello, Livello::SolaLettura);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn token_casuale_lungo_e_mai_nei_log() {
        let a = nuovo_token().unwrap();
        let b = nuovo_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|x| x.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert!(token_accettabile(&a));
        assert!(!token_accettabile("corto"));
        assert!(!token_accettabile(&"x y".repeat(20)));
        assert!(!token_accettabile(&"a\r\n".repeat(20)));

        let c = McpConfig {
            token: a.clone(),
            ..Default::default()
        };
        let log = format!("{c:?}");
        assert!(!log.contains(&a), "{log}");
        assert!(log.contains("********"));

        let m = token_mascherato(&a);
        assert!(m.ends_with(&a[60..]));
        assert!(!m.contains(&a[..56]));
        assert_eq!(token_mascherato(""), "non ancora generato");
    }

    #[test]
    fn frammenti_per_il_client() {
        let j = frammento_json("http://127.0.0.1:8765/mcp", "abc");
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let s = &v["mcpServers"]["obdio"];
        assert_eq!(s["type"], "http");
        assert_eq!(s["url"], "http://127.0.0.1:8765/mcp");
        assert_eq!(s["headers"]["Authorization"], "Bearer abc");
        assert!(j.find("\"type\"").unwrap() < j.find("\"url\"").unwrap());
        assert!(j.find("\"url\"").unwrap() < j.find("\"headers\"").unwrap());
        assert!(j.contains("\n      \"url\": \"http://127.0.0.1:8765/mcp\",\n"));
        assert_eq!(j.lines().count(), 11);
        assert_eq!(
            comando_claude("http://127.0.0.1:8765/mcp", "abc"),
            "claude mcp add --transport http obdio http://127.0.0.1:8765/mcp --header \
             \"Authorization: Bearer abc\""
        );
        assert_eq!(
            McpConfig::default().indirizzo(),
            "http://127.0.0.1:8765/mcp"
        );
        assert!(McpConfig::default().problema_porta().is_none());
        let bassa = McpConfig {
            porta: 80,
            ..Default::default()
        };
        assert!(bassa.problema_porta().is_some());
    }

    #[test]
    fn nessuna_azione_cancella_i_codici() {
        // Tutte le varianti di `Azione`: se ne nasce una nuova, questo elenco
        // non compila più finché non la si aggiunge qui (vedi `tutte`).
        for a in tutte() {
            let nome = format!("{a:?}");
            let c = a.in_comando();
            assert!(
                !matches!(
                    c,
                    Comando::CancellaDtc(_)
                        | Comando::Esci
                        | Comando::Collega(_)
                        | Comando::Scollega
                        | Comando::ConfiguraMcp(_)
                        | Comando::ConfiguraMqtt(_)
                ),
                "{nome} porta a un comando non ammesso"
            );
        }
    }

    /// Una per variante; il `match` esaustivo obbliga ad aggiornarla.
    fn tutte() -> Vec<Azione> {
        let campione = [
            Azione::LeggiDiagnosi,
            Azione::AvviaLettura(vec!["giri".into()]),
            Azione::FermaLettura,
            Azione::Marcatore("x".into()),
            Azione::AvviaRegistrazione,
            Azione::FermaRegistrazione,
        ];
        for a in &campione {
            match a {
                Azione::LeggiDiagnosi
                | Azione::AvviaLettura(_)
                | Azione::FermaLettura
                | Azione::Marcatore(_)
                | Azione::AvviaRegistrazione
                | Azione::FermaRegistrazione => {}
            }
        }
        campione.to_vec()
    }

    #[test]
    fn ospite_spento_non_ha_azioni_ne_stato() {
        let mut o = Ospite::new(std::env::temp_dir());
        assert!(!o.attivo());
        assert!(o.prossima_azione().is_none());
        assert!(o.stato_cambiato().is_none());
        assert_eq!(
            o.configura(&McpConfig::default()),
            StatoMcpInfo::disattivato()
        );
        // Attivo ma senza token: errore chiaro, nessun server.
        let info = o.configura(&McpConfig {
            attivo: true,
            porta: 0,
            ..Default::default()
        });
        assert!(matches!(info.stato, StatoMcp::Errore(_)), "{info:?}");
        assert!(!o.attivo());
    }
}
