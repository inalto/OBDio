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
