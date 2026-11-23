//! Sessione diagnostica con la centralina Td5: apertura (inizializzazione,
//! sessione, accesso di sicurezza), mantenimento, ripristino trasparente
//! quando cade, letture.
//!
//! Sequenza (ricerca/td5/proto.md): inizializzazione rapida con `81 13 F7
//! 81`, `10 A0` → `50 A0`, `27 01` → seme, `27 02 <chiave>` → `67 02`. Senza
//! messaggi per circa 2 s la centralina chiude la sessione: OBDio manda
//! TesterPresent dopo 1 s di silenzio ([`SessioneTd5::mantieni`]).

use std::time::{Duration, Instant};

use super::DiagnosiTd5;
use super::decodifica::{self, Valore};
use super::frame::{self, Risposta};
use super::guasti;
use super::richieste::{Chiave, IdLocale, RichiestaTd5};
use super::stn::{CanaleTd5, ErroreTd5};
use crate::diagnosis::{DiagnosisReport, Lettura};
use crate::model::{Stato, adesso_iso};

/// Silenzio dopo il quale si manda TesterPresent (la centralina chiude dopo
/// circa 2 s; proto.md chiede almeno uno ogni 1,5 s).
pub const INTERVALLO_MANTENIMENTO: Duration = Duration::from_millis(1000);

pub struct SessioneTd5 {
    canale: Box<dyn CanaleTd5>,
    ultimo_scambio: Instant,
    /// Quante volte la sessione è stata riaperta dopo una caduta.
    pub ripristini: u32,
}

impl SessioneTd5 {
    pub fn new(canale: Box<dyn CanaleTd5>) -> Self {
        SessioneTd5 {
            canale,
            ultimo_scambio: Instant::now(),
            ripristini: 0,
        }
    }

    pub fn canale(&mut self) -> &mut dyn CanaleTd5 {
        self.canale.as_mut()
    }

    pub fn descrizione(&self) -> String {
        self.canale.descrizione()
    }

    pub fn is_demo(&self) -> bool {
        self.canale.is_demo()
    }

    pub fn strategia(&self) -> Option<String> {
        self.canale.strategia()
    }

    pub fn traccia(&self) -> bool {
        self.canale.traccia()
    }

    /// Apre la sessione su un adattatore già preparato
    /// ([`CanaleTd5::prepara`]).
    pub fn apri_sessione(&mut self) -> Result<(), ErroreTd5> {
        self.apri()
    }

    /// Prepara l'adattatore e apre la sessione. Restituisce l'identificazione
    /// dell'adattatore.
    pub fn collega(&mut self) -> Result<String, ErroreTd5> {
        let identita = self.canale.prepara()?;
        self.apri()?;
        Ok(identita)
    }

    fn scambio(&mut self, r: RichiestaTd5) -> Result<Risposta, ErroreTd5> {
        let esito = self.canale.richiesta(r);
        // Ogni messaggio mandato tiene viva la sessione, risposta o no.
        self.ultimo_scambio = Instant::now();
        esito
    }

    /// Inizializzazione, sessione diagnostica e accesso di sicurezza.
    fn apri(&mut self) -> Result<(), ErroreTd5> {
        self.canale.avvia_sessione()?;
        self.ultimo_scambio = Instant::now();
        let seme = match self.scambio(RichiestaTd5::RichiestaSeme)? {
            Risposta::Positiva(c) => match c.as_slice() {
                [0x67, 0x01, a, b, ..] => u16::from_be_bytes([*a, *b]),
                _ => {
                    return Err(ErroreTd5::Trama(format!(
                        "risposta inattesa alla richiesta del seme: {}",
                        frame::esadecimale(&c)
                    )));
                }
            },
            Risposta::Negativa { codice, .. } => {
                return Err(ErroreTd5::AccessoNegato(format!(
                    "la centralina rifiuta la richiesta del seme: {}",
                    frame::motivo_negativo(codice)
                )));
            }
        };
        match self.scambio(RichiestaTd5::InvioChiave(Chiave::da_seme(seme)))? {
            Risposta::Positiva(c) if c.starts_with(&[0x67, 0x02]) => {}
            Risposta::Positiva(c) => {
                return Err(ErroreTd5::Trama(format!(
                    "risposta inattesa all'invio della chiave: {}",
                    frame::esadecimale(&c)
                )));
            }
            Risposta::Negativa { codice, .. } => {
                return Err(ErroreTd5::AccessoNegato(format!(
                    "la centralina rifiuta la chiave calcolata dal seme {seme:04X}: {}. Può \
                     essere una versione di centralina con un algoritmo diverso",
                    frame::motivo_negativo(codice)
                )));
            }
        }
        self.canale
            .annota("Accesso di sicurezza concesso: sessione Td5 pronta".into());
        Ok(())
    }

    /// TesterPresent: `Ok(true)` se la centralina risponde positivamente.
    fn viva(&mut self) -> Result<bool, ErroreTd5> {
        match self.scambio(RichiestaTd5::Mantenimento) {
            Ok(Risposta::Positiva(c)) => Ok(c.first() == Some(&0x7E)),
            Ok(Risposta::Negativa { .. }) => Ok(false),
            Err(e) if e.fatale() => Err(e),
            Err(_) => Ok(false),
        }
    }

    /// Riapre la sessione una volta. Se non riesce la sessione è persa.
    fn ripristina(&mut self, motivo: &str) -> Result<(), ErroreTd5> {
        self.ripristini += 1;
        self.canale.annota(format!(
            "Sessione Td5 caduta ({motivo}): nuova inizializzazione (ripristino n. {})",
            self.ripristini
        ));
        match self.apri() {
            Ok(()) => Ok(()),
            Err(ErroreTd5::Scollegato(m)) => Err(ErroreTd5::Scollegato(m)),
            Err(e) => {
                self.canale
                    .annota(format!("Nuova inizializzazione non riuscita: {e}"));
                Err(ErroreTd5::SessionePersa(e.to_string()))
            }
        }
    }

    /// Esegue una richiesta. Se non arriva una risposta positiva si controlla
    /// con TesterPresent che la sessione sia viva; se non lo è la si riapre
    /// (una volta) e si ripete la richiesta. Errori fatali: collegamento
    /// caduto o sessione persa.
    pub fn richiesta(&mut self, r: RichiestaTd5) -> Result<Risposta, ErroreTd5> {
        let primo = self.scambio(r);
        match &primo {
            Ok(Risposta::Positiva(_)) => return primo,
            Err(e) if e.fatale() => return primo,
            _ => {}
        }
        if self.viva()? {
            // La sessione c'è: il rifiuto o il silenzio riguardano solo questa
            // richiesta.
            return primo;
        }
        let motivo = match &primo {
            Ok(Risposta::Negativa { codice, .. }) => {
                format!("risposta negativa: {}", frame::motivo_negativo(*codice))
            }
            Err(e) => e.to_string(),
            Ok(Risposta::Positiva(_)) => String::new(),
        };
        self.ripristina(&motivo)?;
        self.scambio(r)
    }

    /// Manda TesterPresent se da [`INTERVALLO_MANTENIMENTO`] non passa alcun
    /// messaggio. Se la centralina non risponde riapre la sessione (una volta).
    pub fn mantieni(&mut self) -> Result<(), ErroreTd5> {
        if self.ultimo_scambio.elapsed() < INTERVALLO_MANTENIMENTO {
            return Ok(());
        }
        if self.viva()? {
            return Ok(());
        }
        self.ripristina("nessuna risposta al mantenimento")
    }

    /// Legge un identificativo. `Err` solo se l'errore è fatale: rifiuti,
    /// silenzi e risposte illeggibili diventano stati dei valori.
    pub fn leggi(&mut self, id: IdLocale) -> Result<Vec<Valore>, ErroreTd5> {
        Ok(match self.richiesta(RichiestaTd5::Dati(id)) {
            Ok(Risposta::Positiva(c)) => decodifica::decodifica(id, &c),
            Ok(Risposta::Negativa { codice, .. }) => decodifica::con_stato(
                id,
                if matches!(codice, 0x11 | 0x12 | 0x31) {
                    Stato::NonSupportato
                } else {
                    Stato::RispostaMancante
                },
            ),
            Err(ErroreTd5::NessunaRisposta) => decodifica::con_stato(id, Stato::RispostaMancante),
            Err(e) if e.fatale() => return Err(e),
            Err(_) => decodifica::con_stato(id, Stato::ErroreComunicazione),
        })
    }

    /// Legge una volta ogni identificativo: i valori con stato `Ok` dicono
    /// quali parametri questa centralina fornisce.
    pub fn sonda(&mut self) -> Result<Vec<Valore>, ErroreTd5> {
        let mut v = Vec::new();
        for id in IdLocale::TUTTI {
            v.extend(self.leggi(id)?);
        }
        Ok(v)
    }

    /// Lettura diagnostica: codici guasto, tipo di centralina, variante della
    /// mappa, tensione della batteria e (solo in memoria) il numero di telaio.
    pub fn diagnosi(
        &mut self,
        adattatore: &str,
        protocollo: &str,
    ) -> Result<DiagnosisReport, ErroreTd5> {
        let non_td5 = |d: &str| Lettura {
            stato: Stato::NonSupportato,
            dettaglio: d.to_string(),
        };
        let mut r = DiagnosisReport {
            letto_il: adesso_iso(),
            demo: self.is_demo(),
            adattatore: adattatore.to_string(),
            collegamento: self.descrizione(),
            protocollo: protocollo.to_string(),
            ecu: vec!["Td5".into()],
            lettura_prontezza: non_td5(
                "Il Td5 non ha la spia MIL né i monitor di prontezza dell'OBD-II",
            ),
            lettura_in_sospeso: non_td5("Il Td5 non distingue i codici in sospeso"),
            lettura_permanenti: non_td5("Il Td5 non ha codici permanenti"),
            ..Default::default()
        };
        r.fermo.lettura = non_td5("Il Td5 non ha il fermo immagine dell'OBD-II");
        r.mode06.lettura = non_td5("Il Td5 non ha il Mode 06");

        let lettura = |stato: Stato, dettaglio: String| Lettura { stato, dettaglio };
        let mut td5 = DiagnosiTd5 {
            strategia: self.strategia(),
            ..Default::default()
        };
        td5.lettura_guasti = match self.richiesta(RichiestaTd5::CodiciGuasto) {
            Ok(Risposta::Positiva(c)) => match guasti::decodifica(&c) {
                Ok(g) => {
                    r.dtc = g.iter().map(|x| x.riga()).collect();
                    lettura(Stato::Ok, String::new())
                }
                Err(m) => lettura(Stato::ErroreComunicazione, m),
            },
            Ok(Risposta::Negativa { codice, .. }) => lettura(
                Stato::NonSupportato,
                format!(
                    "La centralina rifiuta la lettura: {}",
                    frame::motivo_negativo(codice)
                ),
            ),
            Err(ErroreTd5::NessunaRisposta) => lettura(
                Stato::RispostaMancante,
                "Nessuna risposta dalla centralina".into(),
            ),
            Err(e) if e.fatale() => return Err(e),
            Err(e) => lettura(Stato::ErroreComunicazione, e.to_string()),
        };
        r.lettura_confermati = td5.lettura_guasti.clone();

        td5.tipo_centralina = self.testo(RichiestaTd5::TipoCentralina, [0x5A, 0x9A])?;
        td5.variante_mappa = self.testo(RichiestaTd5::VarianteMappa, [0x61, 0x32])?;
        td5.tensione_batteria = self
            .leggi(IdLocale::Batteria)?
            .into_iter()
            .find(|v| v.stato == Stato::Ok)
            .and_then(|v| v.valore);
        r.vin = match self.richiesta(RichiestaTd5::Vin) {
            Ok(Risposta::Positiva(c)) => vin_da(&c),
            Err(e) if e.fatale() => return Err(e),
            _ => None,
        };
        r.td5 = Some(td5);
        Ok(r)
    }

    /// Dato identificativo in testo: ASCII se stampabile, altrimenti in
    /// esadecimale. `None` se la centralina non lo fornisce.
    fn testo(&mut self, r: RichiestaTd5, testa: [u8; 2]) -> Result<Option<String>, ErroreTd5> {
        Ok(match self.richiesta(r) {
            Ok(Risposta::Positiva(c)) => c.strip_prefix(&testa).and_then(testo_da),
            Err(e) if e.fatale() => return Err(e),
            _ => None,
        })
    }

    pub fn chiudi(&mut self) {
        self.canale.chiudi();
    }
}

/// ASCII stampabile, oppure i byte in esadecimale (formato non documentato).
fn testo_da(d: &[u8]) -> Option<String> {
    if d.is_empty() {
        return None;
    }
    if d.iter().all(|b| (0x20..0x7F).contains(b)) {
        let t = String::from_utf8_lossy(d).trim().to_string();
        return (!t.is_empty()).then_some(t);
    }
    Some(format!("(byte) {}", frame::esadecimale(d)))
}

/// Primi 17 caratteri alfanumerici consecutivi dopo `5A 87`.
fn vin_da(corpo: &[u8]) -> Option<String> {
    let d = corpo.strip_prefix(&[0x5A, 0x87])?;
    d.windows(17)
        .find(|w| w.iter().all(|b| b.is_ascii_alphanumeric()))
        .map(|w| String::from_utf8_lossy(w).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::td5::simulato::SimulatoreTd5;
    use crate::td5::stn::{Stn, Strategia};

    fn sessione(sim: SimulatoreTd5) -> SessioneTd5 {
        SessioneTd5::new(Box::new(
            Stn::new(Box::new(sim)).con_pausa(Duration::from_millis(5)),
        ))
    }

    fn strategia_di(s: &SessioneTd5) -> String {
        s.strategia().unwrap_or_default()
    }

    fn valore(v: &[Valore], k: &str) -> Option<f64> {
        v.iter()
            .find(|x| x.chiave == k && x.stato == Stato::Ok)
            .and_then(|x| x.valore)
    }

    #[test]
    fn strategia_a_quando_funziona_tutto() {
        let sim = SimulatoreTd5::new();
        let c = sim.controllo();
        let mut s = sessione(sim);
        let id = s.collega().unwrap();
        assert!(id.contains("OBDLink CX") && id.contains("STN2255"), "{id}");
        assert_eq!(strategia_di(&s), Strategia::A.descrizione());
        let comandi = c.comandi();
        let attesi = [
            "ATZ",
            "ATE0",
            "STI",
            "STDI",
            "ATL0",
            "ATS1",
            "ATH1",
            "STP 21",
            "STPBR 10400",
            "STPTO 500",
            "STPTRQ 50",
            "STIP1X 40",
            "STIFI 25, 25, 8113F7810C",
            "0210A0",
            "022701",
        ];
        assert_eq!(&comandi[..attesi.len()], attesi);
        // Chiave calcolata dal seme 34A5 del simulatore: 54D3.
        assert_eq!(comandi[attesi.len()], "04270254D3");
        assert_eq!(
            c.messaggi()[..3],
            [
                vec![0x10, 0xA0],
                vec![0x27, 0x01],
                vec![0x27, 0x02, 0x54, 0xD3]
            ]
        );
        let v = s.leggi(IdLocale::Giri).unwrap();
        assert_eq!(valore(&v, "giri"), Some(744.0));
    }

    #[test]
    fn ripiego_su_b_poi_su_c() {
        // Solo B.
        let sim = SimulatoreTd5::new().con_strategie(false, true, false);
        let c = sim.controllo();
        let mut s = sessione(sim);
        s.collega().unwrap();
        assert_eq!(strategia_di(&s), Strategia::B.descrizione());
        let comandi = c.comandi();
        let i = comandi.iter().position(|x| x == "STP 25").unwrap();
        let dopo: Vec<&str> = comandi[i..].iter().map(String::as_str).collect();
        assert_eq!(
            dopo[..10],
            [
                "STP 25",
                "STPBR 10400",
                "STPTO 500",
                "STPTRQ 50",
                "STIP1X 40",
                "STIFI 25, 25, 8113F7810C",
                "STP 21",
                "STPTO 500",
                "STPTRQ 50",
                "0210A0"
            ]
        );
        assert!(valore(&s.leggi(IdLocale::Giri).unwrap(), "giri").is_some());

        // Solo C: A rifiuta STIFI, B perde la sessione al cambio di protocollo.
        let sim = SimulatoreTd5::new().con_strategie(false, false, true);
        let c = sim.controllo();
        let mut s = sessione(sim);
        s.collega().unwrap();
        assert_eq!(strategia_di(&s), Strategia::C.descrizione());
        let comandi = c.comandi();
        assert!(comandi.contains(&"ATSW 00".to_string()));
        assert!(comandi.contains(&"STPX h:0210A0, d:, r:1".to_string()));
        assert!(comandi.contains(&"STPX h:042702, d:54D3, r:1".to_string()));
        assert_eq!(c.inizializzazioni(), 3);
        let v = s.leggi(IdLocale::Temperature).unwrap();
        assert!((valore(&v, "temp_refrigerante").unwrap() - 70.6).abs() < 1e-9);
        // Il mantenimento passa anch'esso per STPX.
        std::thread::sleep(INTERVALLO_MANTENIMENTO + Duration::from_millis(20));
        s.mantieni().unwrap();
        assert_eq!(
            c.comandi().last().map(String::as_str),
            Some("STPX h:023E01, d:, r:1")
        );
    }

    #[test]
    fn nessuna_strategia_funziona() {
        let sim = SimulatoreTd5::new().con_strategie(false, false, false);
        let mut s = sessione(sim);
        let e = s.collega().unwrap_err();
        let ErroreTd5::Inizializzazione(m) = &e else {
            panic!("{e:?}")
        };
        assert!(
            m.contains("A:") && m.contains("B:") && m.contains("C:"),
            "{m}"
        );
        // Il registro dice che cosa ha risposto l'adattatore a ogni passo.
        let registro = s.canale().prendi_registro().join("\n");
        assert!(
            registro.contains("→ STIFI 25, 25, 8113F7810C   ← ?"),
            "{registro}"
        );
        assert!(registro.contains("→ 0210A0   ← NO DATA"), "{registro}");
        assert!(
            registro.contains("→ STPX h:0210A0, d:, r:1   ← NO DATA"),
            "{registro}"
        );
    }

    #[test]
    fn adattatore_senza_stn() {
        let mut s = sessione(SimulatoreTd5::new().senza_stn());
        let e = s.collega().unwrap_err();
        assert!(matches!(e, ErroreTd5::NonStn(_)), "{e:?}");
        let testo = e.to_string();
        assert!(
            testo.contains("ELM327") && testo.contains("OBDLink CX"),
            "{testo}"
        );
    }

    #[test]
    fn chiave_rifiutata() {
        let mut s = sessione(SimulatoreTd5::new().con_chiave_errata());
        let e = s.collega().unwrap_err();
        assert!(
            matches!(&e, ErroreTd5::AccessoNegato(m) if m.contains("chiave non valida (35)")),
            "{e:?}"
        );
    }

    #[test]
    fn sessione_caduta_ripristinata_una_volta() {
        let sim = SimulatoreTd5::new();
        let c = sim.controllo();
        let mut s = sessione(sim);
        s.collega().unwrap();
        assert_eq!(c.inizializzazioni(), 1);
        c.caduta_sessione
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // La lettura riesce lo stesso: TesterPresent senza risposta, nuova
        // inizializzazione con la stessa strategia, richiesta ripetuta.
        let v = s.leggi(IdLocale::Giri).unwrap();
        assert_eq!(valore(&v, "giri"), Some(744.0));
        assert_eq!(c.inizializzazioni(), 2);
        assert_eq!(s.ripristini, 1);
        let registro = s.canale().prendi_registro().join("\n");
        assert!(registro.contains("Sessione Td5 caduta"), "{registro}");
    }

    #[test]
    fn sessione_persa_se_il_ripristino_fallisce() {
        // Dopo la prima inizializzazione la centralina non si sveglia più.
        let sim = SimulatoreTd5::new().con_inizializzazioni_massime(1);
        let c = sim.controllo();
        let mut s = sessione(sim);
        s.collega().unwrap();
        c.caduta_sessione
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let e = s.leggi(IdLocale::Giri).unwrap_err();
        assert!(matches!(e, ErroreTd5::SessionePersa(_)), "{e:?}");
        assert!(e.fatale());
        // Un solo tentativo di ripristino, con la strategia già scelta.
        assert_eq!(c.inizializzazioni(), 2);
        assert_eq!(s.ripristini, 1);
    }

    #[test]
    fn mantenimento_a_riposo_tiene_viva_la_sessione() {
        let sim = SimulatoreTd5::new().con_scadenza(Duration::from_millis(1500));
        let c = sim.controllo();
        let mut s = sessione(sim);
        s.collega().unwrap();
        // 3 s a riposo chiamando `mantieni` come fa il thread di lavoro.
        let inizio = Instant::now();
        while inizio.elapsed() < Duration::from_secs(3) {
            s.mantieni().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(c.inizializzazioni(), 1, "la sessione non doveva cadere");
        let mantenimenti = c
            .messaggi()
            .iter()
            .filter(|m| m.as_slice() == [0x3E, 0x01])
            .count();
        assert!((2..=4).contains(&mantenimenti), "{mantenimenti}");
        assert!(valore(&s.leggi(IdLocale::Giri).unwrap(), "giri").is_some());
    }

    #[test]
    fn senza_mantenimento_la_sessione_cade_e_si_riapre() {
        let sim = SimulatoreTd5::new().con_scadenza(Duration::from_millis(300));
        let c = sim.controllo();
        let mut s = sessione(sim);
        s.collega().unwrap();
        std::thread::sleep(Duration::from_millis(400));
        // Il silenzio ha chiuso la sessione: la lettura la riapre da sola.
        assert!(valore(&s.leggi(IdLocale::Giri).unwrap(), "giri").is_some());
        assert_eq!(c.inizializzazioni(), 2);
    }

    #[test]
    fn identificativo_rifiutato_senza_ripristino() {
        let sim = SimulatoreTd5::new().senza_identificativi(&[IdLocale::Iniettori]);
        let c = sim.controllo();
        let mut s = sessione(sim);
        s.collega().unwrap();
        let v = s.leggi(IdLocale::Iniettori).unwrap();
        assert!(
            v.iter()
                .all(|x| x.stato == Stato::NonSupportato && x.valore.is_none())
        );
        // La sessione era viva: nessuna nuova inizializzazione.
        assert_eq!(c.inizializzazioni(), 1);
        let presenti: Vec<&str> = s
            .sonda()
            .unwrap()
            .into_iter()
            .filter(|v| v.stato == Stato::Ok)
            .map(|v| v.chiave)
            .collect();
        assert!(presenti.contains(&"giri") && !presenti.contains(&"td5_iniettore_1"));
    }

    #[test]
    fn diagnosi_con_guasti_e_vin_solo_in_memoria() {
        let mut s = sessione(SimulatoreTd5::new());
        s.collega().unwrap();
        s.canale().prendi_registro();
        let r = s.diagnosi("OBDLink CX", "Td5").unwrap();
        let codici: Vec<&str> = r.dtc.iter().map(|d| d.codice.as_str()).collect();
        assert_eq!(codici, ["02-4", "32-1"]);
        assert_eq!(r.vin.as_deref(), Some("TDCINQUE123456789"));
        let td5 = r.td5.as_ref().unwrap();
        assert_eq!(td5.tipo_centralina.as_deref(), Some("PROVA"));
        assert_eq!(td5.variante_mappa.as_deref(), Some("MAPPA PROVA"));
        assert!((td5.tensione_batteria.unwrap() - 13.891).abs() < 1e-9);
        assert_eq!(td5.lettura_guasti.stato, Stato::Ok);
        assert_eq!(r.lettura_prontezza.stato, Stato::NonSupportato);
        // Il VIN non compare nel registro né nel file della diagnosi.
        let registro = s.canale().prendi_registro().join("\n");
        assert!(registro.contains("→ 021A87   ← (risposta omessa dal registro)"));
        assert!(!registro.contains("TDCINQUE") && !registro.contains("54 44 43 49"));
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("TDCINQUE"));
        assert!(json.contains("\"td5\""));
    }

    #[test]
    fn mai_servizi_vietati_alla_centralina() {
        let sim = SimulatoreTd5::new();
        let c = sim.controllo();
        let mut s = sessione(sim);
        s.collega().unwrap();
        s.sonda().unwrap();
        s.diagnosi("", "").unwrap();
        std::thread::sleep(INTERVALLO_MANTENIMENTO);
        s.mantieni().unwrap();
        for m in c.messaggi() {
            assert!(
                matches!(m.first(), Some(0x10 | 0x27 | 0x3E | 0x21 | 0x1A)),
                "messaggio vietato: {m:02X?}"
            );
        }
    }

    #[test]
    fn vin_e_testi() {
        let mut c = vec![0x5A, 0x87, 0x00];
        c.extend_from_slice(b"SALLDHMF7XA123456");
        assert_eq!(vin_da(&c).as_deref(), Some("SALLDHMF7XA123456"));
        assert_eq!(vin_da(&[0x5A, 0x87, b'A']), None);
        assert_eq!(testo_da(b" MSB101 "), Some("MSB101".into()));
        assert_eq!(testo_da(&[0x01, 0xFF]), Some("(byte) 01 FF".into()));
        assert_eq!(testo_da(&[]), None);
    }
}
