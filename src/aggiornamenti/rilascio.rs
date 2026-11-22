//! Rilascio pubblicato su GitHub, letto dalla risposta di
//! `GET /repos/{proprietario}/{repo}/releases/latest` (REST API, oggetto
//! «Release»), e nomi dei file che l'app si aspetta di trovarvi.

use serde::Deserialize;

use super::ErroreAggiornamento;
use super::versione::{self, Confronto, Versione};

/// Nome del file con le somme SHA-256 dei file del rilascio.
pub const NOME_SOMME: &str = "SHA256SUMS.txt";

/// Piattaforma per cui esiste l'installazione automatica: `windows-x64`
/// solo su Windows a 64 bit (x86-64). Altrove `None`.
pub fn piattaforma_corrente() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows-x64")
    } else {
        None
    }
}

/// Nome dell'eseguibile atteso nel rilascio: `OBDio-<versione>-<piattaforma>.exe`.
pub fn nome_eseguibile(versione: &Versione, piattaforma: &str) -> String {
    format!("OBDio-{versione}-{piattaforma}.exe")
}

/// Nome del pacchetto portatile atteso nel rilascio (solo informativo: l'app
/// scarica l'eseguibile).
pub fn nome_pacchetto(versione: &Versione, piattaforma: &str) -> String {
    format!("OBDio-{versione}-{piattaforma}.zip")
}

/// File allegato a un rilascio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Risorsa {
    pub nome: String,
    /// `browser_download_url`.
    pub indirizzo: String,
    /// Dimensione in byte dichiarata dall'API.
    pub dimensione: u64,
}

/// Rilascio come lo usa l'app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rilascio {
    /// Etichetta git, per esempio `v0.4.0`.
    pub etichetta: String,
    /// Versione letta dall'etichetta; `None` se illeggibile.
    pub versione: Option<Versione>,
    /// Titolo del rilascio (può essere vuoto).
    pub nome: String,
    /// Note di rilascio in testo semplice (Markdown non interpretato).
    pub note: String,
    /// Pagina del rilascio sul sito (`html_url`).
    pub pagina: String,
    /// Data di pubblicazione ISO 8601, se presente.
    pub pubblicato: Option<String>,
    pub bozza: bool,
    pub pre_rilascio: bool,
    pub risorse: Vec<Risorsa>,
}

// Campi dell'oggetto «Release» della REST API di GitHub che servono qui;
// gli altri vengono ignorati.
#[derive(Deserialize)]
struct ReleaseGithub {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<AssetGithub>,
}

#[derive(Deserialize)]
struct AssetGithub {
    name: String,
    browser_download_url: String,
    size: u64,
    /// `uploaded` quando il file è completo; `open` durante il caricamento.
    #[serde(default)]
    state: Option<String>,
}

/// Lunghezza massima delle note mostrate: oltre vengono troncate.
const NOTE_MASSIME: usize = 64 * 1024;

impl Rilascio {
    /// Legge la risposta JSON dell'API.
    pub fn da_json(testo: &str) -> Result<Self, ErroreAggiornamento> {
        let r: ReleaseGithub = serde_json::from_str(testo).map_err(|e| {
            ErroreAggiornamento::RispostaNonValida(format!("JSON del rilascio non leggibile: {e}"))
        })?;
        let mut note = r.body.unwrap_or_default().replace("\r\n", "\n");
        if note.len() > NOTE_MASSIME {
            let mut fine = NOTE_MASSIME;
            while !note.is_char_boundary(fine) {
                fine -= 1;
            }
            note.truncate(fine);
            note.push_str("\n…");
        }
        Ok(Rilascio {
            versione: Versione::leggi(&r.tag_name),
            etichetta: r.tag_name,
            nome: r.name.unwrap_or_default(),
            note,
            pagina: r.html_url,
            pubblicato: r.published_at,
            bozza: r.draft,
            pre_rilascio: r.prerelease,
            risorse: r
                .assets
                .into_iter()
                .filter(|a| a.state.as_deref().is_none_or(|s| s == "uploaded"))
                .map(|a| Risorsa {
                    nome: a.name,
                    indirizzo: a.browser_download_url,
                    dimensione: a.size,
                })
                .collect(),
        })
    }

    /// Risorsa con esattamente questo nome.
    pub fn risorsa(&self, nome: &str) -> Option<&Risorsa> {
        self.risorse.iter().find(|r| r.nome == nome)
    }

    /// Eseguibile per la piattaforma, se il rilascio lo contiene con il nome
    /// atteso per la sua versione.
    pub fn eseguibile(&self, piattaforma: &str) -> Option<&Risorsa> {
        let v = self.versione.as_ref()?;
        self.risorsa(&nome_eseguibile(v, piattaforma))
    }

    /// Confronto con la versione installata. Bozze e pre-rilasci non sono
    /// mai proposti, anche se l'etichetta sembra più nuova.
    pub fn confronto(&self, installata: &Versione) -> Confronto {
        if self.bozza || self.pre_rilascio {
            return Confronto::PreRilascio;
        }
        versione::confronta(installata, &self.etichetta)
    }

    /// Versione da mostrare: quella letta oppure l'etichetta così com'è.
    pub fn versione_testo(&self) -> String {
        self.versione
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| self.etichetta.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Risposta d'esempio scritta seguendo lo schema documentato della REST
    /// API di GitHub («Get the latest release»), con i campi principali.
    pub(crate) const ESEMPIO: &str = r#"{
  "url": "https://api.github.com/repos/inalto/OBDio/releases/1",
  "html_url": "https://github.com/inalto/OBDio/releases/tag/v0.4.0",
  "assets_url": "https://api.github.com/repos/inalto/OBDio/releases/1/assets",
  "upload_url": "https://uploads.github.com/repos/inalto/OBDio/releases/1/assets{?name,label}",
  "tarball_url": "https://api.github.com/repos/inalto/OBDio/tarball/v0.4.0",
  "zipball_url": "https://api.github.com/repos/inalto/OBDio/zipball/v0.4.0",
  "id": 1,
  "node_id": "MDc6UmVsZWFzZTE=",
  "tag_name": "v0.4.0",
  "target_commitish": "main",
  "name": "OBDio 0.4.0",
  "body": "Novità:\r\n- controllo degli aggiornamenti\r\n- [pagina](https://example.com)",
  "draft": false,
  "prerelease": false,
  "created_at": "2026-12-01T09:00:00Z",
  "published_at": "2026-12-01T10:00:00Z",
  "author": {"login": "inalto", "id": 1, "type": "User", "site_admin": false},
  "assets": [
    {
      "url": "https://api.github.com/repos/inalto/OBDio/releases/assets/10",
      "browser_download_url": "https://github.com/inalto/OBDio/releases/download/v0.4.0/OBDio-0.4.0-windows-x64.zip",
      "id": 10, "node_id": "MDEyOlJlbGVhc2VBc3NldDEw",
      "name": "OBDio-0.4.0-windows-x64.zip", "label": "",
      "state": "uploaded", "content_type": "application/zip",
      "size": 5242880, "download_count": 3,
      "created_at": "2026-12-01T09:30:00Z", "updated_at": "2026-12-01T09:31:00Z",
      "uploader": {"login": "inalto", "id": 1}
    },
    {
      "url": "https://api.github.com/repos/inalto/OBDio/releases/assets/11",
      "browser_download_url": "https://github.com/inalto/OBDio/releases/download/v0.4.0/OBDio-0.4.0-windows-x64.exe",
      "id": 11, "node_id": "MDEyOlJlbGVhc2VBc3NldDEx",
      "name": "OBDio-0.4.0-windows-x64.exe", "label": null,
      "state": "uploaded", "content_type": "application/x-msdownload",
      "size": 12582912, "download_count": 5,
      "created_at": "2026-12-01T09:30:00Z", "updated_at": "2026-12-01T09:31:00Z",
      "uploader": null
    },
    {
      "url": "https://api.github.com/repos/inalto/OBDio/releases/assets/12",
      "browser_download_url": "https://github.com/inalto/OBDio/releases/download/v0.4.0/SHA256SUMS.txt",
      "id": 12, "node_id": "MDEyOlJlbGVhc2VBc3NldDEy",
      "name": "SHA256SUMS.txt", "label": "",
      "state": "uploaded", "content_type": "text/plain",
      "size": 190, "download_count": 5,
      "created_at": "2026-12-01T09:30:00Z", "updated_at": "2026-12-01T09:31:00Z"
    },
    {
      "url": "https://api.github.com/repos/inalto/OBDio/releases/assets/13",
      "browser_download_url": "https://github.com/inalto/OBDio/releases/download/v0.4.0/incompleto.bin",
      "id": 13, "name": "incompleto.bin", "state": "open", "size": 0
    }
  ]
}"#;

    #[test]
    fn lettura_della_risposta_d_esempio() {
        let r = Rilascio::da_json(ESEMPIO).unwrap();
        assert_eq!(r.etichetta, "v0.4.0");
        assert_eq!(r.versione_testo(), "0.4.0");
        assert_eq!(r.nome, "OBDio 0.4.0");
        assert!(r.note.contains("- controllo degli aggiornamenti\n"));
        assert!(!r.note.contains('\r'));
        assert_eq!(
            r.pagina,
            "https://github.com/inalto/OBDio/releases/tag/v0.4.0"
        );
        assert_eq!(r.pubblicato.as_deref(), Some("2026-12-01T10:00:00Z"));
        // Il file ancora in caricamento («open») non conta.
        assert_eq!(r.risorse.len(), 3);
        let exe = r.eseguibile("windows-x64").unwrap();
        assert_eq!(exe.dimensione, 12_582_912);
        assert!(exe.indirizzo.ends_with("/OBDio-0.4.0-windows-x64.exe"));
        assert!(r.risorsa(NOME_SOMME).is_some());
        assert!(r.eseguibile("linux-x64").is_none());
        let installata = Versione::leggi("0.3.0").unwrap();
        assert_eq!(r.confronto(&installata), Confronto::PiuNuova);
    }

    #[test]
    fn bozze_e_pre_rilasci_non_sono_proposti() {
        let installata = Versione::leggi("0.3.0").unwrap();
        let bozza = ESEMPIO.replace("\"draft\": false", "\"draft\": true");
        assert_eq!(
            Rilascio::da_json(&bozza).unwrap().confronto(&installata),
            Confronto::PreRilascio
        );
        let pre = ESEMPIO.replace("\"prerelease\": false", "\"prerelease\": true");
        assert_eq!(
            Rilascio::da_json(&pre).unwrap().confronto(&installata),
            Confronto::PreRilascio
        );
        let rc = ESEMPIO.replace("\"tag_name\": \"v0.4.0\"", "\"tag_name\": \"v0.4.0-rc1\"");
        assert_eq!(
            Rilascio::da_json(&rc).unwrap().confronto(&installata),
            Confronto::PreRilascio
        );
    }

    #[test]
    fn etichetta_illeggibile_e_risorsa_mancante() {
        let installata = Versione::leggi("0.3.0").unwrap();
        let strana = ESEMPIO.replace("\"tag_name\": \"v0.4.0\"", "\"tag_name\": \"ultima\"");
        let r = Rilascio::da_json(&strana).unwrap();
        assert_eq!(r.confronto(&installata), Confronto::Sconosciuta);
        assert_eq!(r.versione_testo(), "ultima");
        // Senza versione leggibile non si sceglie alcun eseguibile.
        assert!(r.eseguibile("windows-x64").is_none());

        // L'etichetta dice 0.5.0 ma i file sono della 0.4.0: nessun eseguibile.
        let diversa = ESEMPIO.replace("\"tag_name\": \"v0.4.0\"", "\"tag_name\": \"v0.5.0\"");
        assert!(
            Rilascio::da_json(&diversa)
                .unwrap()
                .eseguibile("windows-x64")
                .is_none()
        );

        let senza = r#"{"tag_name":"v0.4.0","html_url":"https://github.com/x/y","assets":[]}"#;
        let r = Rilascio::da_json(senza).unwrap();
        assert!(r.eseguibile("windows-x64").is_none());
        assert!(r.note.is_empty() && r.nome.is_empty() && r.pubblicato.is_none());
    }

    #[test]
    fn risposte_non_valide() {
        for t in [
            "",
            "<html>rate limited</html>",
            "{}",
            "[]",
            r#"{"tag_name": 4, "html_url": "x"}"#,
            r#"{"tag_name":"v1.0.0","html_url":"x","assets":[{"name":"a"}]}"#,
            r#"{"tag_name":"v1.0.0","html_url":"x","assets":[{"name":"a","browser_download_url":"u","size":-1}]}"#,
        ] {
            assert!(
                matches!(
                    Rilascio::da_json(t),
                    Err(ErroreAggiornamento::RispostaNonValida(_))
                ),
                "{t}"
            );
        }
    }

    #[test]
    fn note_lunghissime_troncate() {
        let lunga = format!(
            r#"{{"tag_name":"v1.0.0","html_url":"x","body":"{}"}}"#,
            "è".repeat(NOTE_MASSIME)
        );
        let r = Rilascio::da_json(&lunga).unwrap();
        assert!(r.note.len() <= NOTE_MASSIME + 4);
        assert!(r.note.ends_with('…'));
    }

    #[test]
    fn nomi_attesi() {
        let v = Versione::leggi("v1.2.3").unwrap();
        assert_eq!(
            nome_eseguibile(&v, "windows-x64"),
            "OBDio-1.2.3-windows-x64.exe"
        );
        assert_eq!(
            nome_pacchetto(&v, "windows-x64"),
            "OBDio-1.2.3-windows-x64.zip"
        );
        if cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!(piattaforma_corrente(), Some("windows-x64"));
        }
    }
}
