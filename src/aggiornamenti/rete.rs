//! Richieste HTTPS verso GitHub: l'ultimo rilascio dall'API e i file da
//! scaricare. Nessun cookie, nessuna credenziale, nessun dato dell'utente:
//! solo `User-Agent: OBDio/<versione>` e le intestazioni `Accept`.
//!
//! Regole, tutte qui:
//! - solo HTTPS con certificato verificato dal sistema (SChannel su
//!   Windows); l'HTTP in chiaro esiste solo per le prove, scelto alla
//!   costruzione con [`ImpostazioniRete::per_prove`];
//! - i rinvii (3xx) si seguono a mano, al più [`ImpostazioniRete::massimo_rinvii`],
//!   e solo verso gli host ammessi;
//! - tempi massimi di connessione e di lettura, e un limite di dimensione
//!   per ogni risposta.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use super::ErroreAggiornamento;
use super::rilascio::Rilascio;
use super::somme::Calcolo;

/// Repository pubblico del progetto.
pub const REPOSITORY: &str = "inalto/OBDio";
/// Indirizzo dell'API di GitHub.
pub const API_GITHUB: &str = "https://api.github.com";

/// Dove e come collegarsi. Non è una scelta dell'utente: la versione per
/// l'utente usa sempre [`ImpostazioniRete::github`].
#[derive(Clone, Debug)]
pub struct ImpostazioniRete {
    /// Base dell'API, senza barra finale.
    pub base_api: String,
    /// `proprietario/repository`.
    pub repository: String,
    /// Host ammessi per l'API e i suoi rinvii.
    pub host_api: Vec<String>,
    /// Host ammessi per scaricare i file (anche dopo un rinvio). Una voce che
    /// inizia con «.» ammette i sottodomini.
    pub host_scaricamento: Vec<String>,
    /// HTTP in chiaro ammesso: solo per le prove con un server locale.
    http_in_chiaro: bool,
    pub tempo_connessione: Duration,
    /// Tempo massimo di attesa per ogni lettura dalla rete.
    pub tempo_lettura: Duration,
    /// Tempo massimo complessivo della richiesta all'API.
    pub tempo_api: Duration,
    pub limite_json: u64,
    pub limite_somme: u64,
    pub limite_eseguibile: u64,
    pub massimo_rinvii: u32,
}

impl ImpostazioniRete {
    /// GitHub, come nella versione per l'utente.
    pub fn github() -> Self {
        ImpostazioniRete {
            base_api: API_GITHUB.into(),
            repository: REPOSITORY.into(),
            host_api: vec!["api.github.com".into()],
            // github.com risponde con un rinvio verso un sottodominio di
            // githubusercontent.com, dove stanno i file.
            host_scaricamento: vec!["github.com".into(), ".githubusercontent.com".into()],
            http_in_chiaro: false,
            tempo_connessione: Duration::from_secs(15),
            tempo_lettura: Duration::from_secs(30),
            tempo_api: Duration::from_secs(45),
            limite_json: 2 * 1024 * 1024,
            limite_somme: 64 * 1024,
            limite_eseguibile: 200 * 1024 * 1024,
            massimo_rinvii: 5,
        }
    }

    /// Solo per le prove: un server locale in HTTP in chiaro, per esempio
    /// `http://127.0.0.1:4567`. Ammette come host solo quello della base.
    pub fn per_prove(base: &str) -> Self {
        let host = base
            .parse::<Uri>()
            .ok()
            .and_then(|u| u.host().map(str::to_ascii_lowercase))
            .unwrap_or_default();
        ImpostazioniRete {
            base_api: base.trim_end_matches('/').into(),
            host_api: vec![host.clone()],
            host_scaricamento: vec![host],
            http_in_chiaro: true,
            tempo_connessione: Duration::from_secs(5),
            tempo_lettura: Duration::from_secs(5),
            tempo_api: Duration::from_secs(10),
            ..Self::github()
        }
    }

    /// Vero se l'HTTP in chiaro è ammesso (solo con [`Self::per_prove`]).
    pub fn http_in_chiaro(&self) -> bool {
        self.http_in_chiaro
    }

    /// Indirizzo della richiesta dell'ultimo rilascio.
    pub fn indirizzo_ultimo(&self) -> String {
        format!(
            "{}/repos/{}/releases/latest",
            self.base_api.trim_end_matches('/'),
            self.repository
        )
    }
}

/// Vero se `host` è nell'elenco: uguale a una voce, o sottodominio di una
/// voce che inizia con «.».
pub fn host_ammesso(host: &str, ammessi: &[String]) -> bool {
    let host = host.to_ascii_lowercase();
    ammessi.iter().any(|a| {
        let a = a.to_ascii_lowercase();
        if a.starts_with('.') {
            host.len() > a.len() && host.ends_with(&a)
        } else {
            host == a
        }
    })
}

/// Controlla che l'indirizzo sia HTTPS (o HTTP se ammesso), senza
/// credenziali, su una porta normale e verso un host ammesso.
pub fn controlla_indirizzo(
    indirizzo: &str,
    ammessi: &[String],
    http_in_chiaro: bool,
) -> Result<(), ErroreAggiornamento> {
    let rifiuta = |motivo: &str| {
        Err(ErroreAggiornamento::IndirizzoNonAmmesso(format!(
            "{indirizzo} ({motivo})"
        )))
    };
    let Ok(uri) = indirizzo.parse::<Uri>() else {
        return rifiuta("indirizzo non valido");
    };
    match uri.scheme_str() {
        Some("https") => {}
        Some("http") if http_in_chiaro => {}
        _ => return rifiuta("serve HTTPS"),
    }
    let Some(autorita) = uri.authority() else {
        return rifiuta("host mancante");
    };
    if autorita.as_str().contains('@') {
        return rifiuta("credenziali nell'indirizzo");
    }
    let host = uri.host().unwrap_or_default();
    if !http_in_chiaro && uri.port_u16().is_some_and(|p| p != 443) {
        return rifiuta("porta non ammessa");
    }
    if !host_ammesso(host, ammessi) {
        return rifiuta("host non ammesso");
    }
    Ok(())
}

/// Destinazione di un rinvio, controllata. `posizione` è l'intestazione
/// `Location`: assoluta, relativa alla radice («/…») o senza schema («//…»).
pub fn prossimo_indirizzo(
    attuale: &str,
    posizione: &str,
    ammessi: &[String],
    http_in_chiaro: bool,
) -> Result<String, ErroreAggiornamento> {
    let posizione = posizione.trim();
    let nuovo = if posizione.starts_with("//") {
        let schema = attuale.split_once("://").map(|(s, _)| s).unwrap_or("https");
        format!("{schema}:{posizione}")
    } else if posizione.starts_with('/') {
        let uri: Uri = attuale.parse().map_err(|_| {
            ErroreAggiornamento::IndirizzoNonAmmesso(format!("{attuale} (indirizzo non valido)"))
        })?;
        let (Some(schema), Some(autorita)) = (uri.scheme_str(), uri.authority()) else {
            return Err(ErroreAggiornamento::IndirizzoNonAmmesso(attuale.into()));
        };
        format!("{schema}://{autorita}{posizione}")
    } else {
        posizione.to_string()
    };
    controlla_indirizzo(&nuovo, ammessi, http_in_chiaro)?;
    Ok(nuovo)
}

/// Momento (secondi Unix) dopo il quale GitHub accetta di nuovo richieste,
/// dalle intestazioni `Retry-After` (secondi) o `x-ratelimit-reset`
/// (secondi Unix).
pub fn fine_limite(retry_after: Option<&str>, reset: Option<&str>, ora: i64) -> Option<i64> {
    if let Some(s) = retry_after.and_then(|t| t.trim().parse::<i64>().ok())
        && (0..=86_400).contains(&s)
    {
        return Some(ora + s);
    }
    reset
        .and_then(|t| t.trim().parse::<i64>().ok())
        .filter(|t| *t > ora && *t - ora <= 86_400)
}

/// Secondi Unix adesso.
pub fn adesso() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn errore_rete(e: ureq::Error) -> ErroreAggiornamento {
    match e {
        ureq::Error::Timeout(_) => ErroreAggiornamento::TempoScaduto,
        ureq::Error::Io(ref io) if io.kind() == std::io::ErrorKind::TimedOut => {
            ErroreAggiornamento::TempoScaduto
        }
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            ErroreAggiornamento::NonRaggiungibile(e.to_string())
        }
        ureq::Error::RequireHttpsOnly(u) => ErroreAggiornamento::IndirizzoNonAmmesso(u),
        ureq::Error::BodyExceedsLimit(n) => ErroreAggiornamento::TroppoGrande(n),
        ureq::Error::TooManyRedirects => ErroreAggiornamento::TroppiRinvii,
        altro => ErroreAggiornamento::Rete(altro.to_string()),
    }
}

/// Corpo della risposta come testo UTF-8, al più `limite` byte.
fn leggi_testo(
    risposta: ureq::http::Response<ureq::Body>,
    limite: u64,
) -> Result<String, ErroreAggiornamento> {
    let byte = risposta
        .into_body()
        .into_with_config()
        .limit(limite)
        .read_to_vec()
        .map_err(|e| match errore_rete(e) {
            ErroreAggiornamento::Rete(t) => ErroreAggiornamento::RispostaNonValida(t),
            altro => altro,
        })?;
    String::from_utf8(byte)
        .map_err(|_| ErroreAggiornamento::RispostaNonValida("testo non in UTF-8".into()))
}

/// Elimina il file alla fine, se non è stato tenuto con [`FileParziale::tieni`].
struct FileParziale<'a> {
    percorso: &'a Path,
    tenuto: bool,
}

impl FileParziale<'_> {
    fn tieni(mut self) {
        self.tenuto = true;
    }
}

impl Drop for FileParziale<'_> {
    fn drop(&mut self) {
        if !self.tenuto {
            let _ = fs::remove_file(self.percorso);
        }
    }
}

/// Client HTTP per il controllo e lo scaricamento.
pub struct Cliente {
    imp: ImpostazioniRete,
    api: ureq::Agent,
    file: ureq::Agent,
}

impl Cliente {
    pub fn new(imp: ImpostazioniRete) -> Self {
        let api = Self::agente(&imp, Some(imp.tempo_api));
        let file = Self::agente(&imp, None);
        Cliente { imp, api, file }
    }

    pub fn impostazioni(&self) -> &ImpostazioniRete {
        &self.imp
    }

    fn agente(imp: &ImpostazioniRete, globale: Option<Duration>) -> ureq::Agent {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            // Le radici del sistema; la verifica del certificato resta attiva.
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let mut b = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(!imp.http_in_chiaro)
            .http_status_as_error(false)
            // I rinvii si seguono a mano, controllando ogni destinazione.
            .max_redirects(0)
            .user_agent(format!("OBDio/{}", env!("CARGO_PKG_VERSION")))
            .timeout_resolve(Some(imp.tempo_connessione))
            .timeout_connect(Some(imp.tempo_connessione))
            .timeout_recv_response(Some(imp.tempo_lettura))
            .timeout_per_read(Some(imp.tempo_lettura))
            .timeout_global(globale)
            .max_response_header_size(64 * 1024);
        if imp.http_in_chiaro {
            // Le prove non devono passare da un proxy di sistema.
            b = b.proxy(None);
        }
        b.build().into()
    }

    /// GET con i rinvii seguiti a mano verso gli host ammessi.
    fn richiedi(
        &self,
        agente: &ureq::Agent,
        indirizzo: &str,
        accetta: &str,
        ammessi: &[String],
    ) -> Result<ureq::http::Response<ureq::Body>, ErroreAggiornamento> {
        controlla_indirizzo(indirizzo, ammessi, self.imp.http_in_chiaro)?;
        let mut attuale = indirizzo.to_string();
        for _ in 0..=self.imp.massimo_rinvii {
            let risposta = agente
                .get(&attuale)
                .header("Accept", accetta)
                .header("X-GitHub-Api-Version", "2022-11-28")
                .call()
                .map_err(errore_rete)?;
            let stato = risposta.status().as_u16();
            if matches!(stato, 301 | 302 | 303 | 307 | 308) {
                let posizione = risposta
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        ErroreAggiornamento::RispostaNonValida("rinvio senza destinazione".into())
                    })?;
                attuale =
                    prossimo_indirizzo(&attuale, posizione, ammessi, self.imp.http_in_chiaro)?;
                continue;
            }
            return Ok(risposta);
        }
        Err(ErroreAggiornamento::TroppiRinvii)
    }

    /// Ultimo rilascio pubblicato (bozze e pre-rilasci esclusi da GitHub).
    pub fn ultimo_rilascio(&self) -> Result<Rilascio, ErroreAggiornamento> {
        let risposta = self.richiedi(
            &self.api,
            &self.imp.indirizzo_ultimo(),
            "application/vnd.github+json",
            &self.imp.host_api,
        )?;
        let stato = risposta.status().as_u16();
        match stato {
            200 => {}
            404 => return Err(ErroreAggiornamento::NessunaVersione),
            403 | 429 => {
                let h = risposta.headers();
                let testo = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
                let retry = testo("retry-after");
                let resto = testo("x-ratelimit-remaining");
                if stato == 429 || retry.is_some() || resto.as_deref() == Some("0") {
                    return Err(ErroreAggiornamento::LimiteRichieste {
                        riprova_alle: fine_limite(
                            retry.as_deref(),
                            testo("x-ratelimit-reset").as_deref(),
                            adesso(),
                        ),
                    });
                }
                return Err(ErroreAggiornamento::Rifiutato(stato));
            }
            s => return Err(ErroreAggiornamento::Rifiutato(s)),
        }
        let testo = leggi_testo(risposta, self.imp.limite_json)?;
        Rilascio::da_json(&testo)
    }

    /// Scarica un file di testo piccolo (le somme SHA-256).
    pub fn scarica_testo(&self, indirizzo: &str) -> Result<String, ErroreAggiornamento> {
        let risposta = self.richiedi(
            &self.file,
            indirizzo,
            "application/octet-stream",
            &self.imp.host_scaricamento,
        )?;
        match risposta.status().as_u16() {
            200 => {}
            s => return Err(ErroreAggiornamento::Rifiutato(s)),
        }
        leggi_testo(risposta, self.imp.limite_somme)
    }

    /// Scarica `indirizzo` in `destinazione` (file nuovo), calcolando lo
    /// SHA-256 man mano. Si ferma, ed elimina il file, se i byte superano
    /// `attesi`, se il totale è diverso, se `annulla` diventa vero o per
    /// qualunque errore. `progresso` riceve byte scaricati e totale.
    pub fn scarica_file(
        &self,
        indirizzo: &str,
        destinazione: &Path,
        attesi: u64,
        annulla: &AtomicBool,
        progresso: &mut dyn FnMut(u64, u64),
    ) -> Result<String, ErroreAggiornamento> {
        if attesi == 0 || attesi > self.imp.limite_eseguibile {
            return Err(ErroreAggiornamento::TroppoGrande(attesi));
        }
        let risposta = self.richiedi(
            &self.file,
            indirizzo,
            "application/octet-stream",
            &self.imp.host_scaricamento,
        )?;
        match risposta.status().as_u16() {
            200 => {}
            s => return Err(ErroreAggiornamento::Rifiutato(s)),
        }
        let dichiarati = risposta
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|t| t.trim().parse::<u64>().ok());
        if let Some(d) = dichiarati
            && d != attesi
        {
            return Err(ErroreAggiornamento::DimensioneDiversa {
                attesa: attesi,
                ottenuta: d,
            });
        }
        let _ = fs::remove_file(destinazione);
        // Dichiarato prima di `file`, quindi eliminato dopo: a ogni uscita
        // anticipata il file viene prima chiuso e poi cancellato.
        let parziale = FileParziale {
            percorso: destinazione,
            tenuto: false,
        };
        let mut file: File = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destinazione)
            .map_err(|e| ErroreAggiornamento::scrittura(destinazione, &e))?;
        // Senza limite di ureq: i byte si contano qui sotto e lo scaricamento
        // si ferma al primo pezzo oltre la dimensione attesa.
        let mut lettore = risposta.into_body().into_reader();
        let mut calcolo = Calcolo::default();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut letti: u64 = 0;
        progresso(0, attesi);
        loop {
            if annulla.load(Ordering::Relaxed) {
                drop(file);
                return Err(ErroreAggiornamento::Annullato);
            }
            let n = match lettore.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    drop(file);
                    return Err(match e.kind() {
                        std::io::ErrorKind::TimedOut => ErroreAggiornamento::TempoScaduto,
                        _ => ErroreAggiornamento::NonRaggiungibile(e.to_string()),
                    });
                }
            };
            letti += n as u64;
            if letti > attesi {
                drop(file);
                return Err(ErroreAggiornamento::DimensioneDiversa {
                    attesa: attesi,
                    ottenuta: letti,
                });
            }
            calcolo.aggiungi(&buffer[..n]);
            file.write_all(&buffer[..n])
                .map_err(|e| ErroreAggiornamento::scrittura(destinazione, &e))?;
            progresso(letti, attesi);
        }
        if letti != attesi {
            drop(file);
            return Err(ErroreAggiornamento::DimensioneDiversa {
                attesa: attesi,
                ottenuta: letti,
            });
        }
        file.flush()
            .and_then(|_| file.sync_all())
            .map_err(|e| ErroreAggiornamento::scrittura(destinazione, &e))?;
        drop(file);
        parziale.tieni();
        Ok(calcolo.fine())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh() -> ImpostazioniRete {
        ImpostazioniRete::github()
    }

    #[test]
    fn github_solo_https_verso_host_noti() {
        let g = gh();
        assert!(!g.http_in_chiaro());
        assert_eq!(
            g.indirizzo_ultimo(),
            "https://api.github.com/repos/inalto/OBDio/releases/latest"
        );
        let api = &g.host_api;
        let file = &g.host_scaricamento;
        assert!(controlla_indirizzo(&g.indirizzo_ultimo(), api, false).is_ok());
        for buono in [
            "https://github.com/inalto/OBDio/releases/download/v0.4.0/SHA256SUMS.txt",
            "https://objects.githubusercontent.com/github-production-release-asset/1?x=y",
            "https://release-assets.githubusercontent.com/a/b",
            "https://GitHub.com/inalto/OBDio",
            "https://github.com:443/x",
        ] {
            assert!(controlla_indirizzo(buono, file, false).is_ok(), "{buono}");
        }
        for cattivo in [
            "http://github.com/inalto/OBDio",
            "ftp://github.com/x",
            "https://github.com.evil.example/x",
            "https://evilgithub.com/x",
            "https://githubusercontent.com/x",
            "https://evil.example/githubusercontent.com",
            "https://user:pw@github.com/x",
            "https://github.com@evil.example/x",
            "https://github.com:8443/x",
            "https://api.github.com/x",
            "github.com/x",
            "",
            "https:///x",
            "https://127.0.0.1/x",
        ] {
            assert!(
                matches!(
                    controlla_indirizzo(cattivo, file, false),
                    Err(ErroreAggiornamento::IndirizzoNonAmmesso(_))
                ),
                "{cattivo}"
            );
        }
        // L'API ammette solo il suo host.
        assert!(controlla_indirizzo("https://github.com/x", api, false).is_err());
    }

    #[test]
    fn prove_in_chiaro_solo_se_scelto() {
        let p = ImpostazioniRete::per_prove("http://127.0.0.1:4567/");
        assert!(p.http_in_chiaro());
        assert_eq!(p.base_api, "http://127.0.0.1:4567");
        assert_eq!(p.host_api, vec!["127.0.0.1".to_string()]);
        assert!(controlla_indirizzo("http://127.0.0.1:4567/x", &p.host_api, true).is_ok());
        assert!(controlla_indirizzo("http://127.0.0.1:4567/x", &p.host_api, false).is_err());
        assert!(controlla_indirizzo("http://localhost:4567/x", &p.host_api, true).is_err());
    }

    #[test]
    fn rinvii_solo_verso_https_e_host_ammessi() {
        let f = gh().host_scaricamento;
        let da = "https://github.com/inalto/OBDio/releases/download/v1/a.exe";
        assert_eq!(
            prossimo_indirizzo(
                da,
                "https://release-assets.githubusercontent.com/x?sig=1",
                &f,
                false
            )
            .unwrap(),
            "https://release-assets.githubusercontent.com/x?sig=1"
        );
        assert_eq!(
            prossimo_indirizzo(da, "/inalto/OBDio/altro", &f, false).unwrap(),
            "https://github.com/inalto/OBDio/altro"
        );
        assert_eq!(
            prossimo_indirizzo(da, "//objects.githubusercontent.com/y", &f, false).unwrap(),
            "https://objects.githubusercontent.com/y"
        );
        for cattivo in [
            "http://release-assets.githubusercontent.com/x",
            "https://evil.example/a.exe",
            "//evil.example/a.exe",
            "javascript:alert(1)",
            "",
        ] {
            assert!(
                prossimo_indirizzo(da, cattivo, &f, false).is_err(),
                "{cattivo}"
            );
        }
    }

    #[test]
    fn attesa_del_limite_di_richieste() {
        let ora = 1_800_000_000;
        assert_eq!(fine_limite(Some("60"), None, ora), Some(ora + 60));
        assert_eq!(
            fine_limite(None, Some(&(ora + 900).to_string()), ora),
            Some(ora + 900)
        );
        // Retry-After ha la precedenza.
        assert_eq!(
            fine_limite(Some("5"), Some(&(ora + 900).to_string()), ora),
            Some(ora + 5)
        );
        assert_eq!(fine_limite(Some("abc"), None, ora), None);
        assert_eq!(fine_limite(None, Some("1"), ora), None);
        assert_eq!(fine_limite(Some("999999999"), None, ora), None);
        assert_eq!(fine_limite(None, None, ora), None);
    }

    #[test]
    fn host_con_sottodomini() {
        let a = vec![
            ".githubusercontent.com".to_string(),
            "github.com".to_string(),
        ];
        assert!(host_ammesso("objects.githubusercontent.com", &a));
        assert!(host_ammesso("a.b.githubusercontent.com", &a));
        assert!(!host_ammesso("githubusercontent.com", &a));
        assert!(!host_ammesso("xgithubusercontent.com", &a));
        assert!(host_ammesso("GITHUB.COM", &a));
        assert!(!host_ammesso("www.github.com", &a));
    }

    #[test]
    fn fuori_linea_e_porta_chiusa() {
        // Una porta locale chiusa: «non raggiungibile», senza panico.
        let porta = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let c = Cliente::new(ImpostazioniRete::per_prove(&format!(
            "http://127.0.0.1:{porta}"
        )));
        assert!(matches!(
            c.ultimo_rilascio(),
            Err(ErroreAggiornamento::NonRaggiungibile(_))
        ));
        // La versione per l'utente rifiuta l'HTTP anche se l'indirizzo arriva
        // da fuori.
        let g = Cliente::new(ImpostazioniRete {
            base_api: format!("http://127.0.0.1:{porta}"),
            host_api: vec!["127.0.0.1".into()],
            ..ImpostazioniRete::github()
        });
        assert!(matches!(
            g.ultimo_rilascio(),
            Err(ErroreAggiornamento::IndirizzoNonAmmesso(_))
        ));
    }
}
