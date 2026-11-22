//! Archivio condiviso fra il thread di lavoro (che scrive) e il server MCP
//! (che legge): ultimi valori, storia limitata per sensore, statistiche,
//! ultima diagnosi, dati del collegamento.
//!
//! Il thread di lavoro lo aggiorna con [`Archivio::osserva`] a ogni evento
//! che emette: una sezione critica breve, senza I/O, quindi la lettura dei
//! sensori non rallenta. Il server non tocca mai l'adattatore.
//!
//! Un valore assente resta assente (`None`), con lo stato che ne dice il
//! motivo: mai zero.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::diagnosis::DiagnosisReport;
use crate::model::{Marker, Origine, Sample, Stato};
use crate::obd::pids::{self, Param};
use crate::worker::{Evento, InfoConn, InfoRiproduzione, StatoConn, StatoRiproduzione};

/// Punti conservati per ogni sensore; oltre, si scartano i più vecchi.
pub const MAX_PUNTI_STORIA: usize = 2000;
/// Marcatori conservati.
pub const MAX_MARCATORI: usize = 200;

/// Una lettura nella storia di un sensore.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Punto {
    /// Secondi dall'inizio del collegamento (o posizione nella riproduzione).
    pub t: f64,
    /// `None` se la lettura è fallita.
    pub v: Option<f64>,
    pub stato: Stato,
}

/// Minimo, massimo e media dei soli valori validi.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Statistiche {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub conteggio: u64,
    pub somma: f64,
}

impl Statistiche {
    pub fn aggiungi(&mut self, v: f64) {
        self.min = Some(self.min.map_or(v, |m| m.min(v)));
        self.max = Some(self.max.map_or(v, |m| m.max(v)));
        self.conteggio += 1;
        self.somma += v;
    }

    pub fn media(&self) -> Option<f64> {
        (self.conteggio > 0).then(|| self.somma / self.conteggio as f64)
    }
}

/// Ciò che si sa di un sensore dall'inizio del collegamento.
#[derive(Clone, Debug)]
pub struct Sensore {
    /// Centralina di riferimento (la prima che ha risposto), come nei grafici.
    pub ecu: String,
    pub unita: String,
    pub ultimo: Punto,
    /// Data e ora dell'ultima lettura (ISO 8601).
    pub ts: String,
    /// Quando è arrivata l'ultima lettura (orologio del PC).
    pub arrivo: Instant,
    pub stat: Statistiche,
    /// Letture ricevute, fallite comprese.
    pub letture: u64,
    /// Letture fallite.
    pub mancanti: u64,
    pub storia: VecDeque<Punto>,
}

/// Dati del collegamento in corso.
#[derive(Clone, Debug, PartialEq)]
pub struct Collegamento {
    pub origine: Origine,
    pub collegamento: String,
    pub adattatore: String,
    pub protocollo: String,
    pub ecu: Vec<String>,
    pub unione: BTreeSet<u8>,
    pub liberi: BTreeSet<String>,
    pub riproduzione: Option<InfoRiproduzione>,
    pub avvisi: Vec<String>,
}

impl Collegamento {
    fn da_info(i: &InfoConn) -> Self {
        Collegamento {
            origine: i.origine,
            collegamento: i.collegamento.clone(),
            adattatore: i.adattatore.clone(),
            protocollo: i.protocollo.clone(),
            ecu: i.ecu.clone(),
            unione: i.unione.clone(),
            liberi: i.liberi.clone(),
            riproduzione: i.riproduzione.clone(),
            avvisi: i.avvisi.clone(),
        }
    }

    /// Vero se la sorgente offre il parametro (stessa regola della pagina
    /// Sensori).
    pub fn offre(&self, p: &Param) -> bool {
        if p.libero {
            self.liberi.contains(&p.key)
        } else if p.derivato {
            pids::sorgente_derivato(&p.key)
                .and_then(pids::per_key)
                .is_some_and(|s| self.unione.contains(&s.pid))
        } else {
            self.unione.contains(&p.pid)
        }
    }
}

/// Diagnosi letta, con l'origine dei dati al momento della lettura.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosiLetta {
    /// Senza VIN: viene tolto prima di entrare nell'archivio.
    pub referto: DiagnosisReport,
    pub origine: Origine,
}

/// Registrazione in corso.
#[derive(Clone, Debug, PartialEq)]
pub struct RegistrazioneInCorso {
    /// Solo il nome del file, senza cartella.
    pub file: String,
    /// Tempo sull'asse dal vivo in cui è iniziata.
    pub t_inizio: f64,
}

/// Lo stato protetto dal mutex. I campi si leggono con [`Archivio::leggi`].
#[derive(Debug)]
pub struct Interno {
    pub stato: StatoConn,
    pub collegamento: Option<Collegamento>,
    pub lettura: bool,
    /// Chiavi chieste con l'ultimo avvio della lettura.
    pub chiavi_lettura: BTreeSet<String>,
    pub sensori: BTreeMap<String, Sensore>,
    /// Tempo dell'ultima lettura ricevuta.
    pub t_ultimo: f64,
    pub marcatori: VecDeque<Marker>,
    pub diagnosi: Option<DiagnosiLetta>,
    pub registrazione: Option<RegistrazioneInCorso>,
    /// Nome del file e numero di campioni dell'ultima registrazione chiusa.
    pub ultima_registrazione: Option<(String, usize)>,
    pub riproduzione: Option<StatoRiproduzione>,
    /// Titolo e testo dell'ultimo errore mostrato all'utente.
    pub errore: Option<(String, String)>,
    // Contatori che crescono soltanto: servono a chi attende un cambiamento.
    pub n_diagnosi: u64,
    pub n_marcatori: u64,
    pub n_errori: u64,
    pub n_registrazioni_chiuse: u64,
}

impl Default for Interno {
    fn default() -> Self {
        Interno {
            stato: StatoConn::Scollegato,
            collegamento: None,
            lettura: false,
            chiavi_lettura: BTreeSet::new(),
            sensori: BTreeMap::new(),
            t_ultimo: 0.0,
            marcatori: VecDeque::new(),
            diagnosi: None,
            registrazione: None,
            ultima_registrazione: None,
            riproduzione: None,
            errore: None,
            n_diagnosi: 0,
            n_marcatori: 0,
            n_errori: 0,
            n_registrazioni_chiuse: 0,
        }
    }
}

impl Interno {
    /// Origine dei dati correnti, se c'è un collegamento.
    pub fn origine(&self) -> Option<Origine> {
        self.collegamento.as_ref().map(|c| c.origine)
    }

    pub fn in_riproduzione(&self) -> bool {
        self.origine() == Some(Origine::Riproduzione)
    }

    /// Parametri offerti dalla sorgente collegata, per gruppo e poi per PID;
    /// da scollegati, quelli di cui restano dati in memoria.
    pub fn parametri(&self) -> Vec<&'static Param> {
        let liberi = pids::liberi();
        let mut v: Vec<&'static Param> = pids::catalogo()
            .iter()
            .chain(liberi)
            .filter(|p| match &self.collegamento {
                Some(c) => c.offre(p),
                None => self.sensori.contains_key(&p.key),
            })
            .collect();
        v.sort_by_key(|p| (p.gruppo, p.derivato, p.libero, p.pid));
        v
    }

    /// Vero se il parametro è fra quelli che la lettura in corso aggiorna.
    pub fn in_lettura(&self, p: &Param) -> bool {
        if self.in_riproduzione() {
            // In riproduzione scorrono tutti i dati della sessione.
            return self.sensori.contains_key(&p.key);
        }
        if !self.lettura {
            return false;
        }
        match pids::sorgente_derivato(&p.key) {
            // Un calcolato segue la sua sorgente.
            Some(sorgente) if p.derivato => self.chiavi_lettura.contains(sorgente),
            _ => {
                // Parametri dello stesso PID arrivano insieme.
                self.chiavi_lettura.contains(&p.key)
                    || (!p.libero
                        && !p.derivato
                        && self.chiavi_lettura.iter().any(|k| {
                            pids::per_key(k)
                                .is_some_and(|q| !q.derivato && !q.libero && q.pid == p.pid)
                        }))
            }
        }
    }

    fn aggiungi(&mut self, c: &Sample, ora: Instant) {
        let punto = Punto {
            t: c.t,
            v: c.valore.filter(|v| v.is_finite() && c.stato == Stato::Ok),
            stato: c.stato,
        };
        self.t_ultimo = self.t_ultimo.max(c.t);
        let s = match self.sensori.get_mut(&c.key) {
            Some(s) => s,
            None => self.sensori.entry(c.key.clone()).or_insert(Sensore {
                ecu: String::new(),
                unita: c.unita.clone(),
                ultimo: punto,
                ts: String::new(),
                arrivo: ora,
                stat: Statistiche::default(),
                letture: 0,
                mancanti: 0,
                storia: VecDeque::new(),
            }),
        };
        if !c.ecu.is_empty() {
            if s.ecu.is_empty() {
                s.ecu = c.ecu.clone();
            } else if s.ecu != c.ecu {
                // Un'altra centralina risponde allo stesso PID: come nei
                // grafici, non viene mescolata nella stessa serie.
                return;
            }
        }
        s.ultimo = punto;
        s.ts.clone_from(&c.ts);
        s.arrivo = ora;
        s.letture += 1;
        match punto.v {
            Some(v) => s.stat.aggiungi(v),
            None => s.mancanti += 1,
        }
        if s.storia.len() >= MAX_PUNTI_STORIA {
            s.storia.pop_front();
        }
        s.storia.push_back(punto);
    }

    fn svuota_dati(&mut self) {
        self.sensori.clear();
        self.t_ultimo = 0.0;
    }
}

/// L'archivio: un mutex e una condizione per chi attende un cambiamento.
#[derive(Debug, Default)]
pub struct Archivio {
    interno: Mutex<Interno>,
    cambio: Condvar,
}

fn blocca<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Archivio {
    /// Legge lo stato. La funzione deve essere breve e non fare I/O: mentre
    /// gira, il thread di lavoro attende.
    pub fn leggi<R>(&self, f: impl FnOnce(&Interno) -> R) -> R {
        f(&blocca(&self.interno))
    }

    fn scrivi(&self, f: impl FnOnce(&mut Interno)) {
        f(&mut blocca(&self.interno));
        self.cambio.notify_all();
    }

    /// Aggiorna l'archivio con un evento del thread di lavoro. Gli eventi
    /// che non lo riguardano non prendono nemmeno il mutex.
    pub fn osserva(&self, e: &Evento) {
        match e {
            Evento::Stato(s) => self.scrivi(|i| i.stato = s.clone()),
            Evento::Connesso(info) => self.scrivi(|i| {
                i.collegamento = Some(Collegamento::da_info(info));
                i.svuota_dati();
                i.marcatori.clear();
                i.diagnosi = None;
                i.lettura = false;
                i.chiavi_lettura.clear();
                i.registrazione = None;
                i.riproduzione = None;
                i.errore = None;
            }),
            Evento::Diagnosi(d) => self.scrivi(|i| {
                let mut referto = (**d).clone();
                // Il VIN non entra nell'archivio: così non può uscirne.
                referto.vin = None;
                let origine = i.origine().unwrap_or(if referto.demo {
                    Origine::Demo
                } else {
                    Origine::Veicolo
                });
                i.diagnosi = Some(DiagnosiLetta { referto, origine });
                i.n_diagnosi += 1;
            }),
            Evento::Campioni(v) => {
                if v.is_empty() {
                    return;
                }
                let ora = Instant::now();
                self.scrivi(|i| {
                    for c in v {
                        i.aggiungi(c, ora);
                    }
                });
            }
            Evento::Polling(attivo) => self.scrivi(|i| {
                i.lettura = *attivo;
                if !attivo {
                    i.chiavi_lettura.clear();
                }
            }),
            Evento::RegistrazioneAvviata { percorso, t_inizio } => self.scrivi(|i| {
                i.registrazione = Some(RegistrazioneInCorso {
                    file: nome_file(percorso),
                    t_inizio: *t_inizio,
                });
            }),
            Evento::RegistrazioneFermata {
                percorso, campioni, ..
            } => self.scrivi(|i| {
                i.registrazione = None;
                i.ultima_registrazione = Some((nome_file(percorso), *campioni));
                i.n_registrazioni_chiuse += 1;
            }),
            Evento::Marcatore(m) => self.scrivi(|i| {
                if i.marcatori.len() >= MAX_MARCATORI {
                    i.marcatori.pop_front();
                }
                i.marcatori.push_back(m.clone());
                i.n_marcatori += 1;
            }),
            Evento::Scollegato { .. } => self.scrivi(|i| {
                if i.in_riproduzione() {
                    // Chiusa la riproduzione, i suoi dati non restano a
                    // confondersi con quelli di un collegamento futuro.
                    i.svuota_dati();
                    i.marcatori.clear();
                    i.diagnosi = None;
                }
                i.collegamento = None;
                i.lettura = false;
                i.chiavi_lettura.clear();
                i.registrazione = None;
                i.riproduzione = None;
            }),
            Evento::Riproduzione(s) => self.scrivi(|i| i.riproduzione = Some(s.clone())),
            // La posizione è saltata: la storia riparte, come i grafici.
            Evento::Salto { .. } => self.scrivi(|i| i.svuota_dati()),
            Evento::Errore(e) => self.scrivi(|i| {
                i.errore = Some((e.titolo().to_string(), e.messaggio()));
                i.n_errori += 1;
            }),
            Evento::DispositiviBle(_)
            | Evento::PorteSeriali(_)
            | Evento::GattSconosciuto(_)
            | Evento::CancellazioneDtc(_)
            | Evento::Registro(_)
            | Evento::Mqtt(_)
            | Evento::Mcp(_) => {}
        }
    }

    /// Ricorda quali chiavi sono state chieste con l'avvio della lettura.
    pub fn chiavi_lettura(&self, chiavi: &[String], prioritarie: &[String]) {
        self.scrivi(|i| {
            i.chiavi_lettura = chiavi.iter().chain(prioritarie).cloned().collect();
        });
    }

    /// Attende che `pronto` restituisca un valore, al più per `durata`.
    /// `None` se il tempo scade o se `stop` diventa vero (server in
    /// chiusura): chi attende non resta mai appeso.
    pub fn attendi<T>(
        &self,
        durata: Duration,
        stop: &AtomicBool,
        mut pronto: impl FnMut(&Interno) -> Option<T>,
    ) -> Option<T> {
        let scadenza = Instant::now() + durata;
        let mut g = blocca(&self.interno);
        loop {
            if let Some(x) = pronto(&g) {
                return Some(x);
            }
            let resto = scadenza.saturating_duration_since(Instant::now());
            if resto.is_zero() || stop.load(Ordering::Relaxed) {
                return None;
            }
            g = self
                .cambio
                .wait_timeout(g, resto.min(Duration::from_millis(100)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

fn nome_file(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Archivi già popolati per le prove degli altri moduli.
#[cfg(test)]
pub(crate) mod prova {
    use super::*;
    use crate::model::adesso_iso;
    use crate::worker::ErroreUtente;

    pub fn campione(key: &str, t: f64, valore: Option<f64>, stato: Stato) -> Sample {
        let p = pids::per_key(key).expect("chiave del catalogo");
        Sample {
            t,
            ts: adesso_iso(),
            ecu: if valore.is_some() {
                "7E8".into()
            } else {
                String::new()
            },
            key: key.to_string(),
            pid: p.pid,
            valore,
            unita: p.unita.to_string(),
            stato,
        }
    }

    pub fn info_demo() -> InfoConn {
        let unione: BTreeSet<u8> = [0x04, 0x05, 0x0C, 0x0D, 0x15].into_iter().collect();
        InfoConn {
            demo: true,
            origine: Origine::Demo,
            collegamento: "Simulatore (demo)".into(),
            adattatore: "ELM327 v1.5 (demo)".into(),
            protocollo: "ISO 15765-4 CAN (11 bit, 500 kbaud)".into(),
            protocollo_numero: 6,
            ecu: vec!["7E8".into()],
            supportati: [("7E8".to_string(), unione.clone())].into_iter().collect(),
            unione,
            ..Default::default()
        }
    }

    pub fn referto_demo() -> DiagnosisReport {
        use crate::diagnosis::{RigaDtc, StatoDtc};
        DiagnosisReport {
            letto_il: "2026-01-02T03:04:05.000+01:00".into(),
            demo: true,
            adattatore: "ELM327 v1.5 (demo)".into(),
            protocollo: "ISO 15765-4 CAN".into(),
            ecu: vec!["7E8".into()],
            dtc: vec![RigaDtc {
                codice: "P0420".into(),
                descrizione: "Efficienza catalizzatore sotto soglia (bancata 1)".into(),
                ecu: "7E8".into(),
                stati: vec![StatoDtc::Confermato, StatoDtc::Permanente],
            }],
            vin: Some("DEMO0000000000000".into()),
            ..Default::default()
        }
    }

    /// Collegato alla demo, lettura attiva su giri, velocità e sonda a valle
    /// (che non risponde nell'ultima lettura), diagnosi letta, un marcatore.
    pub fn popolato() -> Archivio {
        let a = Archivio::default();
        a.osserva(&Evento::Connesso(Box::new(info_demo())));
        a.osserva(&Evento::Stato(StatoConn::Connesso));
        a.chiavi_lettura(
            &[
                "giri".to_string(),
                "velocita".to_string(),
                "o2s2_tensione".to_string(),
            ],
            &[],
        );
        a.osserva(&Evento::Polling(true));
        let mut v = Vec::new();
        for i in 0..10 {
            let t = i as f64;
            v.push(campione("giri", t, Some(800.0 + 10.0 * t), Stato::Ok));
            v.push(campione("velocita", t, Some(0.0), Stato::Ok));
            v.push(campione("o2s2_tensione", t, Some(0.7), Stato::Ok));
        }
        v.push(campione(
            "o2s2_tensione",
            10.0,
            None,
            Stato::RispostaMancante,
        ));
        a.osserva(&Evento::Campioni(v));
        a.osserva(&Evento::Diagnosi(Box::new(referto_demo())));
        a.osserva(&Evento::Marcatore(Marker {
            t: 5.0,
            ts: adesso_iso(),
            etichetta: "Minimo".into(),
        }));
        a
    }

    #[test]
    fn valore_mancante_resta_assente_mai_zero() {
        let a = popolato();
        a.leggi(|i| {
            let s = &i.sensori["o2s2_tensione"];
            assert_eq!(s.ultimo.v, None);
            assert_eq!(s.ultimo.stato, Stato::RispostaMancante);
            assert_eq!(s.letture, 11);
            assert_eq!(s.mancanti, 1);
            // Le statistiche contano solo i valori validi.
            assert_eq!(s.stat.conteggio, 10);
            assert_eq!(s.stat.min, Some(0.7));
            // Una velocità davvero nulla è un valore, non un'assenza.
            assert_eq!(i.sensori["velocita"].ultimo.v, Some(0.0));
            assert_eq!(i.sensori["giri"].stat.media(), Some(845.0));
        });
        // Un valore non finito o con stato non valido non passa per buono.
        a.osserva(&Evento::Campioni(vec![
            campione("giri", 11.0, Some(f64::NAN), Stato::Ok),
            campione("velocita", 11.0, Some(12.0), Stato::ErroreComunicazione),
        ]));
        a.leggi(|i| {
            assert_eq!(i.sensori["giri"].ultimo.v, None);
            assert_eq!(i.sensori["velocita"].ultimo.v, None);
            assert_eq!(i.sensori["velocita"].stat.max, Some(0.0));
        });
    }

    #[test]
    fn storia_limitata() {
        let a = Archivio::default();
        a.osserva(&Evento::Connesso(Box::new(info_demo())));
        let v: Vec<Sample> = (0..MAX_PUNTI_STORIA + 500)
            .map(|i| campione("giri", i as f64 * 0.1, Some(i as f64), Stato::Ok))
            .collect();
        a.osserva(&Evento::Campioni(v));
        a.leggi(|i| {
            let s = &i.sensori["giri"];
            assert_eq!(s.storia.len(), MAX_PUNTI_STORIA);
            // Restano i più recenti; le statistiche coprono tutto.
            assert_eq!(s.storia.front().unwrap().v, Some(500.0));
            assert_eq!(s.letture as usize, MAX_PUNTI_STORIA + 500);
            assert_eq!(s.stat.min, Some(0.0));
        });
        for n in 0..MAX_MARCATORI + 10 {
            a.osserva(&Evento::Marcatore(Marker {
                t: n as f64,
                ts: String::new(),
                etichetta: format!("m{n}"),
            }));
        }
        a.leggi(|i| {
            assert_eq!(i.marcatori.len(), MAX_MARCATORI);
            assert_eq!(i.n_marcatori as usize, MAX_MARCATORI + 10);
        });
    }

    #[test]
    fn il_vin_non_entra_nell_archivio() {
        let a = popolato();
        a.leggi(|i| {
            let d = i.diagnosi.as_ref().unwrap();
            assert_eq!(d.referto.vin, None);
            assert_eq!(d.origine, Origine::Demo);
            assert_eq!(d.referto.dtc[0].codice, "P0420");
            assert!(!format!("{i:?}").contains("DEMO0000000000000"));
        });
    }

    #[test]
    fn collegamento_e_scollegamento() {
        let a = popolato();
        a.leggi(|i| {
            assert_eq!(i.origine(), Some(Origine::Demo));
            assert!(i.lettura);
            let chiavi: Vec<&str> = i.parametri().iter().map(|p| p.key.as_str()).collect();
            assert!(chiavi.contains(&"giri") && chiavi.contains(&"velocita"));
            // La distanza parziale è calcolata dalla velocità.
            assert!(chiavi.contains(&pids::CHIAVE_DISTANZA_PARZIALE));
            assert!(!chiavi.contains(&"temp_olio"));
            assert!(i.in_lettura(pids::per_key("giri").unwrap()));
            assert!(i.in_lettura(pids::per_key(pids::CHIAVE_DISTANZA_PARZIALE).unwrap()));
            assert!(!i.in_lettura(pids::per_key("temp_refrigerante").unwrap()));
        });
        a.osserva(&Evento::Errore(ErroreUtente::Altro("prova".into())));
        a.osserva(&Evento::Polling(false));
        a.osserva(&Evento::Scollegato { motivo: None });
        a.leggi(|i| {
            assert_eq!(i.origine(), None);
            assert!(!i.lettura);
            assert_eq!(i.n_errori, 1);
            // Dal vivo i dati letti restano consultabili dopo lo scollegamento.
            assert!(i.sensori.contains_key("giri"));
            assert!(i.diagnosi.is_some());
            assert!(!i.in_lettura(pids::per_key("giri").unwrap()));
        });
        // Un nuovo collegamento riparte da zero.
        a.osserva(&Evento::Connesso(Box::new(info_demo())));
        a.leggi(|i| {
            assert!(i.sensori.is_empty());
            assert!(i.diagnosi.is_none());
            assert!(i.marcatori.is_empty());
        });
    }

    #[test]
    fn riproduzione_non_lascia_tracce_alla_chiusura() {
        let a = Archivio::default();
        let mut info = info_demo();
        info.origine = Origine::Riproduzione;
        info.riproduzione = Some(InfoRiproduzione {
            file: "s.obdio.jsonl".into(),
            registrata_il: "2026-01-01T10:00:00+01:00".into(),
            sessione_demo: true,
            importata: None,
        });
        a.osserva(&Evento::Connesso(Box::new(info)));
        a.osserva(&Evento::Diagnosi(Box::new(referto_demo())));
        a.osserva(&Evento::Campioni(vec![campione(
            "giri",
            1.0,
            Some(900.0),
            Stato::Ok,
        )]));
        a.leggi(|i| {
            assert!(i.in_riproduzione());
            assert_eq!(i.diagnosi.as_ref().unwrap().origine, Origine::Riproduzione);
            assert!(i.in_lettura(pids::per_key("giri").unwrap()));
        });
        // Un salto svuota la storia, come i grafici.
        a.osserva(&Evento::Salto { posizione: 30.0 });
        a.leggi(|i| assert!(i.sensori.is_empty()));
        a.osserva(&Evento::Campioni(vec![campione(
            "giri",
            30.0,
            Some(900.0),
            Stato::Ok,
        )]));
        a.osserva(&Evento::Scollegato { motivo: None });
        a.leggi(|i| {
            assert!(i.sensori.is_empty());
            assert!(i.diagnosi.is_none());
        });
    }

    #[test]
    fn attesa_con_scadenza_e_interruzione() {
        let a = std::sync::Arc::new(Archivio::default());
        let stop = AtomicBool::new(false);
        // Scade senza restare appesa.
        let inizio = Instant::now();
        assert_eq!(
            a.attendi(Duration::from_millis(60), &stop, |_| None::<()>),
            None
        );
        assert!(inizio.elapsed() < Duration::from_secs(2));
        // Si sveglia quando l'archivio cambia.
        let b = a.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            b.osserva(&Evento::Polling(true));
        });
        assert_eq!(
            a.attendi(Duration::from_secs(5), &stop, |i| i.lettura.then_some(7)),
            Some(7)
        );
        h.join().unwrap();
        // Con il server in chiusura esce subito.
        stop.store(true, Ordering::Relaxed);
        let inizio = Instant::now();
        assert_eq!(
            a.attendi(Duration::from_secs(30), &stop, |_| None::<()>),
            None
        );
        assert!(inizio.elapsed() < Duration::from_secs(2));
    }
}
