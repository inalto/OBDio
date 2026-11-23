//! Adattatore OBDLink simulato collegato a una centralina Td5 simulata.
//!
//! Serve alle verifiche e alla «Demo Td5». Risponde ai comandi AT e ST usati
//! da [`super::stn::Stn`] come li descrive il manuale OBDLink, con il
//! comportamento IPOTIZZATO per i messaggi senza intestazione: è proprio ciò
//! che non si può sapere senza l'adattatore. Quali strategie «funzionano» si
//! sceglie, così le verifiche coprono anche i ripieghi. La centralina usa il
//! vero algoritmo seme/chiave, può chiudere la sessione dopo un silenzio,
//! rifiutare la chiave o certi identificativi.
//!
//! Questo simulatore dimostra solo che OBDio è coerente con le proprie
//! ipotesi; non sostituisce una prova con l'adattatore e il veicolo.

use std::collections::{BTreeSet, VecDeque};
use std::f64::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::chiave::chiave_da_seme;
use super::frame;
use super::richieste::IdLocale;
use crate::transport::{Scollegato, Transport};

const VIN_PROVA: &[u8; 17] = b"TDCINQUE123456789";
const VIN_DEMO: &[u8; 17] = b"DEMO0000000000TD5";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum StatoEcu {
    /// Nessuna sessione (mai svegliata o caduta).
    Spenta,
    /// Dopo StartCommunication.
    Sveglia,
    /// Dopo `10 A0`.
    Sessione,
    /// Dopo la chiave giusta.
    Autenticata,
}

/// Comandi e messaggi ricevuti, e interruttori per le verifiche.
#[derive(Clone, Default)]
pub struct ControlloSimulatore {
    /// Righe di comando ricevute dall'adattatore, nell'ordine.
    pub comandi: Arc<Mutex<Vec<String>>>,
    /// Corpi dei messaggi arrivati alla centralina.
    pub messaggi: Arc<Mutex<Vec<Vec<u8>>>>,
    /// Se impostato, la centralina chiude la sessione (una volta).
    pub caduta_sessione: Arc<AtomicBool>,
    /// Se impostato, il collegamento con l'adattatore cade.
    pub interrompi: Arc<AtomicBool>,
}

impl ControlloSimulatore {
    pub fn comandi(&self) -> Vec<String> {
        self.comandi
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn messaggi(&self) -> Vec<Vec<u8>> {
        self.messaggi
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Quante inizializzazioni rapide ha ricevuto l'adattatore.
    pub fn inizializzazioni(&self) -> usize {
        self.comandi()
            .iter()
            .filter(|c| c.to_uppercase().starts_with("STIFI"))
            .count()
    }
}

pub struct SimulatoreTd5 {
    ingresso: Vec<u8>,
    uscita: VecDeque<Vec<u8>>,
    pronto_a: Instant,
    latenza: Duration,
    // Adattatore.
    stn: bool,
    eco: bool,
    spazi: bool,
    intestazioni: bool,
    protocollo: u8,
    mantenimento_adattatore: bool,
    funziona: [bool; 3],
    // Centralina.
    ecu: StatoEcu,
    sveglia_con: u8,
    mantenimento_al_risveglio: bool,
    seme: Option<u16>,
    prossimo_seme: u16,
    chiave_errata: bool,
    scadenza: Option<Duration>,
    ultimo_messaggio: Instant,
    senza: BTreeSet<IdLocale>,
    guasti: Vec<(u8, u8)>,
    inizializzazioni: usize,
    inizializzazioni_massime: Option<usize>,
    demo: bool,
    t0: Instant,
    chiuso: bool,
    controllo: ControlloSimulatore,
}

impl Default for SimulatoreTd5 {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatoreTd5 {
    /// Per le verifiche: risposte immediate, valori fissi (gli esempi di
    /// ricerca/td5/proto.md), tutte le strategie funzionanti, nessuna
    /// scadenza della sessione.
    pub fn new() -> Self {
        let ora = Instant::now();
        SimulatoreTd5 {
            ingresso: Vec::new(),
            uscita: VecDeque::new(),
            pronto_a: ora,
            latenza: Duration::ZERO,
            stn: true,
            eco: true,
            spazi: true,
            intestazioni: false,
            protocollo: 0,
            mantenimento_adattatore: true,
            funziona: [true; 3],
            ecu: StatoEcu::Spenta,
            sveglia_con: 0,
            mantenimento_al_risveglio: false,
            seme: None,
            prossimo_seme: 0x34A5,
            chiave_errata: false,
            scadenza: None,
            ultimo_messaggio: ora,
            senza: BTreeSet::new(),
            guasti: vec![(1, 3), (31, 0)],
            inizializzazioni: 0,
            inizializzazioni_massime: None,
            demo: false,
            t0: ora,
            chiuso: false,
            controllo: ControlloSimulatore::default(),
        }
    }

    /// Per la «Demo Td5»: tempi realistici, valori che variano nel tempo,
    /// sessione che cade dopo 2 s di silenzio come su una centralina vera.
    pub fn demo() -> Self {
        let mut s = Self::new();
        s.demo = true;
        s.latenza = Duration::from_millis(40);
        s.scadenza = Some(Duration::from_secs(2));
        s.guasti = vec![(1, 3), (19, 0)];
        s
    }

    /// Quali strategie funzionano su questo adattatore simulato.
    pub fn con_strategie(mut self, a: bool, b: bool, c: bool) -> Self {
        self.funziona = [a, b, c];
        self
    }

    /// Adattatore di tipo ELM327 senza chip STN: «?» a ogni comando ST.
    pub fn senza_stn(mut self) -> Self {
        self.stn = false;
        self
    }

    /// La centralina rifiuta qualunque chiave.
    pub fn con_chiave_errata(mut self) -> Self {
        self.chiave_errata = true;
        self
    }

    /// La centralina chiude la sessione dopo `d` senza messaggi.
    pub fn con_scadenza(mut self, d: Duration) -> Self {
        self.scadenza = Some(d);
        self
    }

    /// Dopo `n` inizializzazioni rapide la centralina non risponde più.
    pub fn con_inizializzazioni_massime(mut self, n: usize) -> Self {
        self.inizializzazioni_massime = Some(n);
        self
    }

    /// Identificativi che la centralina rifiuta (`7F 21 31`).
    pub fn senza_identificativi(mut self, id: &[IdLocale]) -> Self {
        self.senza.extend(id.iter().copied());
        self
    }

    /// Guasti accesi nella mappa: (byte da 0, bit da 0).
    pub fn con_guasti(mut self, guasti: &[(u8, u8)]) -> Self {
        self.guasti = guasti.to_vec();
        self
    }

    pub fn controllo(&self) -> ControlloSimulatore {
        self.controllo.clone()
    }

    fn byte_stampati(&self, byte: &[u8]) -> String {
        // Con ATH1 l'adattatore stampa tutto il messaggio (lunghezza e
        // checksum compresi); con ATH0 solo il corpo. Ipotesi non verificata.
        let parte = if self.intestazioni || byte.len() < 3 {
            byte
        } else {
            &byte[1..byte.len() - 1]
        };
        if self.spazi {
            frame::esadecimale(parte)
        } else {
            frame::esadecimale_compatto(parte)
        }
    }

    fn rispondi(&mut self, grezzo: &str) -> String {
        let cmd: String = grezzo
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_uppercase();
        self.controllo
            .comandi
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(grezzo.to_string());
        let corpo = self.esegui(&cmd);
        let eco = if self.eco {
            format!("{grezzo}\r")
        } else {
            String::new()
        };
        format!("{eco}{corpo}\r\r>")
    }

    fn esegui(&mut self, cmd: &str) -> String {
        let ok = || "OK".to_string();
        if cmd == "ATZ" {
            self.eco = true;
            self.spazi = true;
            self.intestazioni = false;
            self.protocollo = 0;
            self.mantenimento_adattatore = true;
            return if self.stn {
                "\rELM327 v1.4b".into()
            } else {
                "\rELM327 v1.5".into()
            };
        }
        if let Some(resto) = cmd.strip_prefix("AT") {
            match resto {
                "E0" => self.eco = false,
                "E1" => self.eco = true,
                "S0" => self.spazi = false,
                "S1" => self.spazi = true,
                "H0" => self.intestazioni = false,
                "H1" => self.intestazioni = true,
                "L0" | "L1" => {}
                "SW00" => self.mantenimento_adattatore = false,
                r if r.starts_with("WM") => {}
                _ => return "?".into(),
            }
            return ok();
        }
        if cmd.starts_with("ST") {
            if !self.stn {
                return "?".into();
            }
            return self.comando_st(cmd);
        }
        // Richiesta esadecimale semplice.
        let Some(byte) = frame::da_esadecimale(cmd) else {
            return "?".into();
        };
        let accetta = self.protocollo == 21
            && ((self.sveglia_con == 21 && self.funziona[0])
                || (self.sveglia_con == 25 && self.funziona[1]));
        if !accetta {
            // In KWP l'adattatore aggiungerebbe la propria intestazione: la
            // centralina Td5 non capisce e tace.
            return "NO DATA".into();
        }
        self.alla_centralina(byte)
    }

    fn comando_st(&mut self, cmd: &str) -> String {
        if cmd == "STI" {
            return "STN2255 v5.6.19".into();
        }
        if cmd == "STDI" {
            return "OBDLink CX r1.0".into();
        }
        if let Some(p) = cmd.strip_prefix("STP")
            && p.len() == 2
            && let Ok(n) = p.parse::<u8>()
        {
            if !(21..=25).contains(&n) {
                return "?".into();
            }
            // Strategia B non funzionante: cambiando protocollo l'adattatore
            // chiude la linea e la sessione appena aperta si perde.
            if self.protocollo == 25 && n == 21 && self.ecu > StatoEcu::Spenta && !self.funziona[1]
            {
                self.ecu = StatoEcu::Spenta;
            }
            self.protocollo = n;
            return "OK".into();
        }
        for prefisso in ["STPBR", "STPTO", "STPTRQ", "STIP1X"] {
            if cmd.starts_with(prefisso) {
                return "OK".into();
            }
        }
        if let Some(param) = cmd.strip_prefix("STIFI") {
            return self.stifi(param);
        }
        if let Some(param) = cmd.strip_prefix("STPX") {
            return self.stpx(param);
        }
        "?".into()
    }

    fn stifi(&mut self, param: &str) -> String {
        let parti: Vec<&str> = param.split(',').collect();
        let [basso, alto, messaggio] = parti.as_slice() else {
            return "?".into();
        };
        if *basso != "25" || *alto != "25" {
            return "BUS INIT: ERROR".into();
        }
        let ammesso = self.protocollo == 25 || (self.protocollo == 21 && self.funziona[0]);
        if !ammesso {
            return "?".into();
        }
        if frame::da_esadecimale(messaggio).as_deref()
            != Some(&super::richieste::INIZIO_COMUNICAZIONE)
        {
            return "NO DATA".into();
        }
        self.inizializzazioni += 1;
        if self
            .inizializzazioni_massime
            .is_some_and(|n| self.inizializzazioni > n)
        {
            return "BUS INIT: ...ERROR".into();
        }
        self.ecu = StatoEcu::Sveglia;
        self.sveglia_con = self.protocollo;
        self.mantenimento_al_risveglio = self.protocollo == 25 && self.mantenimento_adattatore;
        self.seme = None;
        self.ultimo_messaggio = Instant::now();
        self.byte_stampati(&[0x03, 0xC1, 0x57, 0x8F, 0xAA])
    }

    fn stpx(&mut self, param: &str) -> String {
        let mut h = Vec::new();
        let mut d = Vec::new();
        for p in param.split(',') {
            if let Some(x) = p.strip_prefix("H:") {
                h = frame::da_esadecimale(x).unwrap_or_default();
            } else if let Some(x) = p.strip_prefix("D:") {
                d = if x.is_empty() {
                    Vec::new()
                } else {
                    match frame::da_esadecimale(x) {
                        Some(b) => b,
                        None => return "?".into(),
                    }
                };
            }
        }
        if h.is_empty() {
            return "?".into();
        }
        let funziona = self.protocollo == 25
            && self.sveglia_con == 25
            && self.funziona[2]
            && !self.mantenimento_al_risveglio;
        if !funziona {
            return "NO DATA".into();
        }
        h.extend(d);
        self.alla_centralina(h)
    }

    /// L'adattatore aggiunge il checksum e consegna il messaggio.
    fn alla_centralina(&mut self, mut byte: Vec<u8>) -> String {
        byte.push(frame::checksum(&byte));
        match self.centralina(&byte) {
            Some(corpo) => self.byte_stampati(&frame::componi(&corpo)),
            None => "NO DATA".into(),
        }
    }

    fn centralina(&mut self, trama: &[u8]) -> Option<Vec<u8>> {
        if self
            .controllo
            .caduta_sessione
            .swap(false, Ordering::Relaxed)
        {
            self.ecu = StatoEcu::Spenta;
        }
        if let Some(s) = self.scadenza
            && self.ultimo_messaggio.elapsed() > s
        {
            self.ecu = StatoEcu::Spenta;
        }
        if self.ecu == StatoEcu::Spenta {
            return None;
        }
        let corpo = frame::verifica(trama).ok()?.to_vec();
        self.controllo
            .messaggi
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(corpo.clone());
        self.ultimo_messaggio = Instant::now();
        let autenticata = self.ecu == StatoEcu::Autenticata;
        Some(match corpo.as_slice() {
            [0x10, 0xA0] => {
                self.ecu = self.ecu.max(StatoEcu::Sessione);
                vec![0x50, 0xA0]
            }
            [0x27, 0x01] if self.ecu >= StatoEcu::Sessione => {
                let seme = self.prossimo_seme;
                self.prossimo_seme = seme.wrapping_mul(31).wrapping_add(0x0107);
                self.seme = Some(seme);
                let [a, b] = seme.to_be_bytes();
                vec![0x67, 0x01, a, b]
            }
            [0x27, 0x02, a, b] => match self.seme.take() {
                Some(s)
                    if !self.chiave_errata && chiave_da_seme(s) == u16::from_be_bytes([*a, *b]) =>
                {
                    self.ecu = StatoEcu::Autenticata;
                    vec![0x67, 0x02]
                }
                _ => vec![0x7F, 0x27, 0x35],
            },
            [0x3E, ..] => vec![0x7E],
            [s @ (0x21 | 0x1A), ..] if !autenticata => vec![0x7F, *s, 0x33],
            [0x21, 0x3B] => {
                let mut v = vec![0x61, 0x3B];
                let mut mappa = [0u8; 35];
                for (byte, bit) in &self.guasti {
                    if let Some(m) = mappa.get_mut(*byte as usize) {
                        *m |= 1 << bit;
                    }
                }
                v.extend(mappa);
                v
            }
            [0x21, 0x32] => {
                let mut v = vec![0x61, 0x32];
                v.extend_from_slice(if self.demo {
                    b"MAPPA DEMO"
                } else {
                    b"MAPPA PROVA"
                });
                v
            }
            [0x1A, 0x87] => {
                let mut v = vec![0x5A, 0x87];
                v.extend_from_slice(if self.demo { VIN_DEMO } else { VIN_PROVA });
                v
            }
            [0x1A, 0x9A] => {
                let mut v = vec![0x5A, 0x9A];
                v.extend_from_slice(if self.demo { b"SIMULATA" } else { b"PROVA" });
                v
            }
            [0x21, id] => match IdLocale::da_codice(*id) {
                Some(i) if !self.senza.contains(&i) => {
                    let mut v = vec![0x61, *id];
                    v.extend(self.dati(i));
                    v
                }
                _ => vec![0x7F, 0x21, 0x31],
            },
            // Qualunque altro servizio (anche quelli che OBDio non invia mai).
            [s, ..] => vec![0x7F, *s, 0x11],
            [] => return None,
        })
    }

    /// Dati di un identificativo: gli esempi di proto.md, oppure valori
    /// sintetici che variano nel tempo (demo).
    fn dati(&self, id: IdLocale) -> Vec<u8> {
        if !self.demo {
            let h = |s: &str| frame::da_esadecimale(s).unwrap_or_default();
            return match id {
                IdLocale::Giri => h("02E8"),
                IdLocale::Velocita => h("00"),
                IdLocale::Batteria => h("36433639"),
                IdLocale::Temperature => h("0D6E02BA0B940AB50B740B610D2C0469"),
                IdLocale::Pedale => h("02811144115F00001388"),
                IdLocale::Collettore => h("27E92806002C0020"),
                IdLocale::Interruttori => h("FFBF"),
                IdLocale::PressioneAmbiente => h("26DB26DB"),
                IdLocale::Egr => h("0BB8"),
                IdLocale::Wastegate => h("1388"),
                IdLocale::Iniettori => h("00020000FFFD0004FFFD"),
            };
        }
        let t = self.t0.elapsed().as_secs_f64();
        // Ciclo di 60 s: minimo, marcia a 50 km/h fra 20 e 40 s, minimo.
        let fase = t % 60.0;
        let in_marcia = (20.0..40.0).contains(&fase);
        let onda = (t * TAU / 7.0).sin();
        let parola = |v: f64| (v.round().clamp(0.0, 65535.0) as u16).to_be_bytes();
        let kelvin = |c: f64| parola(c * 10.0 + 2732.0);
        let giri = if in_marcia {
            1950.0 + 150.0 * onda
        } else {
            760.0 + 15.0 * onda
        };
        let refrigerante = (55.0 + t * 0.4).min(88.0);
        let gasolio = (25.0 + t * 0.15).min(45.0);
        let mappa = if in_marcia {
            165.0 + 10.0 * onda
        } else {
            101.5
        };
        let pedale = if in_marcia { 1.9 + 0.2 * onda } else { 0.64 };
        match id {
            IdLocale::Giri => parola(giri).to_vec(),
            IdLocale::Velocita => vec![if in_marcia { 50 } else { 0 }],
            IdLocale::Batteria => {
                let mut v = parola(13_900.0 + 60.0 * onda).to_vec();
                v.extend(parola(13_880.0 + 60.0 * onda));
                v
            }
            IdLocale::Temperature => {
                let mut v = Vec::new();
                v.extend(kelvin(refrigerante));
                v.extend([0x02, 0xBA]);
                v.extend(kelvin(19.0 + onda * 0.3));
                v.extend([0x0A, 0xB5]);
                v.extend(kelvin(17.0));
                v.extend([0x0B, 0x61]);
                v.extend(kelvin(gasolio));
                v.extend([0x04, 0x69]);
                v
            }
            IdLocale::Pedale => {
                let mut v = Vec::new();
                v.extend(parola(pedale * 1000.0));
                v.extend(parola((5.06 - pedale) * 1000.0));
                v.extend(parola((5.09 - pedale) * 1000.0));
                v.extend([0x00, 0x00]);
                v.extend(parola(5000.0));
                v
            }
            IdLocale::Collettore => {
                let mut v = parola(mappa * 100.0).to_vec();
                v.extend(parola(mappa * 100.0 + 25.0));
                v.extend([0x00, 0x2C, 0x00, 0x20]);
                v
            }
            IdLocale::Interruttori => vec![0xFF, 0xBF],
            IdLocale::PressioneAmbiente => {
                let mut v = parola(9947.0).to_vec();
                v.extend(parola(9947.0));
                v
            }
            IdLocale::Egr => parola(if in_marcia { 0.0 } else { 3000.0 }).to_vec(),
            IdLocale::Wastegate => parola(if in_marcia { 6500.0 } else { 1500.0 }).to_vec(),
            IdLocale::Iniettori => {
                let mut v = Vec::new();
                for (i, base) in [2i16, 0, -3, 4, -3].into_iter().enumerate() {
                    let x = base + ((t * 0.5 + i as f64).sin() * 1.5).round() as i16;
                    v.extend(x.to_be_bytes());
                }
                v
            }
        }
    }

    fn caduto(&self) -> bool {
        self.chiuso || self.controllo.interrompi.load(Ordering::Relaxed)
    }
}

impl Transport for SimulatoreTd5 {
    fn write(&mut self, data: &[u8]) -> Result<(), Scollegato> {
        if self.caduto() {
            return Err(Scollegato("collegamento simulato interrotto".into()));
        }
        self.ingresso.extend_from_slice(data);
        while let Some(pos) = self.ingresso.iter().position(|b| *b == b'\r') {
            let riga: Vec<u8> = self.ingresso.drain(..=pos).collect();
            let cmd = String::from_utf8_lossy(&riga[..riga.len() - 1]).to_string();
            let risposta = self.rispondi(cmd.trim());
            // Pezzi da 20 byte, come le notifiche BLE.
            for pezzo in risposta.as_bytes().chunks(20) {
                self.uscita.push_back(pezzo.to_vec());
            }
            self.pronto_a = Instant::now() + self.latenza;
        }
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Scollegato> {
        if self.caduto() {
            return Err(Scollegato("collegamento simulato interrotto".into()));
        }
        if self.uscita.is_empty() {
            std::thread::sleep(timeout.min(Duration::from_millis(2)));
            return Ok(Vec::new());
        }
        let ora = Instant::now();
        if ora < self.pronto_a {
            let attesa = self.pronto_a - ora;
            if attesa > timeout {
                std::thread::sleep(timeout);
                return Ok(Vec::new());
            }
            std::thread::sleep(attesa);
        }
        Ok(self.uscita.pop_front().unwrap_or_default())
    }

    fn descrizione(&self) -> String {
        if self.demo {
            "Simulatore OBDLink + Td5 (dati sintetici)".into()
        } else {
            "Simulatore OBDLink + Td5 (verifiche)".into()
        }
    }

    /// Sempre vero: anche nelle verifiche i dati sono sintetici.
    fn is_demo(&self) -> bool {
        true
    }

    fn close(&mut self) {
        self.chiuso = true;
    }
}
