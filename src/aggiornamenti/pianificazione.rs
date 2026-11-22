//! Preferenze del controllo degli aggiornamenti (`aggiornamenti.json`) e
//! regole su quando controllare e che cosa mostrare. Il tempo arriva sempre
//! da fuori (secondi Unix), così le regole si verificano con un orologio
//! finto.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::versione::Versione;
use crate::config;

/// Nome del file nella cartella di configurazione.
pub const FILE_CONFIGURAZIONE: &str = "aggiornamenti.json";
/// Intervallo minimo fra due controlli automatici.
pub const INTERVALLO_S: i64 = 24 * 3600;
/// Un controllo «nel futuro» oltre questo margine vuol dire che l'orologio
/// è stato spostato indietro: il controllo torna dovuto.
const MARGINE_OROLOGIO_S: i64 = 3600;
/// Attesa massima accettata da un limite di richieste (oltre: un giorno).
const ATTESA_MASSIMA_S: i64 = 24 * 3600;

/// Contenuto di `aggiornamenti.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfigAggiornamenti {
    /// Controllo automatico all'avvio (al più una volta ogni 24 ore).
    pub automatico: bool,
    /// Ultimo tentativo di controllo, riuscito o no (secondi Unix).
    pub ultimo_controllo: Option<i64>,
    /// Esito dell'ultimo controllo, in breve, da mostrare.
    pub ultimo_esito: Option<String>,
    /// Ultima versione vista su GitHub.
    pub ultima_versione_vista: Option<String>,
    /// Versione che l'utente ha scelto di ignorare.
    pub ignora_versione: Option<String>,
    /// GitHub ha chiesto di non riprovare prima di questo momento.
    pub non_prima_di: Option<i64>,
}

impl Default for ConfigAggiornamenti {
    fn default() -> Self {
        ConfigAggiornamenti {
            automatico: true,
            ultimo_controllo: None,
            ultimo_esito: None,
            ultima_versione_vista: None,
            ignora_versione: None,
            non_prima_di: None,
        }
    }
}

impl ConfigAggiornamenti {
    /// Legge il file; se manca, i valori predefiniti (controllo automatico
    /// acceso).
    pub fn carica(cartella: &Path) -> Result<Self, String> {
        Ok(config::carica(cartella, FILE_CONFIGURAZIONE)?.unwrap_or_default())
    }

    pub fn salva(&self, cartella: &Path) -> Result<(), String> {
        config::salva(cartella, FILE_CONFIGURAZIONE, self).map(|_| ())
    }

    /// Vero se il controllo automatico va fatto ora.
    pub fn controllo_dovuto(&self, ora: i64) -> bool {
        if !self.automatico {
            return false;
        }
        if let Some(t) = self.non_prima_di
            && ora < t
            && t - ora <= ATTESA_MASSIMA_S
        {
            return false;
        }
        match self.ultimo_controllo {
            None => true,
            // Orologio spostato indietro: meglio controllare che restare
            // fermi per sempre.
            Some(t) if t > ora + MARGINE_OROLOGIO_S => true,
            Some(t) => ora - t >= INTERVALLO_S,
        }
    }

    /// Vero se il limite di richieste di GitHub impedisce ancora un
    /// controllo (anche manuale).
    pub fn limitato(&self, ora: i64) -> Option<i64> {
        self.non_prima_di
            .filter(|t| ora < *t && *t - ora <= ATTESA_MASSIMA_S)
    }

    /// Vero se l'utente ha chiesto di ignorare proprio questa versione. Una
    /// versione più nuova di quella ignorata viene di nuovo proposta.
    pub fn ignorata(&self, versione: &Versione) -> bool {
        self.ignora_versione
            .as_deref()
            .and_then(Versione::leggi)
            .is_some_and(|i| *versione <= i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORA: i64 = 1_795_000_000;

    #[test]
    fn predefinito_acceso_e_dovuto() {
        let c = ConfigAggiornamenti::default();
        assert!(c.automatico);
        assert!(c.controllo_dovuto(ORA));
    }

    #[test]
    fn al_piu_una_volta_al_giorno() {
        let mut c = ConfigAggiornamenti {
            ultimo_controllo: Some(ORA),
            ..Default::default()
        };
        assert!(!c.controllo_dovuto(ORA));
        assert!(!c.controllo_dovuto(ORA + 3600));
        assert!(!c.controllo_dovuto(ORA + INTERVALLO_S - 1));
        assert!(c.controllo_dovuto(ORA + INTERVALLO_S));
        assert!(c.controllo_dovuto(ORA + 10 * INTERVALLO_S));
        // Spento: mai.
        c.automatico = false;
        assert!(!c.controllo_dovuto(ORA + 10 * INTERVALLO_S));
    }

    #[test]
    fn orologio_spostato_indietro() {
        let c = ConfigAggiornamenti {
            ultimo_controllo: Some(ORA + 30 * INTERVALLO_S),
            ..Default::default()
        };
        assert!(c.controllo_dovuto(ORA));
        // Un piccolo scarto (fuso, sincronizzazione) non fa ripetere.
        let c = ConfigAggiornamenti {
            ultimo_controllo: Some(ORA + 600),
            ..Default::default()
        };
        assert!(!c.controllo_dovuto(ORA));
    }

    #[test]
    fn limite_di_richieste_rispettato() {
        let c = ConfigAggiornamenti {
            ultimo_controllo: Some(ORA - 2 * INTERVALLO_S),
            non_prima_di: Some(ORA + 1800),
            ..Default::default()
        };
        assert!(!c.controllo_dovuto(ORA));
        assert_eq!(c.limitato(ORA), Some(ORA + 1800));
        assert!(c.controllo_dovuto(ORA + 1800));
        assert_eq!(c.limitato(ORA + 1800), None);
        // Un'attesa assurda (oltre un giorno) non blocca per sempre.
        let c = ConfigAggiornamenti {
            non_prima_di: Some(ORA + 400 * INTERVALLO_S),
            ..Default::default()
        };
        assert!(c.controllo_dovuto(ORA));
        assert_eq!(c.limitato(ORA), None);
    }

    #[test]
    fn versione_ignorata() {
        let v = |t: &str| Versione::leggi(t).unwrap();
        let mut c = ConfigAggiornamenti::default();
        assert!(!c.ignorata(&v("0.4.0")));
        c.ignora_versione = Some("0.4.0".into());
        assert!(c.ignorata(&v("0.4.0")));
        assert!(c.ignorata(&v("v0.4.0")));
        // Una versione più nuova di quella ignorata torna a essere proposta.
        assert!(!c.ignorata(&v("0.4.1")));
        assert!(!c.ignorata(&v("0.5.0")));
        c.ignora_versione = Some("spazzatura".into());
        assert!(!c.ignorata(&v("0.4.0")));
    }

    #[test]
    fn file_letto_e_scritto() {
        let dir = std::env::temp_dir().join(format!("obdio_agg_conf_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            ConfigAggiornamenti::carica(&dir).unwrap(),
            ConfigAggiornamenti::default()
        );
        let c = ConfigAggiornamenti {
            automatico: false,
            ultimo_controllo: Some(ORA),
            ignora_versione: Some("0.4.0".into()),
            ..Default::default()
        };
        c.salva(&dir).unwrap();
        assert_eq!(ConfigAggiornamenti::carica(&dir).unwrap(), c);
        // Un file parziale prende i valori predefiniti per i campi mancanti.
        std::fs::write(
            dir.join(FILE_CONFIGURAZIONE),
            r#"{"ignora_versione":"1.0.0"}"#,
        )
        .unwrap();
        let p = ConfigAggiornamenti::carica(&dir).unwrap();
        assert!(p.automatico);
        assert_eq!(p.ignora_versione.as_deref(), Some("1.0.0"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
