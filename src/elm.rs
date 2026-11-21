//! Dialogo ELM327: una richiesta alla volta, attesa del prompt, eco, righe
//! multiple, ricerca protocollo, NO DATA e timeout.
//!
//! I comandi inviabili sono solo quelli di [`AtCommand`] e di
//! [`ObdRequest`]: non esiste un'interfaccia pubblica per testo arbitrario.

use std::time::{Duration, Instant};

use crate::obd::frames::{BusKind, EcuMessage, assemble};
use crate::obd::{ObdRequest, supported_from_mask};
use crate::transport::{Scollegato, Transport};

/// Comandi di configurazione dell'adattatore. Agiscono solo sull'adattatore,
/// non sul veicolo, e si annullano togliendo alimentazione.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtCommand {
    /// ATZ: riavvio dell'adattatore.
    Riavvia,
    /// ATE0: eco disattivata.
    EcoSpenta,
    /// ATL0: nessun avanzamento riga.
    SenzaAvanzamentoRiga,
    /// ATS0: risposte senza spazi.
    SenzaSpazi,
    /// ATH1: mostra le intestazioni (serve per sapere quale ECU risponde).
    IntestazioniAttive,
    /// ATAT1: temporizzazione adattiva predefinita.
    TemporizzazioneAdattiva,
    /// ATSP0: scelta automatica del protocollo.
    ProtocolloAutomatico,
    /// ATDPN: numero del protocollo in uso.
    NumeroProtocollo,
    /// ATI: identificazione dell'adattatore.
    Identifica,
    /// ATRV: tensione letta dall'adattatore sul connettore.
    Tensione,
}

impl AtCommand {
    pub fn testo(self) -> &'static str {
        match self {
            AtCommand::Riavvia => "ATZ",
            AtCommand::EcoSpenta => "ATE0",
            AtCommand::SenzaAvanzamentoRiga => "ATL0",
            AtCommand::SenzaSpazi => "ATS0",
            AtCommand::IntestazioniAttive => "ATH1",
            AtCommand::TemporizzazioneAdattiva => "ATAT1",
            AtCommand::ProtocolloAutomatico => "ATSP0",
            AtCommand::NumeroProtocollo => "ATDPN",
            AtCommand::Identifica => "ATI",
            AtCommand::Tensione => "ATRV",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElmError {
    /// Nessun prompt entro il tempo massimo.
    Timeout,
    /// Collegamento con l'adattatore caduto.
    Scollegato(String),
    /// L'adattatore riferisce che nessuna centralina ha risposto.
    NoData,
    /// L'adattatore non è riuscito a stabilire un protocollo con il veicolo.
    ImpossibileCollegarsi,
    /// Errore di bus riferito dall'adattatore (CAN ERROR, BUS BUSY...).
    Bus(String),
    /// Risposta dell'adattatore inattesa (per esempio «?»).
    Adattatore(String),
    /// Risposta non ricomponibile in messaggi.
    Trama(String),
}

impl std::fmt::Display for ElmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElmError::Timeout => write!(f, "nessuna risposta dall'adattatore (timeout)"),
            ElmError::Scollegato(m) => write!(f, "collegamento interrotto: {m}"),
            ElmError::NoData => write!(f, "nessuna risposta dalla centralina (NO DATA)"),
            ElmError::ImpossibileCollegarsi => {
                write!(f, "nessun protocollo trovato (UNABLE TO CONNECT)")
            }
            ElmError::Bus(m) => write!(f, "errore di bus: {m}"),
            ElmError::Adattatore(m) => write!(f, "risposta inattesa dell'adattatore: {m}"),
            ElmError::Trama(m) => write!(f, "risposta non interpretabile: {m}"),
        }
    }
}

impl From<Scollegato> for ElmError {
    fn from(s: Scollegato) -> Self {
        ElmError::Scollegato(s.0)
    }
}

/// Protocollo rilevato dall'adattatore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Protocollo {
    pub numero: u8,
    pub automatico: bool,
    pub kind: BusKind,
    pub descrizione: String,
}

impl Protocollo {
    /// Interpreta la risposta di ATDPN (es. `A6`).
    pub fn da_dpn(risposta: &str) -> Option<Protocollo> {
        let r = risposta.trim().to_uppercase();
        let (automatico, cifra) = match r.strip_prefix('A') {
            Some(resto) if !resto.is_empty() => (true, resto),
            _ => (false, r.as_str()),
        };
        let numero = u8::from_str_radix(cifra, 16).ok()?;
        let (kind, descrizione) = match numero {
            1 => (BusKind::Legacy, "SAE J1850 PWM (41,6 kbit/s)"),
            2 => (BusKind::Legacy, "SAE J1850 VPW (10,4 kbit/s)"),
            3 => (BusKind::Legacy, "ISO 9141-2"),
            4 => (
                BusKind::Legacy,
                "ISO 14230-4 KWP (inizializzazione a 5 baud)",
            ),
            5 => (BusKind::Legacy, "ISO 14230-4 KWP (inizializzazione rapida)"),
            6 => (BusKind::Can11, "ISO 15765-4 CAN, ID a 11 bit, 500 kbit/s"),
            7 => (BusKind::Can29, "ISO 15765-4 CAN, ID a 29 bit, 500 kbit/s"),
            8 => (BusKind::Can11, "ISO 15765-4 CAN, ID a 11 bit, 250 kbit/s"),
            9 => (BusKind::Can29, "ISO 15765-4 CAN, ID a 29 bit, 250 kbit/s"),
            _ => return None,
        };
        Some(Protocollo {
            numero,
            automatico,
            kind,
            descrizione: descrizione.to_string(),
        })
    }
}

/// Risposta a una richiesta OBD, già divisa per centralina.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Risposta {
    /// Risposte positive pertinenti alla richiesta.
    pub messaggi: Vec<EcuMessage>,
    /// Risposte negative: (centralina, codice NRC).
    pub negative: Vec<(String, u8)>,
}

/// Esito di una richiesta visto dagli strati superiori.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Esito {
    Dati(Vec<EcuMessage>),
    /// La centralina rifiuta il servizio o il parametro.
    NonSupportato,
    /// Nessuna centralina ha risposto.
    Mancante,
    /// Timeout, errore di bus o risposta non interpretabile.
    Errore(String),
}

const T_AT: Duration = Duration::from_secs(3);
const T_RIAVVIO: Duration = Duration::from_secs(6);
const T_RICERCA: Duration = Duration::from_secs(25);
const T_OBD: Duration = Duration::from_secs(4);

pub struct Elm {
    t: Box<dyn Transport>,
    kind: BusKind,
    /// Se attivo, gli scambi vengono annotati nel registro.
    pub traccia: bool,
    registro: Vec<String>,
}

impl Elm {
    pub fn new(t: Box<dyn Transport>) -> Self {
        Elm {
            t,
            kind: BusKind::Can11,
            traccia: true,
            registro: Vec::new(),
        }
    }

    pub fn is_demo(&self) -> bool {
        self.t.is_demo()
    }

    pub fn descrizione(&self) -> String {
        self.t.descrizione()
    }

    pub fn kind(&self) -> BusKind {
        self.kind
    }

    /// Preleva le righe di registro accumulate.
    pub fn prendi_registro(&mut self) -> Vec<String> {
        std::mem::take(&mut self.registro)
    }

    pub fn chiudi(&mut self) {
        self.t.close();
    }

    /// Invia un comando e raccoglie le righe fino al prompt `>`.
    fn scambia(
        &mut self,
        cmd: &str,
        attesa: Duration,
        riservato: bool,
    ) -> Result<Vec<String>, ElmError> {
        // Scarta eventuali residui di scambi precedenti.
        while !self.t.read(Duration::ZERO)?.is_empty() {}

        self.t.write(format!("{cmd}\r").as_bytes())?;
        let scadenza = Instant::now() + attesa;
        let mut buf: Vec<u8> = Vec::new();
        while !buf.contains(&b'>') {
            let ora = Instant::now();
            if ora >= scadenza {
                if self.traccia {
                    self.registro.push(format!("→ {cmd}   ← (timeout)"));
                }
                return Err(ElmError::Timeout);
            }
            let pezzo = self
                .t
                .read((scadenza - ora).min(Duration::from_millis(100)))?;
            // L'ELM327 può inserire byte nulli: vanno ignorati.
            buf.extend(pezzo.into_iter().filter(|b| *b != 0));
        }
        let fine = buf.iter().position(|b| *b == b'>').unwrap_or(buf.len());
        let testo = String::from_utf8_lossy(&buf[..fine]).to_string();
        let senza_spazi = |s: &str| {
            s.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_uppercase()
        };
        let eco = senza_spazi(cmd);
        let righe: Vec<String> = testo
            .split(['\r', '\n'])
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty() && senza_spazi(r) != eco)
            .collect();
        if self.traccia {
            if riservato {
                self.registro
                    .push(format!("→ {cmd}   ← (risposta omessa dal registro)"));
            } else {
                self.registro
                    .push(format!("→ {cmd}   ← {}", righe.join(" | ")));
            }
        }
        Ok(righe)
    }

    /// Invia un comando di configurazione e restituisce il testo di risposta.
    pub fn at(&mut self, cmd: AtCommand) -> Result<String, ElmError> {
        let attesa = if cmd == AtCommand::Riavvia {
            T_RIAVVIO
        } else {
            T_AT
        };
        let righe = self.scambia(cmd.testo(), attesa, false)?;
        Ok(righe.join(" "))
    }

    fn at_ok(&mut self, cmd: AtCommand, obbligatorio: bool) -> Result<(), ElmError> {
        let r = self.at(cmd)?;
        if r.to_uppercase().contains("OK") || !obbligatorio {
            Ok(())
        } else {
            Err(ElmError::Adattatore(format!("{} → «{r}»", cmd.testo())))
        }
    }

    /// Verifica che l'adattatore risponda come un ELM327 e lo configura.
    /// Restituisce la stringa di identificazione.
    pub fn inizializza(&mut self) -> Result<String, ElmError> {
        let mut identita = match self.at(AtCommand::Riavvia) {
            Ok(r) => r,
            // Un secondo tentativo: il primo può cadere su un adattatore occupato.
            Err(ElmError::Timeout) => self.at(AtCommand::Riavvia)?,
            Err(e) => return Err(e),
        };
        self.at_ok(AtCommand::EcoSpenta, true)?;
        if !identita.to_uppercase().contains("ELM") {
            identita = self.at(AtCommand::Identifica)?;
        }
        if identita.is_empty() || identita.contains('?') {
            return Err(ElmError::Adattatore(
                "il dispositivo non si identifica come ELM327".into(),
            ));
        }
        self.at_ok(AtCommand::SenzaAvanzamentoRiga, false)?;
        self.at_ok(AtCommand::SenzaSpazi, false)?;
        self.at_ok(AtCommand::IntestazioniAttive, true)?;
        self.at_ok(AtCommand::TemporizzazioneAdattiva, false)?;
        self.at_ok(AtCommand::ProtocolloAutomatico, true)?;
        Ok(identita)
    }

    /// Tensione al connettore secondo l'adattatore (es. `12.4V`), se disponibile.
    pub fn tensione(&mut self) -> Result<Option<String>, ElmError> {
        match self.at(AtCommand::Tensione) {
            Ok(r) if r.to_uppercase().ends_with('V') && !r.contains('?') => Ok(Some(r)),
            Ok(_) | Err(ElmError::Timeout) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Prima richiesta al veicolo: fa cercare il protocollo all'adattatore e
    /// restituisce il protocollo trovato con i PID 01–20 dichiarati da ogni ECU.
    pub fn collega_centralina(&mut self) -> Result<(Protocollo, Vec<EcuMessage>), ElmError> {
        let richiesta = ObdRequest::CurrentData(0x00);
        let righe = self.scambia(&richiesta.command(), T_RICERCA, false)?;
        let dati = classifica(righe)?;
        let dpn = self.at(AtCommand::NumeroProtocollo)?;
        let protocollo = Protocollo::da_dpn(&dpn).ok_or_else(|| {
            ElmError::Adattatore(format!("protocollo non OBD-II o non riconosciuto: «{dpn}»"))
        })?;
        self.kind = protocollo.kind;
        let messaggi: Vec<EcuMessage> = assemble(&dati, self.kind)
            .map_err(|e| ElmError::Trama(e.to_string()))?
            .into_iter()
            .filter(|m| richiesta.matches(&m.data) && m.data.len() >= 6)
            .collect();
        if messaggi.is_empty() {
            return Err(ElmError::NoData);
        }
        Ok((protocollo, messaggi))
    }

    /// Esegue una richiesta OBD di sola lettura.
    pub fn richiesta(&mut self, r: ObdRequest) -> Result<Risposta, ElmError> {
        let riservato = r == ObdRequest::Vin;
        let righe = self.scambia(&r.command(), T_OBD, riservato)?;
        let dati = classifica(righe)?;
        let tutti = assemble(&dati, self.kind).map_err(|e| ElmError::Trama(e.to_string()))?;
        let mut risposta = Risposta::default();
        for m in tutti {
            if r.matches(&m.data) {
                risposta.messaggi.push(m);
            } else if m.data.len() >= 3 && m.data[0] == 0x7F && m.data[1] == r.service() {
                risposta.negative.push((m.ecu, m.data[2]));
            }
        }
        Ok(risposta)
    }

    /// Come [`Elm::richiesta`], ma distingue solo fra dati, non supportato,
    /// risposta mancante ed errore. `Err` significa collegamento caduto.
    pub fn interroga(&mut self, r: ObdRequest) -> Result<Esito, Scollegato> {
        match self.richiesta(r) {
            Ok(risposta) if !risposta.messaggi.is_empty() => Ok(Esito::Dati(risposta.messaggi)),
            Ok(risposta) => {
                // 0x11 servizio non supportato, 0x12 sottofunzione non supportata,
                // 0x31 richiesta fuori intervallo.
                if risposta
                    .negative
                    .iter()
                    .any(|(_, c)| matches!(c, 0x11 | 0x12 | 0x31))
                {
                    Ok(Esito::NonSupportato)
                } else if !risposta.negative.is_empty() {
                    Ok(Esito::Mancante)
                } else {
                    Ok(Esito::Errore(
                        "risposta non pertinente alla richiesta".into(),
                    ))
                }
            }
            Err(ElmError::NoData) => Ok(Esito::Mancante),
            Err(ElmError::Scollegato(m)) => Err(Scollegato(m)),
            Err(e) => Ok(Esito::Errore(e.to_string())),
        }
    }

    /// Legge le maschere dei PID supportati oltre il 20 per ogni centralina.
    /// `iniziali` sono i messaggi di risposta a 0100.
    pub fn pid_supportati(
        &mut self,
        iniziali: &[EcuMessage],
    ) -> Result<std::collections::BTreeMap<String, std::collections::BTreeSet<u8>>, Scollegato>
    {
        let mut mappa: std::collections::BTreeMap<String, std::collections::BTreeSet<u8>> =
            Default::default();
        for m in iniziali {
            mappa
                .entry(m.ecu.clone())
                .or_default()
                .extend(supported_from_mask(0x00, &m.data[2..]));
        }
        for base in [0x20u8, 0x40, 0x60, 0x80, 0xA0, 0xC0] {
            if !mappa.values().any(|s| s.contains(&base)) {
                break;
            }
            match self.interroga(ObdRequest::CurrentData(base))? {
                Esito::Dati(msg) => {
                    for m in msg.iter().filter(|m| m.data.len() >= 6) {
                        mappa
                            .entry(m.ecu.clone())
                            .or_default()
                            .extend(supported_from_mask(base, &m.data[2..]));
                    }
                }
                _ => break,
            }
        }
        Ok(mappa)
    }
}

/// Separa le righe di dati dai messaggi testuali dell'adattatore.
fn classifica(righe: Vec<String>) -> Result<Vec<String>, ElmError> {
    let mut dati = Vec::new();
    for riga in righe {
        let r = riga.to_uppercase();
        if r.starts_with("SEARCHING") || r == "BUS INIT: OK" || r == "OK" {
            continue;
        }
        if r.contains("NO DATA") {
            return Err(ElmError::NoData);
        }
        if r.contains("UNABLE TO CONNECT") {
            return Err(ElmError::ImpossibileCollegarsi);
        }
        if r == "?" {
            return Err(ElmError::Adattatore(
                "comando non riconosciuto («?»)".into(),
            ));
        }
        const ERRORI: [&str; 11] = [
            "CAN ERROR",
            "BUS ERROR",
            "BUS BUSY",
            "BUS INIT",
            "FB ERROR",
            "DATA ERROR",
            "RX ERROR",
            "BUFFER FULL",
            "STOPPED",
            "LV RESET",
            "ACT ALERT",
        ];
        if ERRORI.iter().any(|e| r.contains(e)) || r.starts_with("ERR") {
            return Err(ElmError::Bus(riga));
        }
        dati.push(riga);
    }
    if dati.is_empty() {
        return Err(ElmError::Adattatore("risposta vuota".into()));
    }
    Ok(dati)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::copione::{Battuta, Copione};

    fn elm(c: Copione) -> Elm {
        Elm::new(Box::new(c))
    }

    fn pronto(mut c: Copione) -> Elm {
        c.testo("0100", "SEARCHING...\r7E8 06 41 00 BE 3F A8 13 AA\r\r>");
        let mut e = elm(c);
        e.inizializza().unwrap();
        e.collega_centralina().unwrap();
        e
    }

    #[test]
    fn inizializzazione_con_eco_e_frammenti_da_3_byte() {
        let mut e = elm(Copione::con_inizializzazione(3));
        assert_eq!(e.inizializza().unwrap(), "ELM327 v1.5");
    }

    #[test]
    fn ricerca_protocollo_e_ecu() {
        let mut c = Copione::con_inizializzazione(7);
        c.testo("0100", "SEARCHING...\r7E8 06 41 00 BE 3F A8 13 AA\r\r>");
        let mut e = elm(c);
        e.inizializza().unwrap();
        let (p, m) = e.collega_centralina().unwrap();
        assert_eq!(p.numero, 6);
        assert!(p.automatico);
        assert_eq!(p.kind, BusKind::Can11);
        assert!(p.descrizione.contains("11 bit") && p.descrizione.contains("500"));
        assert_eq!(m[0].ecu, "7E8");
    }

    #[test]
    fn centralina_assente() {
        let mut c = Copione::con_inizializzazione(20);
        c.testo("0100", "SEARCHING...\rUNABLE TO CONNECT\r\r>");
        let mut e = elm(c);
        e.inizializza().unwrap();
        assert_eq!(
            e.collega_centralina().unwrap_err(),
            ElmError::ImpossibileCollegarsi
        );
    }

    #[test]
    fn adattatore_muto() {
        let mut c = Copione::new(20);
        c.sequenza("ATZ", vec![Battuta::Silenzio]);
        let mut e = elm(c);
        // Tempi ridotti non disponibili: si verifica solo che l'errore sia un timeout.
        let r = e.scambia("ATZ", Duration::from_millis(60), false);
        assert_eq!(r.unwrap_err(), ElmError::Timeout);
    }

    #[test]
    fn dispositivo_non_elm() {
        let mut c = Copione::new(20);
        c.testo("ATZ", "?\r\r>");
        c.testo("ATE0", "OK\r\r>");
        c.testo("ATI", "?\r\r>");
        let mut e = elm(c);
        assert!(matches!(e.inizializza(), Err(ElmError::Adattatore(_))));
    }

    #[test]
    fn no_data_e_non_supportato() {
        let mut c = Copione::con_inizializzazione(20);
        c.testo("010C", "NO DATA\r\r>");
        c.testo("0A", "7E8 03 7F 0A 11\r\r>");
        c.testo("0105", "CAN ERROR\r\r>");
        let mut e = pronto(c);
        assert_eq!(
            e.interroga(ObdRequest::CurrentData(0x0C)).unwrap(),
            Esito::Mancante
        );
        assert_eq!(
            e.interroga(ObdRequest::PermanentDtcs).unwrap(),
            Esito::NonSupportato
        );
        assert!(matches!(
            e.interroga(ObdRequest::CurrentData(0x05)).unwrap(),
            Esito::Errore(_)
        ));
    }

    #[test]
    fn risposte_da_piu_centraline() {
        let mut c = Copione::con_inizializzazione(5);
        c.testo("010C", "7E8 04 41 0C 21 40\r7E9 04 41 0C 21 44\r\r>");
        let mut e = pronto(c);
        let Esito::Dati(m) = e.interroga(ObdRequest::CurrentData(0x0C)).unwrap() else {
            panic!()
        };
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].ecu, "7E9");
    }

    #[test]
    fn caduta_del_collegamento() {
        let mut c = Copione::con_inizializzazione(20);
        c.sequenza("010C", vec![Battuta::Caduta]);
        let mut e = pronto(c);
        assert!(e.interroga(ObdRequest::CurrentData(0x0C)).is_err());
    }

    #[test]
    fn byte_nulli_e_avanzamento_riga() {
        let mut c = Copione::con_inizializzazione(4);
        c.testo("010D", "\0\r\n7E8 03 41 0D 45\r\n\r\n>");
        let mut e = pronto(c);
        let Esito::Dati(m) = e.interroga(ObdRequest::CurrentData(0x0D)).unwrap() else {
            panic!()
        };
        assert_eq!(m[0].data, vec![0x41, 0x0D, 0x45]);
    }

    #[test]
    fn maschere_oltre_il_20() {
        let mut c = Copione::con_inizializzazione(20);
        c.testo("0120", "7E8 06 41 20 90 00 00 01\r\r>");
        c.testo("0140", "7E8 06 41 40 50 00 00 00\r\r>");
        c.testo("0100", "7E8 06 41 00 BE 3F A8 13\r\r>");
        let mut e = elm(c);
        e.inizializza().unwrap();
        let (_, iniziali) = e.collega_centralina().unwrap();
        let mappa = e.pid_supportati(&iniziali).unwrap();
        let s = &mappa["7E8"];
        assert!(s.contains(&0x0C) && s.contains(&0x21) && s.contains(&0x24));
        assert!(s.contains(&0x42) && s.contains(&0x44) && !s.contains(&0x60));
    }

    #[test]
    fn il_registro_non_contiene_il_vin() {
        let mut c = Copione::con_inizializzazione(20);
        c.testo(
            "0902",
            "7E8 10 14 49 02 01 56 46 33\r7E8 21 41 42 43 44 45 46 47\r7E8 22 48 49 4A 4B 4C 4D 4E\r\r>",
        );
        let mut e = pronto(c);
        e.prendi_registro();
        let r = e.richiesta(ObdRequest::Vin).unwrap();
        assert_eq!(r.messaggi[0].data.len(), 20);
        let registro = e.prendi_registro().join("\n");
        assert!(registro.contains("omessa") && !registro.contains("56 46 33"));
    }
}
