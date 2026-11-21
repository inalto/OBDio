//! Simulatore di adattatore ELM327 collegato a una centralina benzina con P0420.
//!
//! Produce testo ELM327 grezzo (eco, «SEARCHING...», trame CAN con riempimento,
//! prompt) spezzato in frammenti da 20 byte: i dati attraversano quindi lo
//! stesso identico percorso di decodifica di un adattatore reale.
//! Tutti i segnali, sonde lambda comprese, sono SINTETICI.
//!
//! Il servizio 04 (cancellazione) è simulato come da norma: spariscono i
//! codici confermati e in sospeso, il fermo immagine e la prontezza; il P0420
//! resta fra i codici permanenti (0A).

use std::collections::VecDeque;
use std::f64::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{Scollegato, Transport};
use crate::obd::mask_from_supported;

const ECU: &str = "7E8";
const PID_SERVIZIO_01: [u8; 23] = [
    0x01, 0x03, 0x04, 0x05, 0x06, 0x07, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x13, 0x15, 0x1C, 0x1F,
    0x20, 0x21, 0x24, 0x33, 0x40, 0x42, 0x44,
];
const PID_SERVIZIO_02: [u8; 9] = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x0B, 0x0C, 0x0D];
const VIN_FINTO: &[u8; 17] = b"DEMO0000000000000";

pub struct DemoTransport {
    ingresso: Vec<u8>,
    uscita: VecDeque<Vec<u8>>,
    pronto_a: Instant,
    eco: bool,
    intestazioni: bool,
    spazi: bool,
    a_capo: bool,
    protocollo_trovato: bool,
    t0: Instant,
    latenza: Duration,
    interrompi: Arc<AtomicBool>,
    chiuso: bool,
    /// Vero dopo un servizio 04: codici, fermo immagine e prontezza sono
    /// azzerati come su una centralina reale. Resta valido per tutta la vita
    /// del simulatore (un ATZ non riporta indietro i codici).
    codici_cancellati: bool,
}

impl Default for DemoTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl DemoTransport {
    pub fn new() -> Self {
        Self::con_latenza(Duration::from_millis(35))
    }

    /// `latenza` è il tempo simulato fra richiesta e risposta.
    pub fn con_latenza(latenza: Duration) -> Self {
        DemoTransport {
            ingresso: Vec::new(),
            uscita: VecDeque::new(),
            pronto_a: Instant::now(),
            eco: true,
            intestazioni: false,
            spazi: true,
            a_capo: false,
            protocollo_trovato: false,
            t0: Instant::now(),
            latenza,
            interrompi: Arc::new(AtomicBool::new(false)),
            chiuso: false,
            codici_cancellati: false,
        }
    }

    /// Interruttore per simulare la caduta del collegamento (verifiche).
    pub fn interruttore(&self) -> Arc<AtomicBool> {
        self.interrompi.clone()
    }

    fn caduto(&self) -> bool {
        self.chiuso || self.interrompi.load(Ordering::Relaxed)
    }

    fn rispondi(&mut self, grezzo: &str) -> String {
        let cmd: String = grezzo
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_uppercase();
        let mut corpo: Vec<String> = Vec::new();
        if let Some(at) = cmd.strip_prefix("AT") {
            corpo.push(self.comando_at(at));
        } else if !cmd.is_empty() && cmd.chars().all(|c| c.is_ascii_hexdigit()) {
            if !self.protocollo_trovato {
                corpo.push("SEARCHING...".into());
                self.protocollo_trovato = true;
            }
            match self.dati(&cmd) {
                Some(payload) => corpo.extend(self.trame(&payload)),
                None => corpo.push("NO DATA".into()),
            }
        } else {
            corpo.push("?".into());
        }
        let sep = if self.a_capo { "\r\n" } else { "\r" };
        let mut out = String::new();
        if self.eco {
            out.push_str(grezzo);
            out.push_str(sep);
        }
        out.push_str(&corpo.join(sep));
        out.push_str(sep);
        out.push_str(sep);
        out.push('>');
        out
    }

    fn comando_at(&mut self, at: &str) -> String {
        let ok = "OK".to_string();
        match at {
            "Z" => {
                self.eco = true;
                self.intestazioni = false;
                self.spazi = true;
                self.a_capo = false;
                self.protocollo_trovato = false;
                "\rELM327 v1.5".into()
            }
            "I" => "ELM327 v1.5".into(),
            "@1" => "SIMULATORE OBDIO".into(),
            "E0" | "E1" => {
                self.eco = at == "E1";
                ok
            }
            "L0" | "L1" => {
                self.a_capo = at == "L1";
                ok
            }
            "S0" | "S1" => {
                self.spazi = at == "S1";
                ok
            }
            "H0" | "H1" => {
                self.intestazioni = at == "H1";
                ok
            }
            "AT0" | "AT1" | "AT2" => ok,
            "SP0" => {
                self.protocollo_trovato = false;
                ok
            }
            "DPN" => "A6".into(),
            "DP" => "AUTO, ISO 15765-4 (CAN 11/500)".into(),
            "RV" => "13.9V".into(),
            _ => "?".into(),
        }
    }

    /// Formatta il carico utile come trame ISO-TP su CAN a 11 bit.
    fn trame(&self, payload: &[u8]) -> Vec<String> {
        let mut trame: Vec<Vec<u8>> = Vec::new();
        if !self.intestazioni {
            trame.push(payload.to_vec());
        } else if payload.len() <= 7 {
            let mut t = vec![payload.len() as u8];
            t.extend_from_slice(payload);
            t.resize(8, 0xAA);
            trame.push(t);
        } else {
            let n = payload.len();
            let mut t = vec![0x10 | ((n >> 8) as u8 & 0x0F), (n & 0xFF) as u8];
            t.extend_from_slice(&payload[..6]);
            trame.push(t);
            for (i, pezzo) in payload[6..].chunks(7).enumerate() {
                let mut t = vec![0x20 | (((i + 1) & 0x0F) as u8)];
                t.extend_from_slice(pezzo);
                t.resize(8, 0xAA);
                trame.push(t);
            }
        }
        let sep = if self.spazi { " " } else { "" };
        trame
            .iter()
            .map(|t| {
                let mut parti: Vec<String> = Vec::new();
                if self.intestazioni {
                    parti.push(ECU.to_string());
                }
                parti.extend(t.iter().map(|b| format!("{b:02X}")));
                parti.join(sep)
            })
            .collect()
    }

    fn dati(&mut self, cmd: &str) -> Option<Vec<u8>> {
        let byte = |i: usize| u8::from_str_radix(cmd.get(i..i + 2)?, 16).ok();
        let servizio = byte(0)?;
        let cancellati = self.codici_cancellati;
        match (servizio, cmd.len()) {
            // Cancellazione: risposta positiva e stato azzerato.
            (0x04, 2) => {
                self.codici_cancellati = true;
                Some(vec![0x44])
            }
            // Dopo la cancellazione: nessun fotogramma (DTC 0000), altri PID assenti.
            (0x02, 6) if cancellati => {
                let pid = byte(2)?;
                (pid == 0x02 && byte(4)? == 0).then(|| vec![0x42, 0x02, 0x00, 0x00, 0x00])
            }
            (0x03, 2) if cancellati => Some(vec![0x43, 0x00]),
            (0x07, 2) if cancellati => Some(vec![0x47, 0x00]),
            // I codici permanenti (0A) restano: il servizio 04 non li cancella.
            (0x01, 4) => {
                let pid = byte(2)?;
                let mut out = vec![0x41, pid];
                out.extend(self.servizio_01(pid)?);
                Some(out)
            }
            (0x02, 6) => {
                let pid = byte(2)?;
                if byte(4)? != 0 {
                    return None;
                }
                let mut out = vec![0x42, pid, 0x00];
                out.extend(servizio_02(pid)?);
                Some(out)
            }
            (0x03, 2) => Some(vec![0x43, 0x01, 0x04, 0x20]),
            (0x07, 2) => Some(vec![0x47, 0x01, 0x04, 0x20]),
            (0x0A, 2) => Some(vec![0x4A, 0x01, 0x04, 0x20]),
            (0x06, 4) => match byte(2)? {
                0x00 => Some([&[0x46, 0x00][..], &mask_from_supported(0x00, &[0x20])].concat()),
                0x20 => Some([&[0x46, 0x20][..], &mask_from_supported(0x20, &[0x21])].concat()),
                // Monitor catalizzatore B1: valore sintetico oltre il massimo.
                0x21 => Some(vec![
                    0x46, 0x21, 0x80, 0x20, 0x00, 0x90, 0x00, 0x00, 0x00, 0x80,
                ]),
                _ => None,
            },
            (0x09, 4) if byte(2)? == 0x02 => {
                let mut out = vec![0x49, 0x02, 0x01];
                out.extend_from_slice(VIN_FINTO);
                Some(out)
            }
            _ => None,
        }
    }

    fn servizio_01(&self, pid: u8) -> Option<Vec<u8>> {
        if pid == 0x00 || pid == 0x20 || pid == 0x40 {
            return Some(mask_from_supported(pid, &PID_SERVIZIO_01).to_vec());
        }
        if !PID_SERVIZIO_01.contains(&pid) {
            return None;
        }
        let t = self.t0.elapsed().as_secs_f64();
        let s = Scenario::al_tempo(t);
        let parola = |v: f64| {
            let w = v.round().clamp(0.0, 65535.0) as u16;
            vec![(w >> 8) as u8, (w & 0xFF) as u8]
        };
        let ottetto = |v: f64| vec![v.round().clamp(0.0, 255.0) as u8];
        Some(match pid {
            // Dopo la cancellazione: MIL spenta, 0 DTC; catalizzatore, evaporativo,
            // sonde e riscaldatori non completati.
            0x01 if self.codici_cancellati => vec![0x00, 0x07, 0x65, 0x65],
            // MIL accesa, 1 DTC; monitor catalizzatore, sonde e riscaldatori completati.
            0x01 => vec![0x81, 0x07, 0x65, 0x00],
            0x03 => vec![0x02, 0x00],
            0x04 => ottetto(s.carico * 255.0 / 100.0),
            0x05 => ottetto(83.0 + 5.0 * (1.0 - (-t / 120.0).exp()) + 40.0),
            0x06 => {
                let stft = 2.5 * (TAU * 0.35 * t).sin() + 0.8 * (TAU * 1.3 * t).sin();
                ottetto(128.0 + stft * 128.0 / 100.0)
            }
            0x07 => vec![0x8B],
            0x0B => ottetto(s.map),
            0x0C => parola((s.giri + 15.0 * (TAU * 0.7 * t).sin()) * 4.0),
            0x0D => ottetto(s.velocita),
            0x0E => ottetto((8.0 + 0.004 * (s.giri - 850.0) + 64.0) * 2.0),
            0x0F => ottetto(28.0 + 40.0),
            0x11 => ottetto((12.0 + s.carico * 0.25) * 255.0 / 100.0),
            0x13 => vec![0x03],
            0x15 => {
                // Interruzione simulata di 2,5 s ogni minuto, per mostrare i buchi nel grafico.
                let fase = t % 60.0;
                if (48.0..50.5).contains(&fase) {
                    return None;
                }
                let v = 0.45 + 0.30 * (TAU * 1.1 * (t - 0.35)).sin();
                vec![(v.clamp(0.05, 0.9) * 200.0).round() as u8, 0xFF]
            }
            0x1C => vec![0x06],
            0x1F => parola(t),
            0x21 => parola(137.0),
            0x24 => {
                let lambda = 1.0 + 0.025 * (TAU * 1.1 * t).sin();
                let tensione = 1.5 + 0.06 * (TAU * 1.1 * t).sin();
                let mut out = parola(lambda * 65536.0 / 2.0);
                out.extend(parola(tensione * 65536.0 / 8.0));
                out
            }
            0x33 => vec![100],
            0x42 => parola(13_900.0),
            0x44 => parola(32768.0),
            _ => return None,
        })
    }
}

/// Fermo immagine del caso di riferimento: P0420 a 69 km/h, 2.128 giri/min,
/// 83 °C, STFT +0,78 %, LTFT +8,59 %.
fn servizio_02(pid: u8) -> Option<Vec<u8>> {
    Some(match pid {
        0x00 => mask_from_supported(0x00, &PID_SERVIZIO_02).to_vec(),
        0x02 => vec![0x04, 0x20],
        0x03 => vec![0x02, 0x00],
        0x04 => vec![0x5A],
        0x05 => vec![0x7B],
        0x06 => vec![0x81],
        0x07 => vec![0x8B],
        0x0B => vec![0x2D],
        0x0C => vec![0x21, 0x40],
        0x0D => vec![0x45],
        _ => return None,
    })
}

/// Condizioni di guida simulate: minimo, 2.500 giri da fermo, velocità costante.
struct Scenario {
    giri: f64,
    velocita: f64,
    carico: f64,
    map: f64,
}

impl Scenario {
    const FASI: [(f64, [f64; 4]); 3] = [
        (25.0, [850.0, 0.0, 22.0, 32.0]),
        (20.0, [2500.0, 0.0, 28.0, 36.0]),
        (30.0, [2128.0, 69.0, 38.0, 48.0]),
    ];

    fn al_tempo(t: f64) -> Scenario {
        let ciclo: f64 = Self::FASI.iter().map(|f| f.0).sum();
        let mut resto = t.rem_euclid(ciclo);
        let mut indice = 0;
        for (i, fase) in Self::FASI.iter().enumerate() {
            if resto < fase.0 {
                indice = i;
                break;
            }
            resto -= fase.0;
        }
        let attuale = Self::FASI[indice].1;
        let precedente = Self::FASI[(indice + Self::FASI.len() - 1) % Self::FASI.len()].1;
        // Transizione di 3 secondi dalla fase precedente.
        let k = (resto / 3.0).clamp(0.0, 1.0);
        let k = k * k * (3.0 - 2.0 * k);
        let mix = |i: usize| precedente[i] + (attuale[i] - precedente[i]) * k;
        Scenario {
            giri: mix(0),
            velocita: mix(1),
            carico: mix(2),
            map: mix(3),
        }
    }
}

impl Transport for DemoTransport {
    fn write(&mut self, data: &[u8]) -> Result<(), Scollegato> {
        if self.caduto() {
            return Err(Scollegato("collegamento demo interrotto".into()));
        }
        self.ingresso.extend_from_slice(data);
        while let Some(pos) = self.ingresso.iter().position(|b| *b == b'\r') {
            let riga: Vec<u8> = self.ingresso.drain(..=pos).collect();
            let cmd = String::from_utf8_lossy(&riga[..riga.len() - 1]).to_string();
            let risposta = self.rispondi(cmd.trim());
            for pezzo in risposta.as_bytes().chunks(20) {
                self.uscita.push_back(pezzo.to_vec());
            }
            self.pronto_a = Instant::now() + self.latenza;
        }
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, Scollegato> {
        if self.caduto() {
            return Err(Scollegato("collegamento demo interrotto".into()));
        }
        let ora = Instant::now();
        if self.uscita.is_empty() {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            return Ok(Vec::new());
        }
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
        "Simulatore demo (dati sintetici)".into()
    }

    fn is_demo(&self) -> bool {
        true
    }

    fn close(&mut self) {
        self.chiuso = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invia(d: &mut DemoTransport, cmd: &str) -> String {
        d.write(format!("{cmd}\r").as_bytes()).unwrap();
        let mut out = String::new();
        while !out.contains('>') {
            out.push_str(&String::from_utf8_lossy(
                &d.read(Duration::from_millis(50)).unwrap(),
            ));
        }
        out
    }

    #[test]
    fn cancellazione_simulata_resiste_al_riavvio() {
        let mut d = DemoTransport::con_latenza(Duration::ZERO);
        for c in ["ATZ", "ATE0", "ATS1", "ATH1"] {
            invia(&mut d, c);
        }
        assert!(invia(&mut d, "03").contains("7E8 04 43 01 04 20"));
        assert!(invia(&mut d, "04").contains("7E8 01 44 AA"));
        invia(&mut d, "ATZ");
        for c in ["ATE0", "ATS1", "ATH1"] {
            invia(&mut d, c);
        }
        assert!(invia(&mut d, "03").contains("7E8 02 43 00"));
        assert!(invia(&mut d, "07").contains("7E8 02 47 00"));
        assert!(invia(&mut d, "0101").contains("7E8 06 41 01 00 07 65 65"));
        assert!(invia(&mut d, "020200").contains("7E8 05 42 02 00 00 00"));
        // I codici permanenti non si cancellano con il servizio 04.
        assert!(invia(&mut d, "0A").contains("7E8 04 4A 01 04 20"));
        assert!(invia(&mut d, "0621").contains("46 21"));
    }
}
