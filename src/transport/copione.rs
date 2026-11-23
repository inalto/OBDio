//! Trasporto a copione per le verifiche: a ogni comando risponde con un testo
//! prestabilito, spezzato in frammenti come farebbe un collegamento BLE.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use super::{Scollegato, Transport};

/// Cosa fare quando arriva un comando.
#[derive(Clone, Debug)]
pub enum Battuta {
    /// Risposta completa (il prompt `>` va incluso nel testo).
    Testo(String),
    /// Nessuna risposta: provoca un timeout.
    Silenzio,
    /// Il collegamento cade.
    Caduta,
}

pub struct Copione {
    risposte: HashMap<String, VecDeque<Battuta>>,
    predefinita: Battuta,
    uscita: VecDeque<Vec<u8>>,
    ingresso: Vec<u8>,
    frammento: usize,
    caduto: bool,
    /// Comandi ricevuti, nell'ordine.
    pub ricevuti: Vec<String>,
}

impl Copione {
    /// `frammento` è la dimensione massima di ogni pezzo restituito da `read`.
    pub fn new(frammento: usize) -> Self {
        Copione {
            risposte: HashMap::new(),
            predefinita: Battuta::Testo("NO DATA\r\r>".into()),
            uscita: VecDeque::new(),
            ingresso: Vec::new(),
            frammento: frammento.max(1),
            caduto: false,
            ricevuti: Vec::new(),
        }
    }

    /// Copione con le risposte standard ai comandi di inizializzazione.
    pub fn con_inizializzazione(frammento: usize) -> Self {
        let mut c = Self::new(frammento);
        c.testo("ATZ", "ATZ\r\r\rELM327 v1.5\r\r>");
        // Dopo ATZ l'eco è ancora attiva per il primo comando.
        c.testo("ATE0", "ATE0\rOK\r\r>");
        for cmd in ["ATL0", "ATS0", "ATH1", "ATAT1", "ATSP0"] {
            c.testo(cmd, "OK\r\r>");
        }
        c.testo("ATDPN", "A6\r\r>");
        c.testo("ATRV", "12.4V\r\r>");
        c
    }

    /// Come [`Copione::con_inizializzazione`], per un veicolo a linea K:
    /// `dpn` è la risposta ad ATDPN (`A3` ISO 9141-2, `A4`/`A5` KWP2000).
    pub fn con_linea_k(frammento: usize, dpn: &str) -> Self {
        let mut c = Self::con_inizializzazione(frammento);
        c.testo("ATDPN", &format!("{dpn}\r\r>"));
        c
    }

    /// Risposta fissa a un comando (ripetuta a ogni invio).
    pub fn testo(&mut self, cmd: &str, risposta: &str) -> &mut Self {
        self.risposte.insert(
            cmd.to_string(),
            VecDeque::from([Battuta::Testo(risposta.to_string())]),
        );
        self
    }

    /// Sequenza di battute: l'ultima resta valida per gli invii successivi.
    pub fn sequenza(&mut self, cmd: &str, battute: Vec<Battuta>) -> &mut Self {
        self.risposte.insert(cmd.to_string(), battute.into());
        self
    }
}

/// Riga di risposta ISO 9141-2 come la stampa l'ELM327 con le intestazioni:
/// `48 6B <ecu>`, dati e checksum (somma a 8 bit).
pub fn riga_iso9141(ecu: u8, dati: &[u8]) -> String {
    riga_con_checksum(&[0x48, 0x6B, ecu], dati)
}

/// Riga di risposta KWP2000 (ISO 14230-4): `8x F1 <ecu>`, dove `x` è il numero
/// di byte di dati, poi dati e checksum.
pub fn riga_kwp(ecu: u8, dati: &[u8]) -> String {
    riga_con_checksum(&[0x80 | (dati.len() as u8 & 0x3F), 0xF1, ecu], dati)
}

fn riga_con_checksum(intestazione: &[u8], dati: &[u8]) -> String {
    let mut byte: Vec<u8> = intestazione.iter().chain(dati).copied().collect();
    let somma = byte.iter().fold(0u8, |s, b| s.wrapping_add(*b));
    byte.push(somma);
    byte.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl Transport for Copione {
    fn write(&mut self, data: &[u8]) -> Result<(), Scollegato> {
        if self.caduto {
            return Err(Scollegato("collegamento caduto (copione)".into()));
        }
        self.ingresso.extend_from_slice(data);
        while let Some(pos) = self.ingresso.iter().position(|b| *b == b'\r') {
            let riga: Vec<u8> = self.ingresso.drain(..=pos).collect();
            let cmd = String::from_utf8_lossy(&riga[..riga.len() - 1])
                .trim()
                .to_string();
            self.ricevuti.push(cmd.clone());
            let battuta = match self.risposte.get_mut(&cmd) {
                Some(coda) if coda.len() > 1 => coda.pop_front().unwrap(),
                Some(coda) => coda.front().cloned().unwrap_or(Battuta::Silenzio),
                None => self.predefinita.clone(),
            };
            match battuta {
                Battuta::Testo(t) => {
                    for pezzo in t.as_bytes().chunks(self.frammento) {
                        self.uscita.push_back(pezzo.to_vec());
                    }
                }
                Battuta::Silenzio => {}
                Battuta::Caduta => {
                    self.caduto = true;
                    return Err(Scollegato("collegamento caduto (copione)".into()));
                }
            }
        }
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Scollegato> {
        if self.caduto {
            return Err(Scollegato("collegamento caduto (copione)".into()));
        }
        match self.uscita.pop_front() {
            Some(p) => Ok(p),
            None => {
                // Niente in arrivo: consuma una piccola parte dell'attesa.
                std::thread::sleep(timeout.min(Duration::from_millis(2)));
                Ok(Vec::new())
            }
        }
    }

    fn descrizione(&self) -> String {
        "Copione di verifica".into()
    }
}
