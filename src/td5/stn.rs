//! Come i messaggi del Td5 arrivano sulla linea K.
//!
//! [`CanaleTd5`] separa i messaggi del Td5 (che cosa si chiede) dal modo in
//! cui raggiungono il filo. L'unica realizzazione è [`Stn`]: un adattatore
//! OBDLink con chip STN (CX su Bluetooth LE, LX su porta COM), comandato con i
//! comandi AT e ST attraverso un qualunque [`Transport`]. Un cavo «KKL» (linea
//! K su porta seriale, con l'inizializzazione rapida e l'eco gestite dall'app)
//! sarebbe un'altra realizzazione dello stesso tratto: qui non c'è.
//!
//! NULLA DI QUESTO È STATO PROVATO CON UN ADATTATORE VERO. I comandi vengono
//! dal manuale OBDLink (ricerca/td5/frpm.txt, tabelle 15 e 16); come
//! l'adattatore tratti i messaggi senza intestazione del Td5 non è
//! documentato. Per questo ci sono tre strategie, provate in ordine finché una
//! ottiene `50 A0` all'avvio della sessione, e ogni comando con la sua
//! risposta grezza va nel registro dei collegamenti: un tentativo fallito sul
//! veicolo dice esattamente che cosa ha risposto l'adattatore.

use std::time::{Duration, Instant};

use super::frame::{self, Risposta};
use super::richieste::{INIZIO_COMUNICAZIONE, RichiestaTd5};
use crate::registri::Diario;
use crate::transport::{Scollegato, Transport};

/// Errori del dialogo con il Td5.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErroreTd5 {
    /// Il collegamento con l'adattatore è caduto.
    Scollegato(String),
    /// L'adattatore non ha restituito il prompt in tempo.
    Timeout(String),
    /// L'adattatore non ha un chip STN: non può parlare con il Td5.
    NonStn(String),
    /// Risposta inattesa dell'adattatore («?», errore di bus…).
    Adattatore(String),
    /// La centralina non ha risposto (NO DATA o niente).
    NessunaRisposta,
    /// Risposta non interpretabile.
    Trama(String),
    /// Nessuna strategia è riuscita ad aprire la sessione diagnostica.
    Inizializzazione(String),
    /// La centralina ha rifiutato il seme o la chiave.
    AccessoNegato(String),
    /// La sessione è caduta e la nuova inizializzazione non è riuscita.
    SessionePersa(String),
}

impl std::fmt::Display for ErroreTd5 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErroreTd5::Scollegato(m) => write!(f, "collegamento interrotto: {m}"),
            ErroreTd5::Timeout(c) => {
                write!(f, "nessuna risposta dall'adattatore a «{c}» (timeout)")
            }
            ErroreTd5::NonStn(m) => write!(
                f,
                "questo adattatore non ha un chip STN ({m}): un adattatore di tipo ELM327 \
                 aggiunge sempre un'intestazione ai messaggi e non può leggere la centralina \
                 Td5. Serve un OBDLink CX (Bluetooth LE) o LX (Bluetooth classico)"
            ),
            ErroreTd5::Adattatore(m) => write!(f, "risposta inattesa dell'adattatore: {m}"),
            ErroreTd5::NessunaRisposta => write!(f, "nessuna risposta dalla centralina"),
            ErroreTd5::Trama(m) => write!(f, "risposta non interpretabile: {m}"),
            ErroreTd5::Inizializzazione(m) => write!(
                f,
                "la centralina Td5 non ha aperto la sessione diagnostica con nessuno dei modi \
                 provati ({m}). Controlla che il quadro sia acceso; i dettagli sono nel \
                 registro dei collegamenti"
            ),
            ErroreTd5::AccessoNegato(m) => write!(f, "accesso di sicurezza non riuscito: {m}"),
            ErroreTd5::SessionePersa(m) => write!(
                f,
                "la sessione con la centralina Td5 è caduta e non si è riaperta: {m}"
            ),
        }
    }
}

impl From<Scollegato> for ErroreTd5 {
    fn from(s: Scollegato) -> Self {
        ErroreTd5::Scollegato(s.0)
    }
}

impl ErroreTd5 {
    /// Vero se il collegamento non è più utilizzabile.
    pub fn fatale(&self) -> bool {
        matches!(self, ErroreTd5::Scollegato(_) | ErroreTd5::SessionePersa(_))
    }
}

/// Confine fra i messaggi del Td5 e il modo in cui raggiungono la linea K.
pub trait CanaleTd5: Send {
    /// Verifica e configura l'adattatore. Restituisce la sua identificazione.
    fn prepara(&mut self) -> Result<String, ErroreTd5>;
    /// Inizializzazione rapida e avvio della sessione diagnostica (`10 A0`).
    /// Dopo la prima volta ripete il modo che ha funzionato.
    fn avvia_sessione(&mut self) -> Result<(), ErroreTd5>;
    /// Invia una richiesta e ne restituisce la risposta.
    fn richiesta(&mut self, r: RichiestaTd5) -> Result<Risposta, ErroreTd5>;
    fn descrizione(&self) -> String;
    fn is_demo(&self) -> bool;
    /// Modo che ha aperto la sessione, per l'utente e il registro.
    fn strategia(&self) -> Option<String>;
    /// Se attivo, ogni scambio va nel registro dell'app.
    fn imposta_traccia(&mut self, attiva: bool);
    fn traccia(&self) -> bool;
    /// Ogni riga del registro va subito anche qui (registro dei collegamenti).
    fn imposta_diario(&mut self, diario: Diario);
    /// Preleva le righe di registro accumulate.
    fn prendi_registro(&mut self) -> Vec<String>;
    /// Annota una riga nel registro (passi della sessione).
    fn annota(&mut self, riga: String);
    fn chiudi(&mut self);
}

/// Modi di far passare i messaggi senza intestazione attraverso un OBDLink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategia {
    /// `STP 21` (ISO 9141 senza intestazione né inizializzazione automatica),
    /// `STIFI`, poi richieste esadecimali semplici.
    A,
    /// `STP 25` (ISO 14230 con inizializzazione rapida) per `STIFI`, poi
    /// `STP 21` e richieste semplici: la sessione regge se il passaggio dura
    /// meno di 2 s.
    B,
    /// `STP 25`, mantenimento automatico dell'adattatore spento (`ATSW 00`),
    /// `STIFI`, poi ogni messaggio con `STPX h:…, d:…`.
    C,
}

impl Strategia {
    pub const ORDINE: [Strategia; 3] = [Strategia::A, Strategia::B, Strategia::C];

    pub fn descrizione(self) -> &'static str {
        match self {
            Strategia::A => "strategia A: STP 21, STIFI, richieste senza intestazione",
            Strategia::B => "strategia B: STP 25 per STIFI, poi STP 21 e richieste semplici",
            Strategia::C => "strategia C: STP 25, ATSW 00, STIFI, messaggi con STPX",
        }
    }

    /// Protocollo in cui si inviano le richieste dopo l'inizializzazione.
    fn protocollo_iniziale(self) -> &'static str {
        match self {
            Strategia::A => "21",
            Strategia::B | Strategia::C => "25",
        }
    }
}

/// Testo dell'inizializzazione rapida: 25 ms bassa, 25 ms alta, poi
/// StartCommunication (ricerca/td5/proto.md; sintassi da frpm.txt, STIFI).
pub fn comando_stifi() -> String {
    format!(
        "STIFI 25, 25, {}",
        frame::esadecimale_compatto(&INIZIO_COMUNICAZIONE)
    )
}

/// Comando con cui una richiesta raggiunge la linea, secondo la strategia.
/// Il checksum lo aggiunge l'adattatore (STPCB 1, l'impostazione predefinita).
pub fn comando_richiesta(s: Strategia, r: RichiestaTd5) -> String {
    let trama = r.trama();
    let senza_checksum = &trama[..trama.len() - 1];
    match s {
        Strategia::A | Strategia::B => frame::esadecimale_compatto(senza_checksum),
        Strategia::C => {
            // Intestazione di tre byte = i primi tre del messaggio; il resto è
            // «dati» (può essere vuoto, come nell'esempio `STPX h:123456, d:`).
            let (h, d) = senza_checksum.split_at(3.min(senza_checksum.len()));
            format!(
                "STPX h:{}, d:{}, r:1",
                frame::esadecimale_compatto(h),
                frame::esadecimale_compatto(d)
            )
        }
    }
}

const T_AT: Duration = Duration::from_secs(3);
const T_RIAVVIO: Duration = Duration::from_secs(6);
const T_STIFI: Duration = Duration::from_secs(5);
const T_RICHIESTA: Duration = Duration::from_secs(3);
/// Fra un tentativo e l'altro: oltre i 2 s di silenzio la centralina chiude
/// qualunque sessione aperta a metà (proto.md, «Keep-Alive»).
const PAUSA_FRA_TENTATIVI: Duration = Duration::from_millis(2500);

/// Esito di un tentativo: `Ok(Err(motivo))` se la strategia non ha
/// funzionato, `Err` se il collegamento è inservibile.
type Tentativo = Result<Result<(), String>, ErroreTd5>;

/// Adattatore OBDLink (chip STN).
pub struct Stn {
    t: Box<dyn Transport>,
    ordine: Vec<Strategia>,
    strategia: Option<Strategia>,
    pausa: Duration,
    traccia: bool,
    registro: Vec<String>,
    diario: Diario,
}

impl Stn {
    pub fn new(t: Box<dyn Transport>) -> Self {
        Stn {
            t,
            ordine: Strategia::ORDINE.to_vec(),
            strategia: None,
            pausa: PAUSA_FRA_TENTATIVI,
            traccia: true,
            registro: Vec::new(),
            diario: Diario::muto(),
        }
    }

    /// Pausa fra due tentativi (le verifiche la accorciano).
    pub fn con_pausa(mut self, pausa: Duration) -> Self {
        self.pausa = pausa;
        self
    }

    /// Strategie da provare, in ordine (verifiche).
    pub fn con_ordine(mut self, ordine: &[Strategia]) -> Self {
        self.ordine = ordine.to_vec();
        self
    }

    pub fn strategia_scelta(&self) -> Option<Strategia> {
        self.strategia
    }

    fn scrivi_registro(&mut self, riga: String) {
        self.diario.passo(format!("TD5 {riga}"));
        self.registro.push(riga);
    }

    /// Invia un comando e raccoglie le righe fino al prompt `>`.
    fn scambia(
        &mut self,
        cmd: &str,
        attesa: Duration,
        riservato: bool,
    ) -> Result<Vec<String>, ErroreTd5> {
        while !self.t.read(Duration::ZERO)?.is_empty() {}
        self.t.write(format!("{cmd}\r").as_bytes())?;
        let scadenza = Instant::now() + attesa;
        let mut buf: Vec<u8> = Vec::new();
        while !buf.contains(&b'>') {
            let ora = Instant::now();
            if ora >= scadenza {
                if self.traccia {
                    self.scrivi_registro(format!("→ {cmd}   ← (timeout)"));
                }
                return Err(ErroreTd5::Timeout(cmd.to_string()));
            }
            let pezzo = self
                .t
                .read((scadenza - ora).min(Duration::from_millis(100)))?;
            buf.extend(pezzo.into_iter().filter(|b| *b != 0));
        }
        let fine = buf.iter().position(|b| *b == b'>').unwrap_or(buf.len());
        let testo = String::from_utf8_lossy(&buf[..fine]).to_string();
        let compatto = |s: &str| {
            s.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_uppercase()
        };
        let eco = compatto(cmd);
        let righe: Vec<String> = testo
            .split(['\r', '\n'])
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty() && compatto(r) != eco)
            .collect();
        if self.traccia {
            if riservato {
                self.scrivi_registro(format!("→ {cmd}   ← (risposta omessa dal registro)"));
            } else {
                self.scrivi_registro(format!("→ {cmd}   ← {}", righe.join(" | ")));
            }
        }
        Ok(righe)
    }

    /// Comando di configurazione: `Ok(true)` se l'adattatore risponde «OK».
    fn configura(&mut self, cmd: &str) -> Result<bool, ErroreTd5> {
        let attesa = if cmd == "ATZ" { T_RIAVVIO } else { T_AT };
        let righe = self.scambia(cmd, attesa, false)?;
        Ok(righe.iter().any(|r| r.to_uppercase().contains("OK")))
    }

    /// Protocollo e tempi. `Ok(Err)` se l'adattatore non accetta il protocollo.
    fn imposta_protocollo(&mut self, protocollo: &str) -> Tentativo {
        if !self.configura(&format!("STP {protocollo}"))? {
            return Ok(Err(format!("l'adattatore non accetta STP {protocollo}")));
        }
        // Facoltativi: se l'adattatore li rifiuta restano i valori predefiniti
        // (10 400 baud per i protocolli ISO). La risposta resta nel registro.
        // Attesa della risposta 500 ms (proto.md: «Recommended timeout»), P3
        // di 50 ms (minimo 25), pausa fra i byte ricevuti di 40 ms.
        for cmd in ["STPBR 10400", "STPTO 500", "STPTRQ 50", "STIP1X 40"] {
            self.configura(cmd)?;
        }
        Ok(Ok(()))
    }

    /// Inizializzazione rapida con StartCommunication. Riuscita se fra i byte
    /// stampati c'è C1 (risposta positiva, `03 C1 57 8F AA`).
    fn inizio_rapido(&mut self) -> Tentativo {
        let righe = self.scambia(&comando_stifi(), T_STIFI, false)?;
        let testo = righe.join(" ").to_uppercase();
        let byte: Vec<u8> = righe
            .iter()
            .filter_map(|r| frame::da_esadecimale(r))
            .flatten()
            .collect();
        if byte.contains(&0xC1) {
            return Ok(Ok(()));
        }
        if testo.contains('?') {
            return Ok(Err(
                "l'adattatore non accetta STIFI in questo protocollo («?»)".into(),
            ));
        }
        if righe.iter().all(|r| r.eq_ignore_ascii_case("OK")) && !righe.is_empty() {
            // Il manuale mostra la risposta della centralina; un semplice «OK»
            // non è documentato. Si prova comunque la sessione.
            self.scrivi_registro(
                "STIFI ha risposto solo «OK»: si prova comunque ad aprire la sessione".into(),
            );
            return Ok(Ok(()));
        }
        Ok(Err(format!(
            "inizializzazione rapida senza risposta valida («{}»)",
            righe.join(" | ")
        )))
    }

    /// Prima richiesta della sessione: deve tornare `50 A0`.
    fn sessione_diagnostica(&mut self, s: Strategia) -> Tentativo {
        match self.invia(s, RichiestaTd5::SessioneDiagnostica) {
            Ok(Risposta::Positiva(c)) if c.starts_with(&[0x50, 0xA0]) => Ok(Ok(())),
            Ok(Risposta::Positiva(c)) => Ok(Err(format!(
                "risposta inattesa all'avvio della sessione: {}",
                frame::esadecimale(&c)
            ))),
            Ok(Risposta::Negativa { codice, .. }) => Ok(Err(format!(
                "la centralina rifiuta l'avvio della sessione: {}",
                frame::motivo_negativo(codice)
            ))),
            Err(e) if e.fatale() => Err(e),
            Err(e) => Ok(Err(e.to_string())),
        }
    }

    fn tenta(&mut self, s: Strategia) -> Tentativo {
        self.scrivi_registro(format!("Inizializzazione Td5 con la {}", s.descrizione()));
        if let Err(m) = self.imposta_protocollo(s.protocollo_iniziale())? {
            return Ok(Err(m));
        }
        if s == Strategia::C && !self.configura("ATSW 00")? {
            // Senza ATSW 00 l'adattatore manderebbe ogni 3 s il proprio
            // mantenimento (C1 33 F1 3E): lo si sostituisce con quello del Td5.
            self.configura("ATWM 023E01")?;
        }
        if let Err(m) = self.inizio_rapido()? {
            return Ok(Err(m));
        }
        if s == Strategia::B {
            // Cambio di protocollo senza toccare la linea: la sessione regge
            // se tutto dura meno di 2 s.
            if !self.configura("STP 21")? {
                return Ok(Err("l'adattatore non accetta STP 21".into()));
            }
            for cmd in ["STPTO 500", "STPTRQ 50"] {
                self.configura(cmd)?;
            }
        }
        self.sessione_diagnostica(s)
    }

    /// Invia una richiesta nel modo della strategia e interpreta la risposta.
    fn invia(&mut self, s: Strategia, r: RichiestaTd5) -> Result<Risposta, ErroreTd5> {
        let trama = r.trama();
        let righe = self.scambia(&comando_richiesta(s, r), T_RICHIESTA, r.riservata())?;
        let mut byte = Vec::new();
        for riga in &righe {
            let u = riga.to_uppercase();
            if let Some(b) = frame::da_esadecimale(riga) {
                byte.extend(b);
                continue;
            }
            if u == "OK"
                || u.starts_with("SEARCHING")
                || (u.starts_with("BUS INIT") && u.ends_with("OK"))
            {
                continue;
            }
            if u.contains("NO DATA") {
                return Err(ErroreTd5::NessunaRisposta);
            }
            if u.contains('?') {
                return Err(ErroreTd5::Adattatore(
                    "comando non riconosciuto («?»)".into(),
                ));
            }
            // Riga con byte e un avviso in coda (per esempio «<DATA ERROR»):
            // il contenuto non è affidabile.
            return Err(ErroreTd5::Adattatore(if r.riservata() {
                "risposta con un errore dell'adattatore".into()
            } else {
                riga.clone()
            }));
        }
        if byte.is_empty() {
            return Err(ErroreTd5::NessunaRisposta);
        }
        frame::estrai(&byte, &trama).ok_or_else(|| {
            ErroreTd5::Trama(if r.riservata() {
                "risposta non interpretabile (omessa dal registro)".into()
            } else {
                frame::esadecimale(&byte)
            })
        })
    }
}

impl CanaleTd5 for Stn {
    fn prepara(&mut self) -> Result<String, ErroreTd5> {
        match self.scambia("ATZ", T_RIAVVIO, false) {
            Ok(_) => {}
            // Un secondo tentativo: il primo può cadere su un adattatore occupato.
            Err(ErroreTd5::Timeout(_)) => {
                self.scambia("ATZ", T_RIAVVIO, false)?;
            }
            Err(e) => return Err(e),
        }
        if !self.configura("ATE0")? {
            return Err(ErroreTd5::Adattatore("ATE0 senza «OK»".into()));
        }
        // STI stampa «STN<id> vX.Y.Z»; un ELM327 risponde «?».
        let sti = self.scambia("STI", T_AT, false)?.join(" ");
        let mut identita = if sti.to_uppercase().contains("STN") {
            sti.clone()
        } else {
            String::new()
        };
        let stdi = self.scambia("STDI", T_AT, false)?.join(" ");
        let stdi_valido = !stdi.contains('?')
            && (stdi.to_uppercase().contains("OBDLINK") || stdi.to_uppercase().contains("STN"));
        if identita.is_empty() && !stdi_valido {
            return Err(ErroreTd5::NonStn(format!("STI → «{sti}», STDI → «{stdi}»")));
        }
        if stdi_valido {
            identita = if identita.is_empty() {
                stdi
            } else {
                format!("{stdi} ({identita})")
            };
        }
        for cmd in ["ATL0", "ATS1", "ATH1"] {
            self.configura(cmd)?;
        }
        Ok(identita)
    }

    fn avvia_sessione(&mut self) -> Result<(), ErroreTd5> {
        // L'inizializzazione va sempre nel registro, anche quando avviene
        // durante la lettura a ripetizione (sessione caduta).
        let traccia = self.traccia;
        self.traccia = true;
        let esito = self.avvia_sessione_tracciata();
        self.traccia = traccia;
        esito
    }

    fn richiesta(&mut self, r: RichiestaTd5) -> Result<Risposta, ErroreTd5> {
        let Some(s) = self.strategia else {
            return Err(ErroreTd5::Adattatore("sessione non avviata".into()));
        };
        if r != RichiestaTd5::Mantenimento {
            return self.invia(s, r);
        }
        // Il mantenimento a riposo è uno al secondo: nel registro va solo se
        // non riesce.
        let traccia = self.traccia;
        self.traccia = false;
        let esito = self.invia(s, r);
        self.traccia = traccia;
        let riuscito = matches!(&esito, Ok(Risposta::Positiva(c)) if c.first() == Some(&0x7E));
        if traccia && !riuscito {
            let comando = comando_richiesta(s, r);
            let come = match &esito {
                Ok(risposta) => format!("{risposta:02X?}"),
                Err(e) => e.to_string(),
            };
            self.scrivi_registro(format!("→ {comando}   ← mantenimento non riuscito: {come}"));
        }
        esito
    }

    fn descrizione(&self) -> String {
        self.t.descrizione()
    }

    fn is_demo(&self) -> bool {
        self.t.is_demo()
    }

    fn strategia(&self) -> Option<String> {
        self.strategia.map(|s| s.descrizione().to_string())
    }

    fn imposta_traccia(&mut self, attiva: bool) {
        self.traccia = attiva;
    }

    fn traccia(&self) -> bool {
        self.traccia
    }

    fn imposta_diario(&mut self, diario: Diario) {
        self.diario = diario;
    }

    fn prendi_registro(&mut self) -> Vec<String> {
        std::mem::take(&mut self.registro)
    }

    fn annota(&mut self, riga: String) {
        self.scrivi_registro(riga);
    }

    fn chiudi(&mut self) {
        self.t.close();
    }
}

impl Stn {
    fn avvia_sessione_tracciata(&mut self) -> Result<(), ErroreTd5> {
        let ordine = match self.strategia {
            Some(s) => vec![s],
            None => self.ordine.clone(),
        };
        let mut motivi = Vec::new();
        for (i, s) in ordine.iter().enumerate() {
            if i > 0 {
                self.scrivi_registro(format!(
                    "Attesa di {} ms prima del tentativo successivo",
                    self.pausa.as_millis()
                ));
                std::thread::sleep(self.pausa);
            }
            match self.tenta(*s)? {
                Ok(()) => {
                    self.scrivi_registro(format!("Sessione Td5 aperta con la {}", s.descrizione()));
                    self.strategia = Some(*s);
                    return Ok(());
                }
                Err(m) => {
                    self.scrivi_registro(format!("{} non riuscita: {m}", s.descrizione()));
                    motivi.push(format!("{s:?}: {m}"));
                }
            }
        }
        Err(ErroreTd5::Inizializzazione(motivi.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::td5::richieste::{Chiave, IdLocale};

    #[test]
    fn comandi_per_strategia() {
        let r = RichiestaTd5::SessioneDiagnostica;
        assert_eq!(comando_richiesta(Strategia::A, r), "0210A0");
        assert_eq!(comando_richiesta(Strategia::B, r), "0210A0");
        assert_eq!(comando_richiesta(Strategia::C, r), "STPX h:0210A0, d:, r:1");
        let k = RichiestaTd5::InvioChiave(Chiave::da_seme(0x34A5));
        assert_eq!(comando_richiesta(Strategia::A, k), "04270254D3");
        assert_eq!(
            comando_richiesta(Strategia::C, k),
            "STPX h:042702, d:54D3, r:1"
        );
        assert_eq!(
            comando_richiesta(Strategia::C, RichiestaTd5::Mantenimento),
            "STPX h:023E01, d:, r:1"
        );
        assert_eq!(
            comando_richiesta(Strategia::A, RichiestaTd5::Dati(IdLocale::Giri)),
            "022109"
        );
        assert_eq!(comando_stifi(), "STIFI 25, 25, 8113F7810C");
    }
}
