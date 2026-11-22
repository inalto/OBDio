//! Controllo e installazione degli aggiornamenti (solo desktop; su Android
//! gli aggiornamenti arrivano dall'APK o dallo store). Vedi
//! `docs/AGGIORNAMENTI.md`.
//!
//! # Strati
//!
//! - [`versione`]: lettura e confronto delle versioni;
//! - [`rilascio`]: il rilascio letto dall'API di GitHub e i nomi dei file;
//! - [`somme`]: `SHA256SUMS.txt` e SHA-256;
//! - [`pianificazione`]: `aggiornamenti.json`, quando controllare, versione
//!   ignorata;
//! - [`rete`]: HTTPS verso GitHub con host ammessi, rinvii controllati,
//!   tempi e dimensioni massimi;
//! - [`sostituzione`]: sostituzione dell'eseguibile in uso con ritorno
//!   indietro;
//! - [`Servizio`]: thread a parte che fa tutto questo senza toccare né
//!   l'interfaccia né il thread di lavoro dell'adattatore.
//!
//! # Sicurezza
//!
//! Solo HTTPS con certificato verificato, solo verso `api.github.com`,
//! `github.com` e `*.githubusercontent.com`; nessun dato dell'utente o del
//! veicolo. Il file scaricato deve avere la dimensione dichiarata dall'API e
//! la somma SHA-256 scritta in `SHA256SUMS.txt` dello stesso rilascio: così
//! si scoprono un file danneggiato o alterato lungo la strada, **non** un
//! account o un repository GitHub compromessi. L'eseguibile non è firmato.

pub mod pianificazione;
pub mod rete;
pub mod rilascio;
pub mod somme;
pub mod sostituzione;
pub mod versione;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

pub use pianificazione::ConfigAggiornamenti;
pub use rete::{Cliente, ImpostazioniRete};
pub use rilascio::{Rilascio, Risorsa};
pub use versione::{Confronto, Versione};

/// Errori del controllo e dell'installazione, ognuno con un messaggio per
/// l'utente ([`ErroreAggiornamento::messaggio`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErroreAggiornamento {
    /// GitHub risponde 404: nessun rilascio pubblicato (o il repository non
    /// esiste ancora).
    NessunaVersione,
    /// GitHub limita le richieste; `riprova_alle` in secondi Unix, se noto.
    LimiteRichieste {
        riprova_alle: Option<i64>,
    },
    /// Altro codice HTTP inatteso.
    Rifiutato(u16),
    /// Nessuna connessione, nome non risolto, connessione rifiutata.
    NonRaggiungibile(String),
    TempoScaduto,
    RispostaNonValida(String),
    /// Indirizzo non HTTPS o verso un host non ammesso.
    IndirizzoNonAmmesso(String),
    TroppiRinvii,
    /// Il rilascio non contiene il file atteso per questa piattaforma.
    RisorsaMancante(String),
    /// Il file del rilascio non ha il nome atteso.
    NomeRisorsaNonValido(String),
    /// `SHA256SUMS.txt` manca o non contiene la somma del file.
    SommaMancante(String),
    SommaDiversa {
        attesa: String,
        ottenuta: String,
    },
    DimensioneDiversa {
        attesa: u64,
        ottenuta: u64,
    },
    TroppoGrande(u64),
    /// La versione offerta non è strettamente più nuova.
    NonPiuNuova {
        installata: String,
        offerta: String,
    },
    Annullato,
    CartellaNonScrivibile(PathBuf, String),
    /// Installazione non consentita (build di sviluppo, piattaforma).
    InstallazioneBloccata(String),
    /// Sostituzione non riuscita, versione in uso ripristinata.
    Sostituzione(String),
    /// Sostituzione non riuscita e ripristino non riuscito.
    SostituzioneCritica(String),
    Disco(String),
    Rete(String),
}

/// Byte in forma leggibile: «12,4 MB».
pub fn byte_leggibili(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / MB).replace('.', ",")
    } else if n >= 1024 {
        format!("{} kB", n / 1024)
    } else {
        format!("{n} byte")
    }
}

/// Ora locale «HH:MM» di un momento in secondi Unix.
fn ora_locale(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "più tardi".into())
}

impl ErroreAggiornamento {
    pub(crate) fn scrittura(percorso: &Path, e: &std::io::Error) -> Self {
        ErroreAggiornamento::Disco(format!("impossibile scrivere {}: {e}", percorso.display()))
    }

    /// Che cosa è successo e che cosa fare, in italiano.
    pub fn messaggio(&self) -> String {
        use ErroreAggiornamento::*;
        match self {
            NessunaVersione => {
                "Su GitHub non è ancora pubblicata nessuna versione di OBDio.".into()
            }
            LimiteRichieste { riprova_alle } => format!(
                "GitHub ha limitato le richieste da questa rete (senza account ne accetta circa \
                 60 all'ora). {}",
                match riprova_alle {
                    Some(t) => format!("Riprova dopo le {}.", ora_locale(*t)),
                    None => "Riprova fra un'ora.".into(),
                }
            ),
            Rifiutato(s) => format!(
                "GitHub ha risposto con il codice {s}. Riprova più tardi o controlla a mano la \
                 pagina delle versioni."
            ),
            NonRaggiungibile(d) => format!(
                "GitHub non è raggiungibile: controlla la connessione a Internet. Il resto \
                 dell'app funziona anche senza rete. Dettaglio: {d}"
            ),
            TempoScaduto => {
                "GitHub non ha risposto in tempo. Controlla la connessione e riprova.".into()
            }
            RispostaNonValida(d) => format!("La risposta di GitHub non è leggibile: {d}"),
            IndirizzoNonAmmesso(d) => format!(
                "Indirizzo rifiutato per sicurezza: {d}. L'app scarica solo in HTTPS da GitHub."
            ),
            TroppiRinvii => "Troppi rinvii da un indirizzo all'altro: scaricamento interrotto, \
                             nulla è stato installato."
                .into(),
            RisorsaMancante(n) => format!(
                "Questa versione non contiene il file {n}: scaricala a mano dalla pagina della \
                 versione."
            ),
            NomeRisorsaNonValido(n) => format!(
                "Il file {n} del rilascio non ha il nome atteso: per sicurezza non viene \
                 scaricato."
            ),
            SommaMancante(n) => format!(
                "Il file SHA256SUMS.txt manca o non contiene una somma valida per {n}: per \
                 sicurezza non viene installato nulla."
            ),
            SommaDiversa { .. } => "Il file scaricato non corrisponde alla somma SHA-256 \
                                    pubblicata: potrebbe essere danneggiato o alterato. È stato \
                                    eliminato e non è stato installato nulla."
                .into(),
            DimensioneDiversa { attesa, ottenuta } => format!(
                "Il file scaricato ha una dimensione diversa da quella dichiarata ({} invece di \
                 {}). È stato eliminato e non è stato installato nulla.",
                byte_leggibili(*ottenuta),
                byte_leggibili(*attesa)
            ),
            TroppoGrande(n) => format!(
                "La risposta è troppo grande ({}): scaricamento interrotto.",
                byte_leggibili(*n)
            ),
            NonPiuNuova {
                installata,
                offerta,
            } => format!(
                "La versione {offerta} non è più nuova di quella installata ({installata}): non \
                 c'è nulla da installare."
            ),
            Annullato => "Scaricamento annullato. Il file parziale è stato eliminato.".into(),
            CartellaNonScrivibile(p, e) => format!(
                "Nella cartella {} non si può scrivere ({e}); succede per esempio in «Programmi», \
                 che richiede i permessi di amministratore. Scarica la versione dalla pagina e \
                 sostituisci OBDio.exe a mano, oppure sposta la cartella di OBDio in una tua.",
                p.display()
            ),
            InstallazioneBloccata(m) => m.clone(),
            Sostituzione(m) => format!("Aggiornamento non installato: {m}."),
            SostituzioneCritica(m) => format!("Attenzione: {m}."),
            Disco(m) => format!("Errore del disco: {m}."),
            Rete(m) => format!("Errore di rete: {m}"),
        }
    }
}

impl fmt::Display for ErroreAggiornamento {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.messaggio())
    }
}

/// Che cosa fare del rilascio trovato.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Valutazione {
    /// L'installata è la più recente.
    NessunaNovita,
    /// Versione più nuova da proporre.
    Disponibile,
    /// Più nuova, ma l'utente ha scelto di ignorarla.
    Ignorata,
    /// Bozza o pre-rilascio: non proposta.
    NonProposta,
    /// Etichetta illeggibile: mai trattata come più nuova.
    Illeggibile,
}

pub fn valuta(installata: &Versione, r: &Rilascio, config: &ConfigAggiornamenti) -> Valutazione {
    match r.confronto(installata) {
        Confronto::PiuNuova => {
            if r.versione.as_ref().is_some_and(|v| config.ignorata(v)) {
                Valutazione::Ignorata
            } else {
                Valutazione::Disponibile
            }
        }
        Confronto::NonPiuNuova => Valutazione::NessunaNovita,
        Confronto::PreRilascio => Valutazione::NonProposta,
        Confronto::Sconosciuta => Valutazione::Illeggibile,
    }
}

/// Installazione riuscita.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installato {
    pub versione: String,
    pub eseguibile: PathBuf,
    /// Copia della versione precedente (eliminata al prossimo avvio).
    pub vecchio: PathBuf,
}

/// Scarica l'eseguibile del rilascio per `piattaforma`, lo verifica
/// (dimensione e SHA-256) e lo mette al posto di `exe`. Rifiuta una versione
/// non strettamente più nuova, file con nomi diversi da quelli attesi, una
/// build di sviluppo (vedi [`sostituzione::blocco_sviluppo`]) e una cartella
/// non scrivibile. In caso di errore `exe` non cambia e non resta alcun file
/// parziale.
pub fn scarica_e_installa(
    cliente: &Cliente,
    rilascio: &Rilascio,
    installata: &Versione,
    exe: &Path,
    piattaforma: &str,
    annulla: &AtomicBool,
    progresso: &mut dyn FnMut(u64, u64),
) -> Result<Installato, ErroreAggiornamento> {
    if let Some(m) = sostituzione::blocco_sviluppo_corrente(exe) {
        return Err(ErroreAggiornamento::InstallazioneBloccata(m));
    }
    if rilascio.confronto(installata) != Confronto::PiuNuova {
        return Err(ErroreAggiornamento::NonPiuNuova {
            installata: installata.to_string(),
            offerta: rilascio.versione_testo(),
        });
    }
    let versione = rilascio
        .versione
        .as_ref()
        .ok_or_else(|| ErroreAggiornamento::RispostaNonValida("versione illeggibile".into()))?;
    let nome = rilascio::nome_eseguibile(versione, piattaforma);
    let risorsa = rilascio
        .risorsa(&nome)
        .ok_or_else(|| ErroreAggiornamento::RisorsaMancante(nome.clone()))?;
    // L'indirizzo deve finire proprio con il nome atteso.
    let percorso_url = risorsa.indirizzo.split(['?', '#']).next().unwrap_or("");
    if !percorso_url.ends_with(&format!("/{nome}")) {
        return Err(ErroreAggiornamento::NomeRisorsaNonValido(
            risorsa.indirizzo.clone(),
        ));
    }
    let somme = rilascio
        .risorsa(rilascio::NOME_SOMME)
        .ok_or_else(|| ErroreAggiornamento::SommaMancante(nome.clone()))?;
    if risorsa.dimensione == 0 || risorsa.dimensione > cliente.impostazioni().limite_eseguibile {
        return Err(ErroreAggiornamento::TroppoGrande(risorsa.dimensione));
    }
    sostituzione::cartella_scrivibile(exe)?;
    let testo = cliente.scarica_testo(&somme.indirizzo)?;
    let attesa = somme::somma_per(&testo, &nome)?;
    if annulla.load(Ordering::Relaxed) {
        return Err(ErroreAggiornamento::Annullato);
    }
    let nuovo = sostituzione::percorso_nuovo(exe);
    let ottenuta = cliente.scarica_file(
        &risorsa.indirizzo,
        &nuovo,
        risorsa.dimensione,
        annulla,
        progresso,
    )?;
    if let Err(e) = somme::verifica(&attesa, &ottenuta) {
        let _ = std::fs::remove_file(&nuovo);
        return Err(e);
    }
    if annulla.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&nuovo);
        return Err(ErroreAggiornamento::Annullato);
    }
    let s = sostituzione::sostituisci(&sostituzione::FileReali, exe, &nuovo)?;
    Ok(Installato {
        versione: versione.to_string(),
        eseguibile: s.eseguibile,
        vecchio: s.vecchio,
    })
}

// ---------------------------------------------------------------------------
// Servizio: thread a parte
// ---------------------------------------------------------------------------

enum Richiesta {
    Controlla,
    Installa(Box<Rilascio>),
    Esci,
}

/// Notizie dal thread degli aggiornamenti all'interfaccia.
#[derive(Debug)]
pub enum Notizia {
    /// Esito di un controllo (quando è stato chiesto, in secondi Unix).
    Controllo {
        esito: Result<Box<Rilascio>, ErroreAggiornamento>,
        quando: i64,
    },
    /// Byte scaricati e totale dell'eseguibile.
    Progresso {
        scaricati: u64,
        totale: u64,
    },
    Installazione(Result<Installato, ErroreAggiornamento>),
}

/// Ritardi, dall'avvio, dei tentativi di eliminare la versione precedente
/// (subito dopo il riavvio può essere ancora in chiusura).
const PULIZIE: [Duration; 4] = [
    Duration::from_secs(3),
    Duration::from_secs(20),
    Duration::from_secs(90),
    Duration::from_secs(300),
];

/// Intervallo minimo fra due notizie di avanzamento.
const PASSO_PROGRESSO: Duration = Duration::from_millis(100);

/// Thread degli aggiornamenti. Le richieste partono solo quando
/// l'interfaccia le chiede; le notizie si leggono con [`Servizio::ricevi`].
pub struct Servizio {
    tx: Sender<Richiesta>,
    rx: Receiver<Notizia>,
    annulla: Arc<AtomicBool>,
    finito: Receiver<()>,
}

impl Servizio {
    /// Avvia il thread. `exe` è l'eseguibile da aggiornare (e di cui pulire
    /// i residui); `notifica` risveglia l'interfaccia dopo ogni notizia.
    pub fn avvia(
        imp: ImpostazioniRete,
        exe: Option<PathBuf>,
        notifica: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx_r) = mpsc::channel::<Richiesta>();
        let (tx_n, rx) = mpsc::channel::<Notizia>();
        let (tx_fine, finito) = mpsc::channel::<()>();
        let annulla = Arc::new(AtomicBool::new(false));
        let annulla_t = annulla.clone();
        let avviato = std::thread::Builder::new()
            .name("obdio-aggiornamenti".into())
            .spawn(move || {
                let cliente = Cliente::new(imp);
                let installata = Versione::corrente();
                let invia = |n: Notizia| {
                    let _ = tx_n.send(n);
                    notifica();
                };
                let inizio = Instant::now();
                let mut pulizie = PULIZIE.iter().peekable();
                if exe.is_none() {
                    pulizie.by_ref().for_each(drop);
                }
                loop {
                    let attesa = pulizie
                        .peek()
                        .map(|d| d.saturating_sub(inizio.elapsed()))
                        .unwrap_or(Duration::from_secs(3600));
                    let richiesta = match rx_r.recv_timeout(attesa) {
                        Ok(r) => Some(r),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    match richiesta {
                        None => {
                            pulizie.next();
                            if let Some(e) = &exe
                                && sostituzione::pulisci_residui(&sostituzione::FileReali, e)
                            {
                                pulizie.by_ref().for_each(drop);
                            }
                        }
                        Some(Richiesta::Esci) => break,
                        Some(Richiesta::Controlla) => {
                            let quando = rete::adesso();
                            let esito = cliente.ultimo_rilascio().map(Box::new);
                            invia(Notizia::Controllo { esito, quando });
                        }
                        Some(Richiesta::Installa(r)) => {
                            let esito = match (&exe, rilascio::piattaforma_corrente()) {
                                (None, _) => Err(ErroreAggiornamento::InstallazioneBloccata(
                                    "Percorso dell'eseguibile sconosciuto.".into(),
                                )),
                                (_, None) => Err(ErroreAggiornamento::InstallazioneBloccata(
                                    "L'installazione automatica è disponibile solo su Windows \
                                     a 64 bit."
                                        .into(),
                                )),
                                (Some(e), Some(p)) => {
                                    let mut ultimo: Option<Instant> = None;
                                    let mut progresso = |scaricati: u64, totale: u64| {
                                        if scaricati == totale
                                            || ultimo.is_none_or(|t| t.elapsed() >= PASSO_PROGRESSO)
                                        {
                                            ultimo = Some(Instant::now());
                                            invia(Notizia::Progresso { scaricati, totale });
                                        }
                                    };
                                    scarica_e_installa(
                                        &cliente,
                                        &r,
                                        &installata,
                                        e,
                                        p,
                                        &annulla_t,
                                        &mut progresso,
                                    )
                                }
                            };
                            invia(Notizia::Installazione(esito));
                        }
                    }
                }
                let _ = tx_fine.send(());
            });
        if let Err(e) = avviato {
            eprintln!("aggiornamenti: thread non avviato: {e}");
        }
        Servizio {
            tx,
            rx,
            annulla,
            finito,
        }
    }

    /// Chiede l'ultimo rilascio a GitHub.
    pub fn controlla(&self) {
        let _ = self.tx.send(Richiesta::Controlla);
    }

    /// Scarica, verifica e installa il rilascio.
    pub fn installa(&self, r: Rilascio) {
        self.annulla.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Richiesta::Installa(Box::new(r)));
    }

    /// Interrompe lo scaricamento in corso (il file parziale viene eliminato).
    pub fn annulla(&self) {
        self.annulla.store(true, Ordering::Relaxed);
    }

    /// Prossima notizia, se c'è.
    pub fn ricevi(&self) -> Option<Notizia> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Servizio {
    fn drop(&mut self) {
        // Uno scaricamento in corso si interrompe ed elimina il file
        // parziale; si aspetta poco, per non trattenere la chiusura dell'app.
        self.annulla.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Richiesta::Esci);
        let _ = self.finito.recv_timeout(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valutazione_del_rilascio() {
        let installata = Versione::leggi("0.3.0").unwrap();
        let mut r = Rilascio::da_json(
            r#"{"tag_name":"v0.4.0","html_url":"https://github.com/inalto/OBDio/releases/tag/v0.4.0"}"#,
        )
        .unwrap();
        let mut c = ConfigAggiornamenti::default();
        assert_eq!(valuta(&installata, &r, &c), Valutazione::Disponibile);
        c.ignora_versione = Some("0.4.0".into());
        assert_eq!(valuta(&installata, &r, &c), Valutazione::Ignorata);
        r.pre_rilascio = true;
        assert_eq!(valuta(&installata, &r, &c), Valutazione::NonProposta);
        r.pre_rilascio = false;
        r.etichetta = "v0.3.0".into();
        assert_eq!(valuta(&installata, &r, &c), Valutazione::NessunaNovita);
        r.etichetta = "release-day".into();
        assert_eq!(valuta(&installata, &r, &c), Valutazione::Illeggibile);
    }

    #[test]
    fn messaggi_in_italiano_e_dimensioni() {
        assert_eq!(byte_leggibili(500), "500 byte");
        assert_eq!(byte_leggibili(2048), "2 kB");
        assert_eq!(byte_leggibili(13_002_342), "12,4 MB");
        let e = ErroreAggiornamento::CartellaNonScrivibile(
            PathBuf::from("C:/Program Files/OBDio"),
            "accesso negato".into(),
        );
        assert!(e.messaggio().contains("a mano"));
        assert!(
            ErroreAggiornamento::LimiteRichieste { riprova_alle: None }
                .messaggio()
                .contains("60")
        );
        assert!(
            ErroreAggiornamento::SommaDiversa {
                attesa: "a".into(),
                ottenuta: "b".into()
            }
            .to_string()
            .contains("eliminato")
        );
    }
}
