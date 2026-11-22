//! Riproduzione in tempo reale di una sessione registrata («rivivere» una
//! sessione, docs/DESIGN.md sezione 6).
//!
//! Il [`Riproduttore`] è logica pura: non ha thread né attese. Il thread di
//! lavoro gli passa il tempo reale trascorso e riceve i campioni e i marcatori
//! il cui momento è arrivato; poi li invia per la stessa strada dei dati dal
//! vivo (interfaccia e MQTT).
//!
//! # Tempi
//!
//! - `t` di ogni campione restituito è la **posizione nella sessione** (secondi
//!   dall'inizio della registrazione), cioè l'orologio della riproduzione:
//!   asse dei grafici, barra di trasporto e marcatori coincidono. Il valore è
//!   quello registrato, quindi non cambia con la velocità.
//! - `ts` resta quello originale: dice quando il dato è stato registrato.
//! - Dopo un salto (posizione cambiata a mano o ripetizione dall'inizio) i
//!   tempi possono tornare indietro: per questo ogni salto è segnalato a parte
//!   e chi riceve svuota i propri grafici prima dei campioni successivi.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Marker, Sample, Stato};
use crate::obd::pids;
use crate::recording::SessionData;

/// Velocità proposte dall'interfaccia.
pub const VELOCITA: [f32; 5] = [0.5, 1.0, 2.0, 5.0, 10.0];
const VELOCITA_MIN: f32 = 0.5;
const VELOCITA_MAX: f32 = 10.0;

/// Comandi della barra di trasporto.
#[derive(Clone, Debug, PartialEq)]
pub enum ComandoRiproduzione {
    /// Riprende; a sessione finita ricomincia dall'inizio.
    Riprendi,
    Pausa,
    /// Fattore di velocità (limitato fra 0,5 e 10).
    Velocita(f32),
    /// Salta a una posizione assoluta in secondi.
    VaiA(f64),
    /// Ripetizione dall'inizio alla fine della sessione.
    Ripeti(bool),
}

/// Stato della riproduzione mostrato dalla barra di trasporto.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatoRiproduzione {
    /// Secondi dall'inizio della sessione.
    pub posizione: f64,
    pub durata: f64,
    pub in_pausa: bool,
    pub velocita: f32,
    pub ripeti: bool,
    /// Arrivata alla fine senza ripetizione: ferma con la posizione alla fine.
    pub finita: bool,
    /// Marcatori della sessione: posizione ed etichetta.
    pub marcatori: Vec<(f64, String)>,
}

impl StatoRiproduzione {
    /// Uguale a `altro` salvo la posizione (che cambia di continuo).
    pub fn stesso_salvo_posizione(&self, altro: &StatoRiproduzione) -> bool {
        self.durata == altro.durata
            && self.in_pausa == altro.in_pausa
            && self.velocita == altro.velocita
            && self.ripeti == altro.ripeti
            && self.finita == altro.finita
            && self.marcatori == altro.marcatori
    }
}

/// Ciò che un passo della riproduzione consegna.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Uscita {
    pub campioni: Vec<Sample>,
    pub marcatori: Vec<Marker>,
    /// La riproduzione è ripartita dall'inizio (ripetizione) **dopo** i
    /// campioni e i marcatori qui sopra: la posizione è saltata a questo valore.
    pub salto: Option<f64>,
    /// La sessione è finita in questo passo.
    pub finita_ora: bool,
}

impl Uscita {
    pub fn vuota(&self) -> bool {
        self.campioni.is_empty()
            && self.marcatori.is_empty()
            && self.salto.is_none()
            && !self.finita_ora
    }
}

/// Motore della riproduzione.
#[derive(Clone, Debug)]
pub struct Riproduttore {
    campioni: Vec<Sample>,
    marcatori: Vec<Marker>,
    durata: f64,
    posizione: f64,
    /// Indice del prossimo campione da consegnare.
    i_campione: usize,
    i_marcatore: usize,
    in_pausa: bool,
    velocita: f32,
    ripeti: bool,
    finita: bool,
}

impl Riproduttore {
    /// Prepara la riproduzione: campioni e marcatori sono ordinati per tempo
    /// (a parità di tempo resta l'ordine d'arrivo). Quelli con un tempo non
    /// finito sono esclusi. Parte subito, a velocità 1×, senza ripetizione.
    pub fn new(mut campioni: Vec<Sample>, mut marcatori: Vec<Marker>) -> Self {
        campioni.retain(|c| c.t.is_finite());
        marcatori.retain(|m| m.t.is_finite());
        campioni.sort_by(|a, b| a.t.total_cmp(&b.t));
        marcatori.sort_by(|a, b| a.t.total_cmp(&b.t));
        let durata = campioni
            .last()
            .map(|c| c.t)
            .into_iter()
            .chain(marcatori.last().map(|m| m.t))
            .fold(0.0_f64, f64::max);
        Riproduttore {
            campioni,
            marcatori,
            durata,
            posizione: 0.0,
            i_campione: 0,
            i_marcatore: 0,
            in_pausa: false,
            velocita: 1.0,
            ripeti: false,
            finita: false,
        }
    }

    pub fn da_sessione(s: &SessionData) -> Self {
        Self::new(s.campioni.clone(), s.marcatori.clone())
    }

    pub fn durata(&self) -> f64 {
        self.durata
    }

    pub fn posizione(&self) -> f64 {
        self.posizione
    }

    /// Vero se il tempo scorre (né in pausa né finita).
    pub fn in_corso(&self) -> bool {
        !self.in_pausa && !self.finita
    }

    pub fn stato(&self) -> StatoRiproduzione {
        StatoRiproduzione {
            posizione: self.posizione,
            durata: self.durata,
            in_pausa: self.in_pausa,
            velocita: self.velocita,
            ripeti: self.ripeti,
            finita: self.finita,
            marcatori: self
                .marcatori
                .iter()
                .map(|m| (m.t, m.etichetta.clone()))
                .collect(),
        }
    }

    /// Applica un comando. Restituisce la nuova posizione se è saltata.
    pub fn comando(&mut self, c: ComandoRiproduzione) -> Option<f64> {
        match c {
            ComandoRiproduzione::Riprendi => {
                let salto = if self.finita { self.vai_a(0.0) } else { None };
                self.in_pausa = false;
                salto
            }
            ComandoRiproduzione::Pausa => {
                self.in_pausa = true;
                None
            }
            ComandoRiproduzione::Velocita(v) => {
                if v.is_finite() && v > 0.0 {
                    self.velocita = v.clamp(VELOCITA_MIN, VELOCITA_MAX);
                }
                None
            }
            ComandoRiproduzione::VaiA(t) => self.vai_a(t),
            ComandoRiproduzione::Ripeti(r) => {
                self.ripeti = r;
                None
            }
        }
    }

    /// Salta a `t` (limitato alla sessione). Nulla di quanto precede `t` sarà
    /// consegnato; i campioni esattamente in `t` arrivano al passo successivo.
    fn vai_a(&mut self, t: f64) -> Option<f64> {
        if !t.is_finite() {
            return None;
        }
        let t = t.clamp(0.0, self.durata);
        self.posizione = t;
        self.i_campione = self.campioni.partition_point(|c| c.t < t);
        self.i_marcatore = self.marcatori.partition_point(|m| m.t < t);
        self.finita = false;
        Some(t)
    }

    /// Fa avanzare la riproduzione di `secondi_reali` (tempo del PC, moltiplicato
    /// per la velocità) e consegna, in ordine, campioni e marcatori con tempo
    /// fino alla nuova posizione. Ogni campione è consegnato una sola volta.
    ///
    /// Alla fine della sessione: con la ripetizione si riparte da zero (salto);
    /// senza, la riproduzione si ferma con la posizione alla fine. Il tempo
    /// eccedente la fine nel passo in cui si riparte non viene recuperato.
    pub fn avanza(&mut self, secondi_reali: f64) -> Uscita {
        let mut u = Uscita::default();
        if !self.in_corso() {
            return u;
        }
        let dt = if secondi_reali.is_finite() && secondi_reali > 0.0 {
            secondi_reali
        } else {
            0.0
        };
        let nuova = self.posizione + dt * self.velocita as f64;
        let alla_fine = nuova >= self.durata;
        let limite = if alla_fine { f64::INFINITY } else { nuova };
        while let Some(c) = self.campioni.get(self.i_campione)
            && c.t <= limite
        {
            u.campioni.push(c.clone());
            self.i_campione += 1;
        }
        while let Some(m) = self.marcatori.get(self.i_marcatore)
            && m.t <= limite
        {
            u.marcatori.push(m.clone());
            self.i_marcatore += 1;
        }
        if !alla_fine {
            self.posizione = nuova;
        } else if self.ripeti && self.durata > 0.0 {
            self.posizione = 0.0;
            self.i_campione = 0;
            self.i_marcatore = 0;
            u.salto = Some(0.0);
        } else {
            self.posizione = self.durata;
            self.finita = true;
            u.finita_ora = true;
        }
        u
    }
}

/// PID e parametri liberi presenti in una sessione, per decidere che cosa la
/// riproduzione «supporta».
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presenti {
    /// PID del servizio 01 per centralina (solo campioni con la centralina).
    pub supportati: BTreeMap<String, BTreeSet<u8>>,
    /// Unione dei PID, compresi quelli delle letture senza risposta.
    pub unione: BTreeSet<u8>,
    /// Chiavi dei parametri liberi presenti.
    pub liberi: BTreeSet<String>,
    /// Centraline viste nei campioni, in ordine di comparsa.
    pub ecu: Vec<String>,
}

/// Un parametro è presente se almeno un campione non dice «non supportato».
/// I parametri calcolati (distanza parziale) fanno risultare la loro sorgente.
pub fn presenti(s: &SessionData) -> Presenti {
    let mut p = Presenti::default();
    for c in &s.campioni {
        if !c.ecu.is_empty() && !p.ecu.contains(&c.ecu) {
            p.ecu.push(c.ecu.clone());
        }
        if c.stato == Stato::NonSupportato {
            continue;
        }
        let Some(par) = pids::per_key(&c.key) else {
            continue;
        };
        if par.libero {
            p.liberi.insert(c.key.clone());
            continue;
        }
        let pid = if par.derivato {
            match pids::sorgente_derivato(&c.key).and_then(pids::per_key) {
                Some(sorgente) => sorgente.pid,
                None => continue,
            }
        } else {
            par.pid
        };
        p.unione.insert(pid);
        if !c.ecu.is_empty() {
            p.supportati.entry(c.ecu.clone()).or_default().insert(pid);
        }
    }
    p
}

/// Data e ora di una sessione per la fascia «RIPRODUZIONE — dati registrati
/// il …»: `21/11/2026 alle 19:37`. Se la data non è leggibile, il testo così
/// com'è.
pub fn data_leggibile(iso: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(iso.trim()) {
        Ok(d) => d.format("%d/%m/%Y alle %H:%M").to_string(),
        Err(_) if iso.trim().is_empty() => "data sconosciuta".into(),
        Err(_) => iso.trim().to_string(),
    }
}

/// `mm:ss` (oppure `h:mm:ss` oltre l'ora) per la barra di trasporto.
pub fn mm_ss(secondi: f64) -> String {
    let s = if secondi.is_finite() {
        secondi.max(0.0).floor() as u64
    } else {
        0
    };
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(t: f64, key: &str, v: Option<f64>) -> Sample {
        Sample {
            t,
            ts: format!("2026-11-21T10:00:{:06.3}+01:00", t % 60.0),
            ecu: if v.is_some() {
                "7E8".into()
            } else {
                String::new()
            },
            key: key.into(),
            pid: pids::per_key(key).map_or(0, |p| p.pid),
            valore: v,
            unita: String::new(),
            stato: if v.is_some() {
                Stato::Ok
            } else {
                Stato::RispostaMancante
            },
        }
    }

    fn m(t: f64, e: &str) -> Marker {
        Marker {
            t,
            ts: String::new(),
            etichetta: e.into(),
        }
    }

    /// Dieci secondi, un campione ogni 0,1 s, uno su sette mancante.
    fn sessione() -> (Vec<Sample>, Vec<Marker>) {
        let campioni = (0..=100)
            .map(|i| {
                let t = i as f64 * 0.1;
                c(t, "giri", (i % 7 != 3).then_some(800.0 + i as f64))
            })
            .collect();
        (campioni, vec![m(2.5, "Minimo"), m(7.05, "2.500 giri")])
    }

    fn tempi(v: &[Sample]) -> Vec<f64> {
        v.iter().map(|c| c.t).collect()
    }

    #[test]
    fn velocita_uno_e_dieci() {
        let (cc, mm) = sessione();
        let mut r = Riproduttore::new(cc.clone(), mm.clone());
        assert_eq!(r.durata(), 10.0);
        // Il primo passo consegna i campioni a t = 0.
        let u = r.avanza(0.0);
        assert_eq!(tempi(&u.campioni), vec![0.0]);
        let u = r.avanza(1.0);
        assert_eq!(u.campioni.len(), 10, "da 0,1 a 1,0 s");
        assert!((r.posizione() - 1.0).abs() < 1e-12);

        r.comando(ComandoRiproduzione::Velocita(10.0));
        let u = r.avanza(0.5);
        assert!((r.posizione() - 6.0).abs() < 1e-9);
        assert_eq!(u.marcatori, vec![m(2.5, "Minimo")]);
        assert_eq!(u.campioni.first().map(|c| c.key.as_str()), Some("giri"));
        // Velocità fuori dall'intervallo: limitata.
        r.comando(ComandoRiproduzione::Velocita(50.0));
        assert_eq!(r.stato().velocita, 10.0);
        r.comando(ComandoRiproduzione::Velocita(0.1));
        assert_eq!(r.stato().velocita, 0.5);
        r.comando(ComandoRiproduzione::Velocita(f32::NAN));
        assert_eq!(r.stato().velocita, 0.5);
        let prima = r.posizione();
        r.avanza(2.0);
        assert!((r.posizione() - prima - 1.0).abs() < 1e-9, "0,5× per 2 s");
    }

    #[test]
    fn pausa_ferma_il_flusso() {
        let (cc, mm) = sessione();
        let mut r = Riproduttore::new(cc, mm);
        r.avanza(1.0);
        r.comando(ComandoRiproduzione::Pausa);
        assert!(r.stato().in_pausa);
        assert!(r.avanza(5.0).vuota());
        assert!((r.posizione() - 1.0).abs() < 1e-12);
        r.comando(ComandoRiproduzione::Riprendi);
        assert_eq!(r.avanza(0.25).campioni.len(), 2);
    }

    #[test]
    fn salto_avanti_e_indietro() {
        let (cc, mm) = sessione();
        let mut r = Riproduttore::new(cc, mm);
        r.avanza(3.0);
        // Avanti: nulla di quanto sta fra 3 e 6 s viene consegnato.
        assert_eq!(r.comando(ComandoRiproduzione::VaiA(6.0)), Some(6.0));
        let u = r.avanza(0.15);
        assert!(
            (u.campioni[0].t - 6.0).abs() < 1e-9,
            "{:?}",
            tempi(&u.campioni)
        );
        assert!(u.campioni.iter().all(|c| c.t >= 6.0 - 1e-9));
        // Indietro: si riparte da 1 s, niente da prima.
        assert_eq!(r.comando(ComandoRiproduzione::VaiA(1.0)), Some(1.0));
        let u = r.avanza(0.5);
        assert!(u.campioni.iter().all(|c| c.t >= 1.0 - 1e-9));
        assert!((u.campioni[0].t - 1.0).abs() < 1e-9);
        // Il marcatore a 2,5 s arriva di nuovo dopo il salto indietro, una volta.
        let u = r.avanza(2.0);
        assert_eq!(u.marcatori.len(), 1);
        // Limiti: oltre la fine e prima dell'inizio.
        assert_eq!(r.comando(ComandoRiproduzione::VaiA(99.0)), Some(10.0));
        assert_eq!(r.comando(ComandoRiproduzione::VaiA(-3.0)), Some(0.0));
        assert_eq!(r.comando(ComandoRiproduzione::VaiA(f64::NAN)), None);
    }

    #[test]
    fn fine_e_ripetizione() {
        let (cc, mm) = sessione();
        let n = cc.len();
        let mut r = Riproduttore::new(cc, mm);
        let u = r.avanza(100.0);
        assert_eq!(u.campioni.len(), n);
        assert_eq!(u.marcatori.len(), 2);
        assert!(u.finita_ora && u.salto.is_none());
        let s = r.stato();
        assert!(s.finita && s.posizione == s.durata);
        assert!(r.avanza(1.0).vuota(), "ferma alla fine");
        // Riprendere una sessione finita ricomincia dall'inizio.
        assert_eq!(r.comando(ComandoRiproduzione::Riprendi), Some(0.0));
        assert!(!r.stato().finita);
        assert_eq!(r.avanza(0.0).campioni.len(), 1);

        // Con la ripetizione: tutto, poi un salto a zero, poi di nuovo.
        r.comando(ComandoRiproduzione::Ripeti(true));
        let u = r.avanza(20.0);
        assert_eq!(u.campioni.len(), n - 1);
        assert_eq!(u.salto, Some(0.0));
        assert!(!r.stato().finita);
        let u = r.avanza(0.05);
        assert_eq!(tempi(&u.campioni), vec![0.0]);
        assert_eq!(r.stato().marcatori.len(), 2);
    }

    #[test]
    fn sessione_vuota_finisce_subito() {
        let mut r = Riproduttore::new(Vec::new(), Vec::new());
        assert_eq!(r.durata(), 0.0);
        let u = r.avanza(0.1);
        assert!(u.finita_ora && u.campioni.is_empty());
        r.comando(ComandoRiproduzione::Ripeti(true));
        r.comando(ComandoRiproduzione::Riprendi);
        // Con durata nulla la ripetizione non gira a vuoto.
        assert!(r.avanza(0.1).finita_ora);
    }

    #[test]
    fn ordina_e_scarta_tempi_non_finiti() {
        let r = Riproduttore::new(
            vec![
                c(2.0, "giri", Some(1.0)),
                c(f64::NAN, "giri", Some(2.0)),
                c(1.0, "giri", Some(3.0)),
            ],
            vec![m(f64::INFINITY, "x")],
        );
        assert_eq!(r.durata(), 2.0);
        assert!(r.stato().marcatori.is_empty());
        let mut r = r;
        assert_eq!(tempi(&r.avanza(5.0).campioni), vec![1.0, 2.0]);
    }

    /// Generatore pseudo-casuale a seme fisso (xorshift), senza dipendenze.
    struct Caso(u64);
    impl Caso {
        fn prossimo(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn reale(&mut self, max: f64) -> f64 {
            (self.prossimo() % 1_000_000) as f64 / 1_000_000.0 * max
        }
    }

    #[test]
    fn nessun_campione_perso_o_doppio_con_passi_qualsiasi() {
        // Tempi irregolari, anche ripetuti (più chiavi nello stesso istante).
        let mut caso = Caso(0x9E37_79B9_7F4A_7C15);
        let mut t = 0.0;
        let mut originali = Vec::new();
        for i in 0..3000 {
            if !caso.prossimo().is_multiple_of(3) {
                t += caso.reale(0.2);
            }
            let k = ["giri", "velocita", "o2s1_lambda"][i % 3];
            originali.push(c(t, k, (i % 11 != 0).then_some(i as f64)));
        }
        let marcatori: Vec<Marker> = (0..20).map(|i| m(i as f64 * 15.0, "m")).collect();
        for prova in 0..20 {
            let mut r = Riproduttore::new(originali.clone(), marcatori.clone());
            let mut visti: Vec<Sample> = Vec::new();
            let mut marc = 0;
            let mut passi = 0;
            while !r.stato().finita {
                // Passi da 0 a 1,5 s reali, velocità che cambia, pause brevi.
                match caso.prossimo() % 10 {
                    0 => {
                        r.comando(ComandoRiproduzione::Pausa);
                    }
                    1 => {
                        r.comando(ComandoRiproduzione::Riprendi);
                    }
                    2 => {
                        let v = VELOCITA[(caso.prossimo() % 5) as usize];
                        r.comando(ComandoRiproduzione::Velocita(v));
                    }
                    _ => {}
                }
                if r.stato().in_pausa && caso.prossimo().is_multiple_of(2) {
                    r.comando(ComandoRiproduzione::Riprendi);
                }
                let u = r.avanza(caso.reale(1.5));
                marc += u.marcatori.len();
                visti.extend(u.campioni);
                passi += 1;
                assert!(
                    passi < 1_000_000,
                    "la riproduzione non finisce (prova {prova})"
                );
            }
            assert_eq!(
                visti, originali,
                "prova {prova}: persi, doppi o fuori ordine"
            );
            assert_eq!(marc, marcatori.len(), "prova {prova}: marcatori");
            // I campioni mancanti restano mancanti: mai uno zero.
            assert!(
                visti
                    .iter()
                    .all(|c| (c.stato == Stato::Ok) == c.valore.is_some())
            );
        }
    }

    #[test]
    fn presenti_nella_sessione() {
        let s = SessionData {
            campioni: vec![
                c(0.0, "giri", Some(900.0)),
                c(0.1, "o2s2_tensione", None),
                c(0.2, pids::CHIAVE_DISTANZA_PARZIALE, Some(0.0)),
                Sample {
                    stato: Stato::NonSupportato,
                    ..c(0.3, "temp_olio", None)
                },
            ],
            ..Default::default()
        };
        let p = presenti(&s);
        assert_eq!(p.ecu, vec!["7E8".to_string()]);
        // Giri e velocità (sorgente della distanza) con la centralina; la sonda
        // senza risposta conta nell'unione; l'olio «non supportato» no.
        assert_eq!(p.supportati["7E8"], BTreeSet::from([0x0C, 0x0D]));
        assert_eq!(p.unione, BTreeSet::from([0x0C, 0x0D, 0x15]));
        assert!(p.liberi.is_empty());
    }

    #[test]
    fn testi_della_barra() {
        assert_eq!(mm_ss(0.0), "00:00");
        assert_eq!(mm_ss(83.9), "01:23");
        assert_eq!(mm_ss(3725.0), "1:02:05");
        assert_eq!(mm_ss(f64::NAN), "00:00");
        assert_eq!(
            data_leggibile("2026-11-21T19:37:41.512+01:00"),
            "21/11/2026 alle 19:37"
        );
        assert_eq!(data_leggibile(""), "data sconosciuta");
        assert_eq!(data_leggibile("ieri"), "ieri");
    }
}
