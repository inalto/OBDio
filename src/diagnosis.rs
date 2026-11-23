//! Lettura diagnostica: DTC, spia MIL, prontezza, fermo immagine, Mode 06.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::elm::{Elm, Esito};
use crate::model::{Stato, adesso_iso};
use crate::obd::dtc::{decode_dtc, descrizione_o_nota, parse_dtc_message};
use crate::obd::mode06::{self, MID_CATALIZZATORE_B1, TestMonitor};
use crate::obd::pids::{
    self, Decodifica, KEY_STATO_CARBURANTE, NOME_STATO_CARBURANTE, stato_carburante,
};
use crate::obd::readiness::{self, Readiness};
use crate::obd::{EcuMessage, ObdRequest, supported_from_mask};
use crate::transport::Scollegato;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatoDtc {
    Confermato,
    InSospeso,
    Permanente,
}

impl StatoDtc {
    pub fn etichetta(self) -> &'static str {
        match self {
            StatoDtc::Confermato => "confermato",
            StatoDtc::InSospeso => "in sospeso",
            StatoDtc::Permanente => "permanente",
        }
    }
}

/// Un codice guasto di una centralina, con tutti gli stati in cui compare.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RigaDtc {
    pub codice: String,
    pub descrizione: String,
    pub ecu: String,
    pub stati: Vec<StatoDtc>,
}

/// Esito della lettura di un gruppo di dati.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lettura {
    pub stato: Stato,
    pub dettaglio: String,
}

impl Lettura {
    fn ok() -> Self {
        Lettura {
            stato: Stato::Ok,
            dettaglio: String::new(),
        }
    }

    fn da_esito(e: &Esito) -> Self {
        match e {
            Esito::Dati(_) => Lettura::ok(),
            Esito::NonSupportato => Lettura {
                stato: Stato::NonSupportato,
                dettaglio: "La centralina non supporta questo servizio".into(),
            },
            Esito::Mancante => Lettura {
                stato: Stato::RispostaMancante,
                dettaglio: "Nessuna risposta dalla centralina".into(),
            },
            Esito::Errore(m) => Lettura {
                stato: Stato::ErroreComunicazione,
                dettaglio: m.clone(),
            },
        }
    }
}

impl Default for Lettura {
    fn default() -> Self {
        Lettura {
            stato: Stato::RispostaMancante,
            dettaglio: "Non letto".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigaFermo {
    pub key: String,
    pub nome: String,
    pub pid: u8,
    pub valore: Option<f64>,
    /// Per i parametri descrittivi (stato del sistema carburante).
    pub testo: Option<String>,
    pub unita: String,
    pub stato: Stato,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FermoImmagine {
    pub lettura: Lettura,
    /// Vero se la centralina ha un fotogramma memorizzato.
    pub presente: bool,
    /// DTC che ha causato il fotogramma, come dichiarato dalla centralina.
    pub dtc: Option<String>,
    pub ecu: String,
    pub righe: Vec<RigaFermo>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SezioneMode06 {
    pub lettura: Lettura,
    pub test: Vec<TestMonitor>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosisReport {
    pub letto_il: String,
    pub demo: bool,
    pub adattatore: String,
    pub collegamento: String,
    pub protocollo: String,
    pub ecu: Vec<String>,
    pub tensione: Option<String>,
    pub lettura_prontezza: Lettura,
    pub prontezza: Vec<Readiness>,
    pub lettura_confermati: Lettura,
    pub lettura_in_sospeso: Lettura,
    pub lettura_permanenti: Lettura,
    pub dtc: Vec<RigaDtc>,
    pub fermo: FermoImmagine,
    pub mode06: SezioneMode06,
    /// PID del servizio 01 dichiarati da ogni centralina.
    pub pid_supportati: BTreeMap<String, Vec<u8>>,
    /// Tipo di carburante dichiarato (PID 51), se la centralina lo espone.
    #[serde(default)]
    pub tipo_carburante: Option<String>,
    /// Norma OBD dichiarata (PID 1C), se la centralina la espone.
    #[serde(default)]
    pub norma_obd: Option<String>,
    /// Mai scritto nei file: resta solo in memoria.
    #[serde(skip)]
    pub vin: Option<String>,
}

/// Dati del collegamento necessari alla lettura.
pub struct Contesto<'a> {
    pub adattatore: &'a str,
    pub protocollo: &'a str,
    pub supportati: &'a BTreeMap<String, BTreeSet<u8>>,
}

/// Parametri che il fermo immagine dovrebbe riportare: se la centralina non
/// li fornisce compaiono come «Non supportato», mai come zero.
const FERMO_ATTESI: [u8; 8] = [0x03, 0x0C, 0x0D, 0x05, 0x04, 0x0B, 0x06, 0x07];

/// Esegue l'intera lettura. `Err` solo se il collegamento cade.
pub fn leggi(elm: &mut Elm, ctx: &Contesto<'_>) -> Result<DiagnosisReport, Scollegato> {
    let can = elm.kind().is_can();
    let mut r = DiagnosisReport {
        letto_il: adesso_iso(),
        demo: elm.is_demo(),
        adattatore: ctx.adattatore.to_string(),
        collegamento: elm.descrizione(),
        protocollo: ctx.protocollo.to_string(),
        ecu: ctx.supportati.keys().cloned().collect(),
        pid_supportati: ctx
            .supportati
            .iter()
            .map(|(k, v)| (k.clone(), v.iter().copied().collect()))
            .collect(),
        ..Default::default()
    };
    r.tensione = match elm.tensione() {
        Ok(t) => t,
        Err(crate::elm::ElmError::Scollegato(m)) => return Err(Scollegato(m)),
        Err(_) => None,
    };

    // Spia MIL e monitor di prontezza (PID 01).
    let e = elm.interroga(ObdRequest::CurrentData(0x01))?;
    r.lettura_prontezza = Lettura::da_esito(&e);
    if let Esito::Dati(msg) = &e {
        r.prontezza = msg
            .iter()
            .filter_map(|m| readiness::decode(&m.ecu, &m.data[2..]))
            .collect();
        if r.prontezza.is_empty() {
            r.lettura_prontezza = Lettura {
                stato: Stato::ErroreComunicazione,
                dettaglio: "Risposta al PID 01 troppo corta".into(),
            };
        }
    }

    // Codici guasto nei tre stati.
    let mut dtc: BTreeMap<(String, String), BTreeSet<StatoDtc>> = BTreeMap::new();
    for (richiesta, stato) in [
        (ObdRequest::StoredDtcs, StatoDtc::Confermato),
        (ObdRequest::PendingDtcs, StatoDtc::InSospeso),
        (ObdRequest::PermanentDtcs, StatoDtc::Permanente),
    ] {
        let e = elm.interroga(richiesta)?;
        let mut lettura = Lettura::da_esito(&e);
        if let Esito::Dati(msg) = &e {
            for m in msg {
                for codice in parse_dtc_message(&m.data, elm.kind()) {
                    dtc.entry((m.ecu.clone(), codice))
                        .or_default()
                        .insert(stato);
                }
            }
        }
        // Sui protocolli non CAN «nessun codice» arriva come NO DATA.
        if !can && lettura.stato == Stato::RispostaMancante && stato != StatoDtc::Permanente {
            lettura = Lettura {
                stato: Stato::Ok,
                dettaglio: "Nessuna risposta: su questo protocollo equivale a nessun codice".into(),
            };
        }
        match stato {
            StatoDtc::Confermato => r.lettura_confermati = lettura,
            StatoDtc::InSospeso => r.lettura_in_sospeso = lettura,
            StatoDtc::Permanente => r.lettura_permanenti = lettura,
        }
    }
    r.dtc = dtc
        .into_iter()
        .map(|((ecu, codice), stati)| RigaDtc {
            descrizione: descrizione_o_nota(&codice),
            codice,
            ecu,
            stati: stati.into_iter().collect(),
        })
        .collect();
    r.dtc
        .sort_by(|a, b| a.codice.cmp(&b.codice).then(a.ecu.cmp(&b.ecu)));

    r.fermo = leggi_fermo(elm)?;
    r.mode06 = leggi_mode06(elm, can)?;

    // Tipo di carburante e norma OBD: letti una volta, solo se dichiarati.
    let dichiarato = |pid: u8| ctx.supportati.values().any(|s| s.contains(&pid));
    if dichiarato(PID_TIPO_CARBURANTE) {
        r.tipo_carburante = leggi_testo(elm, PID_TIPO_CARBURANTE, pids::tipo_carburante)?;
    }
    if dichiarato(PID_NORMA_OBD) {
        r.norma_obd = leggi_testo(elm, PID_NORMA_OBD, pids::norma_obd)?;
    }

    // VIN: su CAN un unico messaggio ricomposto, altrove più messaggi numerati.
    if let Esito::Dati(msg) = elm.interroga(ObdRequest::Vin)? {
        r.vin = if can {
            msg.first().and_then(|m| vin_da_messaggio(&m.data))
        } else {
            vin_da_messaggi_non_can(&msg)
        };
    }
    Ok(r)
}

const PID_TIPO_CARBURANTE: u8 = 0x51;
const PID_NORMA_OBD: u8 = 0x1C;

/// Legge un PID del servizio 01 che produce un testo. Nessuna risposta valida
/// dà `None`: il dato resta assente, mai un valore inventato.
fn leggi_testo(
    elm: &mut Elm,
    pid: u8,
    decodifica: fn(&[u8]) -> Option<String>,
) -> Result<Option<String>, Scollegato> {
    Ok(match elm.interroga(ObdRequest::CurrentData(pid))? {
        Esito::Dati(msg) => msg
            .first()
            .and_then(|m| m.data.get(2..))
            .and_then(decodifica),
        _ => None,
    })
}

fn vin_da_messaggio(data: &[u8]) -> Option<String> {
    // 49 02 01 + 17 caratteri ASCII.
    let ascii: String = data
        .get(3..)?
        .iter()
        .filter(|b| b.is_ascii_alphanumeric())
        .map(|b| *b as char)
        .collect();
    (ascii.len() == 17).then_some(ascii)
}

/// Fuori da CAN (linea K, J1850) il VIN arriva in più messaggi
/// `49 02 <n> <4 byte>`, con `<n>` da 1 in su e il primo riempito di zeri.
/// Vale la prima centralina che risponde.
fn vin_da_messaggi_non_can(msg: &[crate::obd::frames::EcuMessage]) -> Option<String> {
    let ecu = &msg.first()?.ecu;
    let mut pezzi: Vec<(u8, &[u8])> = msg
        .iter()
        .filter(|m| &m.ecu == ecu && m.data.len() > 3)
        .map(|m| (m.data[2], &m.data[3..]))
        .collect();
    pezzi.sort_by_key(|p| p.0);
    pezzi.dedup_by_key(|p| p.0);
    let ascii: String = pezzi
        .iter()
        .flat_map(|p| p.1.iter())
        .filter(|b| b.is_ascii_alphanumeric())
        .map(|b| *b as char)
        .collect();
    (ascii.len() == 17).then_some(ascii)
}

fn leggi_fermo(elm: &mut Elm) -> Result<FermoImmagine, Scollegato> {
    let mut f = FermoImmagine::default();
    let e = elm.interroga(ObdRequest::FreezeFrame(0x02))?;
    f.lettura = Lettura::da_esito(&e);
    let Esito::Dati(msg) = e else {
        if f.lettura.stato == Stato::RispostaMancante {
            f.lettura.dettaglio =
                "Nessuna risposta alla richiesta del fermo immagine: nessun fotogramma disponibile"
                    .into();
        }
        return Ok(f);
    };
    // Prima centralina che dichiara un fotogramma con DTC diverso da 0000.
    let trovato = msg.iter().find_map(|m| {
        let d = m.data.get(3..5)?;
        decode_dtc(d[0], d[1]).map(|c| (m.ecu.clone(), c))
    });
    let Some((ecu, codice)) = trovato else {
        f.lettura.dettaglio = "La centralina non ha un fermo immagine memorizzato".into();
        return Ok(f);
    };
    f.presente = true;
    f.dtc = Some(codice);
    f.ecu = ecu.clone();

    let della_ecu = |msg: Vec<EcuMessage>| msg.into_iter().find(|m| m.ecu == ecu);

    // PID disponibili nel fotogramma.
    let mut disponibili: BTreeSet<u8> = BTreeSet::new();
    for base in [0x00u8, 0x20, 0x40] {
        if base != 0 && !disponibili.contains(&base) {
            break;
        }
        match elm.interroga(ObdRequest::FreezeFrame(base))? {
            Esito::Dati(msg) => {
                if let Some(m) = della_ecu(msg)
                    && m.data.len() >= 7
                {
                    disponibili.extend(supported_from_mask(base, &m.data[3..]));
                }
            }
            _ => break,
        }
    }

    let mut da_leggere: Vec<u8> = FERMO_ATTESI.to_vec();
    for pid in &disponibili {
        let noto = *pid == 0x03 || pids::per_pid(*pid).next().is_some();
        if noto && !da_leggere.contains(pid) {
            da_leggere.push(*pid);
        }
    }

    for pid in da_leggere {
        let dichiarato = disponibili.contains(&pid);
        let (stato, dati) = if !dichiarato && !disponibili.is_empty() {
            (Stato::NonSupportato, None)
        } else {
            match elm.interroga(ObdRequest::FreezeFrame(pid))? {
                Esito::Dati(msg) => match della_ecu(msg) {
                    Some(m) => (Stato::Ok, Some(m.data[3..].to_vec())),
                    None => (Stato::RispostaMancante, None),
                },
                Esito::NonSupportato => (Stato::NonSupportato, None),
                Esito::Mancante if dichiarato => (Stato::RispostaMancante, None),
                Esito::Mancante => (Stato::NonSupportato, None),
                Esito::Errore(_) => (Stato::ErroreComunicazione, None),
            }
        };
        if pid == 0x03 {
            let testo = dati.as_deref().and_then(stato_carburante);
            f.righe.push(RigaFermo {
                key: KEY_STATO_CARBURANTE.into(),
                nome: NOME_STATO_CARBURANTE.into(),
                pid,
                valore: None,
                stato: if stato == Stato::Ok && testo.is_none() {
                    Stato::ErroreComunicazione
                } else {
                    stato
                },
                testo,
                unita: String::new(),
            });
            continue;
        }
        for par in pids::per_pid(pid) {
            let (valore, stato) = match (&dati, stato) {
                (Some(d), Stato::Ok) => match pids::decodifica(par, d) {
                    Decodifica::Valore(v) => (Some(v), Stato::Ok),
                    Decodifica::NonApplicabile => continue,
                    Decodifica::Corta => (None, Stato::ErroreComunicazione),
                },
                (_, s) => (None, s),
            };
            f.righe.push(RigaFermo {
                key: par.key.clone(),
                nome: par.nome.clone(),
                pid,
                valore,
                testo: None,
                unita: par.unita.to_string(),
                stato,
            });
        }
    }
    Ok(f)
}

fn leggi_mode06(elm: &mut Elm, can: bool) -> Result<SezioneMode06, Scollegato> {
    let mut s = SezioneMode06::default();
    if !can {
        s.lettura = Lettura {
            stato: Stato::NonSupportato,
            dettaglio: "Mode 06 letto solo sui protocolli CAN: sugli altri la struttura è diversa \
                        e non è verificata"
                .into(),
        };
        return Ok(s);
    }
    // MID supportati: intervallo 01–20, poi 21–40 se dichiarato.
    let mut mid: BTreeSet<u8> = BTreeSet::new();
    let e = elm.interroga(ObdRequest::MonitorResults(0x00))?;
    s.lettura = Lettura::da_esito(&e);
    let Esito::Dati(msg) = e else {
        return Ok(s);
    };
    for m in msg.iter().filter(|m| m.data.len() >= 6) {
        mid.extend(supported_from_mask(0x00, &m.data[2..]));
    }
    if mid.contains(&0x20)
        && let Esito::Dati(msg) = elm.interroga(ObdRequest::MonitorResults(0x20))?
    {
        for m in msg.iter().filter(|m| m.data.len() >= 6) {
            mid.extend(supported_from_mask(0x20, &m.data[2..]));
        }
    }
    if !mid.contains(&MID_CATALIZZATORE_B1) {
        s.lettura = Lettura {
            stato: Stato::NonSupportato,
            dettaglio: "La centralina non espone il monitor del catalizzatore (MID 21)".into(),
        };
        return Ok(s);
    }
    let e = elm.interroga(ObdRequest::MonitorResults(MID_CATALIZZATORE_B1))?;
    s.lettura = Lettura::da_esito(&e);
    if let Esito::Dati(msg) = e {
        for m in msg {
            s.test.extend(mode06::parse(&m.ecu, &m.data));
        }
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::copione::Copione;

    fn pronto(mut c: Copione) -> (Elm, BTreeMap<String, BTreeSet<u8>>) {
        c.testo("0100", "7E8 06 41 00 BE 3F A8 13\r\r>");
        let mut e = Elm::new(Box::new(c));
        e.inizializza().unwrap();
        let (_, iniziali) = e.collega_centralina().unwrap();
        let s = e.pid_supportati(&iniziali).unwrap();
        (e, s)
    }

    fn leggi_con(c: Copione) -> DiagnosisReport {
        let (mut e, s) = pronto(c);
        leggi(
            &mut e,
            &Contesto {
                adattatore: "ELM327 v1.5",
                protocollo: "CAN",
                supportati: &s,
            },
        )
        .unwrap()
    }

    #[test]
    fn p0420_in_tre_stati_una_sola_riga() {
        let mut c = Copione::con_inizializzazione(6);
        c.testo("0101", "7E8 06 41 01 81 07 65 00\r\r>");
        c.testo("03", "7E8 04 43 01 04 20\r\r>");
        c.testo("07", "7E8 04 47 01 04 20\r\r>");
        c.testo("0A", "7E8 04 4A 01 04 20\r\r>");
        let r = leggi_con(c);
        assert_eq!(r.dtc.len(), 1);
        assert_eq!(r.dtc[0].codice, "P0420");
        assert_eq!(r.dtc[0].ecu, "7E8");
        assert_eq!(
            r.dtc[0].stati,
            vec![
                StatoDtc::Confermato,
                StatoDtc::InSospeso,
                StatoDtc::Permanente
            ]
        );
        assert!(r.prontezza[0].mil_accesa);
        assert_eq!(r.lettura_confermati.stato, Stato::Ok);
    }

    #[test]
    fn dtc_multipli_da_piu_centraline_e_permanenti_non_supportati() {
        let mut c = Copione::con_inizializzazione(20);
        c.testo(
            "03",
            "7E8 10 08 43 03 04 20 01 71\r7E9 04 43 01 07 00\r7E8 21 03 00 AA AA AA AA AA\r\r>",
        );
        c.testo("07", "7E8 02 47 00\r7E9 02 47 00\r\r>");
        c.testo("0A", "7E8 03 7F 0A 11\r\r>");
        let r = leggi_con(c);
        let codici: Vec<(&str, &str)> = r
            .dtc
            .iter()
            .map(|d| (d.codice.as_str(), d.ecu.as_str()))
            .collect();
        assert_eq!(
            codici,
            vec![
                ("P0171", "7E8"),
                ("P0300", "7E8"),
                ("P0420", "7E8"),
                ("P0700", "7E9")
            ]
        );
        assert_eq!(r.lettura_in_sospeso.stato, Stato::Ok);
        assert_eq!(r.lettura_permanenti.stato, Stato::NonSupportato);
        // Prontezza non letta (NO DATA dal copione): risposta mancante, non zero.
        assert_eq!(r.lettura_prontezza.stato, Stato::RispostaMancante);
        assert!(r.prontezza.is_empty());
    }

    #[test]
    fn fermo_immagine_assente() {
        let mut c = Copione::con_inizializzazione(20);
        c.testo("020200", "7E8 05 42 02 00 00 00\r\r>");
        let r = leggi_con(c);
        assert!(!r.fermo.presente);
        assert_eq!(r.fermo.dtc, None);
        assert!(r.fermo.righe.is_empty());
        assert_eq!(r.fermo.lettura.stato, Stato::Ok);
    }

    #[test]
    fn fermo_immagine_con_dtc_diverso_e_pid_non_supportati() {
        let mut c = Copione::con_inizializzazione(20);
        // Il fotogramma appartiene a P0171, non a P0420.
        c.testo("020200", "7E8 05 42 02 00 01 71\r\r>");
        // Supportati nel fotogramma: 02, 03, 05, 0C, 0D (niente 04, 06, 07, 0B).
        c.testo("020000", "7E8 07 42 00 00 68 18 00 00\r\r>");
        c.testo("020300", "7E8 05 42 03 00 02 00\r\r>");
        c.testo("020500", "7E8 04 42 05 00 7B\r\r>");
        c.testo("020C00", "7E8 05 42 0C 00 21 40\r\r>");
        // 0D dichiarato ma senza risposta.
        let r = leggi_con(c);
        assert!(r.fermo.presente);
        assert_eq!(r.fermo.dtc.as_deref(), Some("P0171"));
        let riga = |k: &str| r.fermo.righe.iter().find(|x| x.key == k).unwrap();
        assert_eq!(riga("giri").valore, Some(2128.0));
        assert_eq!(riga("temp_refrigerante").valore, Some(83.0));
        assert!(
            riga(KEY_STATO_CARBURANTE)
                .testo
                .as_ref()
                .unwrap()
                .contains("Anello chiuso")
        );
        assert_eq!(riga("velocita").stato, Stato::RispostaMancante);
        assert_eq!(riga("velocita").valore, None);
        assert_eq!(riga("stft_b1").stato, Stato::NonSupportato);
        assert_eq!(riga("stft_b1").valore, None);
        assert_eq!(riga("map").stato, Stato::NonSupportato);
    }

    #[test]
    fn mode06_catalizzatore() {
        let mut c = Copione::con_inizializzazione(20);
        c.testo("0600", "7E8 06 46 00 00 00 00 01\r\r>");
        c.testo("0620", "7E8 06 46 20 80 00 00 00\r\r>");
        c.testo(
            "0621",
            "7E8 10 0A 46 21 80 7E 12 34\r7E8 21 00 00 FF FF AA AA AA\r\r>",
        );
        let r = leggi_con(c);
        assert_eq!(r.mode06.lettura.stato, Stato::Ok);
        assert_eq!(r.mode06.test.len(), 1);
        assert_eq!(r.mode06.test[0].grezzo_valore, 0x1234);
        // Scala 0x7E non in tabella: solo dato grezzo.
        assert!(r.mode06.test[0].scalato.is_none());
    }

    #[test]
    fn tipo_carburante_e_norma_obd_letti_solo_se_dichiarati() {
        // 0100 dichiara 1C e 20; 0120 dichiara 40; 0140 dichiara 51.
        let mut c = Copione::con_inizializzazione(20);
        c.testo("0120", "7E8 06 41 20 00 00 00 01\r\r>");
        c.testo("0140", "7E8 06 41 40 00 00 80 00\r\r>");
        c.testo("011C", "7E8 03 41 1C 06\r\r>");
        c.testo("0151", "7E8 03 41 51 04\r\r>");
        let r = leggi_con(c);
        assert_eq!(r.norma_obd.as_deref(), Some("EOBD"));
        assert_eq!(r.tipo_carburante.as_deref(), Some("Diesel"));

        // Senza risposta: assenti, non un valore di ripiego.
        let r = leggi_con(Copione::con_inizializzazione(20));
        assert_eq!(r.norma_obd, None);
        assert_eq!(r.tipo_carburante, None);
    }

    #[test]
    fn vecchia_sessione_senza_i_nuovi_campi_si_legge() {
        let r = DiagnosisReport {
            tipo_carburante: Some("Benzina".into()),
            ..Default::default()
        };
        let mut json: serde_json::Value = serde_json::to_value(&r).unwrap();
        let o = json.as_object_mut().unwrap();
        o.remove("tipo_carburante");
        o.remove("norma_obd");
        let letto: DiagnosisReport = serde_json::from_value(json).unwrap();
        assert_eq!(letto.tipo_carburante, None);
        assert_eq!(letto.norma_obd, None);
    }

    #[test]
    fn vin_letto_ma_non_serializzato() {
        let mut c = Copione::con_inizializzazione(20);
        c.testo(
            "0902",
            "7E8 10 14 49 02 01 56 46 33\r7E8 21 41 42 43 44 45 46 47\r7E8 22 48 49 4A 4B 4C 4D 4E\r\r>",
        );
        let r = leggi_con(c);
        assert_eq!(r.vin.as_deref(), Some("VF3ABCDEFGHIJKLMN"));
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("VF3ABCDEFGHIJKLMN"));
    }
}

/// Lettura completa da veicoli a linea K simulati (docs/RICERCA_K_LINE.md).
#[cfg(test)]
mod prove_linea_k {
    use super::*;
    use crate::transport::copione::{Copione, riga_iso9141, riga_kwp};

    const VIN: &str = "ZFA18800004567890";

    /// Copione di un veicolo a linea K con due codici confermati su due
    /// messaggi, nessun codice in sospeso e il VIN su cinque messaggi.
    fn veicolo(dpn: &str, ecu: u8, riga: fn(u8, &[u8]) -> String) -> Copione {
        let risposta = |righe: Vec<String>| format!("{}\r\r>", righe.join("\r"));
        let mut c = Copione::con_linea_k(9, dpn);
        c.testo(
            "0100",
            &risposta(vec![riga(ecu, &[0x41, 0x00, 0xBE, 0x3E, 0xB8, 0x11])]),
        );
        c.testo(
            "0101",
            &risposta(vec![riga(ecu, &[0x41, 0x01, 0x82, 0x07, 0x65, 0x00])]),
        );
        // Tre codici per messaggio, senza byte di conteggio; 00 00 = riempitivo.
        c.testo(
            "03",
            &risposta(vec![
                riga(ecu, &[0x43, 0x04, 0x20, 0x01, 0x71, 0x03, 0x00]),
                riga(ecu, &[0x43, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00]),
            ]),
        );
        c.testo(
            "07",
            &risposta(vec![riga(ecu, &[0x47, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])]),
        );
        // VIN: 49 02 <n> e quattro byte; il primo messaggio ha tre zeri di
        // riempimento. Messi fuori ordine apposta.
        let v = VIN.as_bytes();
        let mut pezzi = vec![riga(ecu, &[0x49, 0x02, 0x01, 0x00, 0x00, 0x00, v[0]])];
        for n in 0..4usize {
            let p = &v[1 + n * 4..5 + n * 4];
            pezzi.push(riga(
                ecu,
                &[0x49, 0x02, n as u8 + 2, p[0], p[1], p[2], p[3]],
            ));
        }
        pezzi.swap(1, 3);
        c.testo("0902", &risposta(pezzi));
        c
    }

    fn leggi_da(c: Copione, protocollo: &str) -> DiagnosisReport {
        let mut e = Elm::new(Box::new(c));
        e.inizializza().unwrap();
        let (p, iniziali) = e.collega_centralina().unwrap();
        assert!(p.linea_k());
        let s = e.pid_supportati(&iniziali).unwrap();
        leggi(
            &mut e,
            &Contesto {
                adattatore: "ELM327 v1.5",
                protocollo,
                supportati: &s,
            },
        )
        .unwrap()
    }

    fn verifica(r: &DiagnosisReport, ecu: &str) {
        let codici: Vec<&str> = r.dtc.iter().map(|d| d.codice.as_str()).collect();
        assert_eq!(codici, vec!["P0171", "P0300", "P0420", "P0700"]);
        assert!(r.dtc.iter().all(|d| d.ecu == ecu));
        assert!(r.dtc.iter().all(|d| d.stati == vec![StatoDtc::Confermato]));
        assert_eq!(r.lettura_confermati.stato, Stato::Ok);
        assert_eq!(r.lettura_in_sospeso.stato, Stato::Ok);
        assert!(r.prontezza[0].mil_accesa);
        assert_eq!(r.vin.as_deref(), Some(VIN));
        // Il VIN resta fuori dal rapporto salvato, come su CAN.
        assert!(!serde_json::to_string(r).unwrap().contains(VIN));
    }

    #[test]
    fn lettura_completa_su_iso_9141() {
        let r = leggi_da(veicolo("A3", 0x10, riga_iso9141), "ISO 9141-2");
        verifica(&r, "10");
    }

    #[test]
    fn lettura_completa_su_kwp2000() {
        let r = leggi_da(veicolo("A5", 0x11, riga_kwp), "ISO 14230-4 KWP");
        verifica(&r, "11");
    }

    #[test]
    fn vin_incompleto_non_viene_inventato() {
        let mut c = veicolo("A3", 0x10, riga_iso9141);
        c.testo(
            "0902",
            &format!(
                "{}\r\r>",
                riga_iso9141(0x10, &[0x49, 0x02, 0x01, 0x00, 0x00, 0x00, b'Z'])
            ),
        );
        let r = leggi_da(c, "ISO 9141-2");
        assert_eq!(r.vin, None);
    }
}
