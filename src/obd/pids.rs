//! Catalogo dei parametri dei servizi 01/02 e relative formule (SAE J1979).
//!
//! Ogni parametro ha una chiave stabile usata nei file. Un PID può produrre
//! più parametri (per esempio lambda e tensione della stessa sonda).

use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Formula di decodifica; i byte sono quelli che seguono il PID.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A·100/255 %
    Percento,
    /// A − 40 °C
    Temperatura,
    /// (A − 128)·100/128 %
    Correzione,
    /// A, senza conversione
    Byte,
    /// (256A + B)/4 giri/min
    Giri,
    /// A/2 − 64 °
    Anticipo,
    /// (256A + B)/100 g/s
    Maf,
    /// A/200 V (sonda a banda stretta)
    O2Tensione,
    /// (B − 128)·100/128 %; B = FF: sonda non usata per la correzione
    O2Correzione,
    /// (256A + B)·2/65536 (rapporto di equivalenza λ)
    Lambda,
    /// (256C + D)·8/65536 V
    LambdaTensione,
    /// (256C + D)/256 − 128 mA
    LambdaCorrente,
    /// 256A + B
    Parola,
    /// (256A + B)/10 − 40 °C
    TemperaturaCat,
    /// (256A + B)/1000 V
    Millivolt,
    /// (256A + B)·100/255 %
    CaricoAssoluto,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub key: String,
    pub pid: u8,
    pub nome: String,
    pub unita: &'static str,
    pub kind: Kind,
    pub decimali: usize,
    /// Mostrare sempre il segno (correzioni carburante).
    pub con_segno: bool,
}

fn p(key: &str, pid: u8, nome: &str, unita: &'static str, kind: Kind, decimali: usize) -> Param {
    Param {
        key: key.to_string(),
        pid,
        nome: nome.to_string(),
        unita,
        kind,
        decimali,
        con_segno: matches!(kind, Kind::Correzione | Kind::O2Correzione),
    }
}

fn nome_sonda(n: u8) -> String {
    match n {
        1 => "Sonda B1S1 (a monte)".to_string(),
        2 => "Sonda B1S2 (a valle)".to_string(),
        _ => format!("Sonda O2 n. {n}"),
    }
}

fn costruisci() -> Vec<Param> {
    use Kind::*;
    let mut v = vec![
        p("carico", 0x04, "Carico motore calcolato", "%", Percento, 1),
        p(
            "temp_refrigerante",
            0x05,
            "Temperatura refrigerante",
            "°C",
            Temperatura,
            0,
        ),
        p("stft_b1", 0x06, "STFT bancata 1", "%", Correzione, 2),
        p("ltft_b1", 0x07, "LTFT bancata 1", "%", Correzione, 2),
        p("stft_b2", 0x08, "STFT bancata 2", "%", Correzione, 2),
        p("ltft_b2", 0x09, "LTFT bancata 2", "%", Correzione, 2),
        p("map", 0x0B, "Pressione collettore", "kPa", Byte, 0),
        p("giri", 0x0C, "Giri motore", "giri/min", Giri, 0),
        p("velocita", 0x0D, "Velocità", "km/h", Byte, 0),
        p("anticipo", 0x0E, "Anticipo di accensione", "°", Anticipo, 1),
        p(
            "temp_aspirazione",
            0x0F,
            "Temperatura aria aspirata",
            "°C",
            Temperatura,
            0,
        ),
        p("maf", 0x10, "Portata aria (MAF)", "g/s", Maf, 2),
        p("farfalla", 0x11, "Posizione farfalla", "%", Percento, 1),
        p(
            "tempo_motore",
            0x1F,
            "Tempo dall'avviamento",
            "s",
            Parola,
            0,
        ),
        p(
            "km_con_mil",
            0x21,
            "Distanza con MIL accesa",
            "km",
            Parola,
            0,
        ),
        p(
            "livello_carburante",
            0x2F,
            "Livello carburante",
            "%",
            Percento,
            1,
        ),
        p(
            "km_da_cancellazione",
            0x31,
            "Distanza dalla cancellazione DTC",
            "km",
            Parola,
            0,
        ),
        p(
            "pressione_barometrica",
            0x33,
            "Pressione barometrica",
            "kPa",
            Byte,
            0,
        ),
        p(
            "temp_cat_b1s1",
            0x3C,
            "Temperatura catalizzatore B1S1",
            "°C",
            TemperaturaCat,
            1,
        ),
        p(
            "temp_cat_b2s1",
            0x3D,
            "Temperatura catalizzatore B2S1",
            "°C",
            TemperaturaCat,
            1,
        ),
        p(
            "temp_cat_b1s2",
            0x3E,
            "Temperatura catalizzatore B1S2",
            "°C",
            TemperaturaCat,
            1,
        ),
        p(
            "temp_cat_b2s2",
            0x3F,
            "Temperatura catalizzatore B2S2",
            "°C",
            TemperaturaCat,
            1,
        ),
        p(
            "tensione_modulo",
            0x42,
            "Tensione modulo di controllo",
            "V",
            Millivolt,
            2,
        ),
        p(
            "carico_assoluto",
            0x43,
            "Carico assoluto",
            "%",
            CaricoAssoluto,
            1,
        ),
        p("lambda_comandato", 0x44, "Lambda comandato", "λ", Lambda, 3),
        p(
            "temp_ambiente",
            0x46,
            "Temperatura ambiente",
            "°C",
            Temperatura,
            0,
        ),
        p("temp_olio", 0x5C, "Temperatura olio", "°C", Temperatura, 0),
    ];
    for n in 1..=8u8 {
        let s = nome_sonda(n);
        v.push(p(
            &format!("o2s{n}_tensione"),
            0x13 + n,
            &format!("{s} — tensione"),
            "V",
            O2Tensione,
            3,
        ));
        v.push(p(
            &format!("o2s{n}_stft"),
            0x13 + n,
            &format!("{s} — correzione associata"),
            "%",
            O2Correzione,
            2,
        ));
        v.push(p(
            &format!("o2s{n}_lambda"),
            0x23 + n,
            &format!("{s} — lambda"),
            "λ",
            Lambda,
            3,
        ));
        v.push(p(
            &format!("o2s{n}_tensione_wr"),
            0x23 + n,
            &format!("{s} — tensione (banda larga)"),
            "V",
            LambdaTensione,
            3,
        ));
        v.push(p(
            &format!("o2s{n}_lambda_i"),
            0x33 + n,
            &format!("{s} — lambda"),
            "λ",
            Lambda,
            3,
        ));
        v.push(p(
            &format!("o2s{n}_corrente"),
            0x33 + n,
            &format!("{s} — corrente"),
            "mA",
            LambdaCorrente,
            3,
        ));
    }
    v.sort_by_key(|x| x.pid);
    v
}

/// Tutti i parametri numerici conosciuti, ordinati per PID.
pub fn catalogo() -> &'static [Param] {
    static C: OnceLock<Vec<Param>> = OnceLock::new();
    C.get_or_init(costruisci)
}

pub fn per_pid(pid: u8) -> impl Iterator<Item = &'static Param> {
    catalogo().iter().filter(move |x| x.pid == pid)
}

pub fn per_key(key: &str) -> Option<&'static Param> {
    catalogo().iter().find(|x| x.key == key)
}

/// Esito della decodifica di un parametro.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decodifica {
    Valore(f64),
    /// La centralina segnala che il dato non è applicabile (es. byte FF).
    NonApplicabile,
    /// Risposta più corta del previsto.
    Corta,
}

fn parola(d: &[u8], i: usize) -> Option<f64> {
    Some(((*d.get(i)? as u32) << 8 | *d.get(i + 1)? as u32) as f64)
}

/// Applica la formula del parametro ai byte dati (dopo il PID).
pub fn decodifica(par: &Param, d: &[u8]) -> Decodifica {
    use Decodifica::*;
    let a = d.first().map(|x| *x as f64);
    let r = match par.kind {
        Kind::Percento => a.map(|a| a * 100.0 / 255.0),
        Kind::Temperatura => a.map(|a| a - 40.0),
        Kind::Correzione => a.map(|a| (a - 128.0) * 100.0 / 128.0),
        Kind::Byte => a,
        Kind::Giri => parola(d, 0).map(|w| w / 4.0),
        Kind::Anticipo => a.map(|a| a / 2.0 - 64.0),
        Kind::Maf => parola(d, 0).map(|w| w / 100.0),
        Kind::O2Tensione => a.map(|a| a / 200.0),
        Kind::O2Correzione => match d.get(1) {
            Some(0xFF) => return NonApplicabile,
            Some(b) => Some((*b as f64 - 128.0) * 100.0 / 128.0),
            None => None,
        },
        Kind::Lambda => parola(d, 0).map(|w| w * 2.0 / 65536.0),
        Kind::LambdaTensione => parola(d, 2).map(|w| w * 8.0 / 65536.0),
        Kind::LambdaCorrente => parola(d, 2).map(|w| w / 256.0 - 128.0),
        Kind::Parola => parola(d, 0),
        Kind::TemperaturaCat => parola(d, 0).map(|w| w / 10.0 - 40.0),
        Kind::Millivolt => parola(d, 0).map(|w| w / 1000.0),
        Kind::CaricoAssoluto => parola(d, 0).map(|w| w * 100.0 / 255.0),
    };
    match r {
        Some(v) => Valore(v),
        None => Corta,
    }
}

/// PID 03: stato del sistema carburante (primo sistema).
pub fn stato_carburante(d: &[u8]) -> Option<String> {
    let a = *d.first()?;
    Some(
        match a {
            0x00 => "Non riportato",
            0x01 => "Anello aperto: temperatura motore insufficiente",
            0x02 => "Anello chiuso: regolazione con sonda lambda",
            0x04 => "Anello aperto: carico motore o taglio in rilascio",
            0x08 => "Anello aperto: guasto del sistema",
            0x10 => "Anello chiuso con almeno una sonda in anomalia",
            altro => return Some(format!("Valore non standard (0x{altro:02X})")),
        }
        .to_string(),
    )
}

/// Chiave e nome del parametro testuale del PID 03.
pub const KEY_STATO_CARBURANTE: &str = "stato_carburante";
pub const NOME_STATO_CARBURANTE: &str = "Stato sistema carburante";

/// Selezione «Diagnosi P0420» calcolata sui PID realmente supportati.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresetP0420 {
    /// Segnale della sonda a monte scelto, se la centralina ne espone uno.
    pub b1s1: Option<String>,
    /// Segnale della sonda a valle scelto, se la centralina ne espone uno.
    pub b1s2: Option<String>,
    /// Parametri sempre inclusi: (chiave, supportato).
    pub base: Vec<(String, bool)>,
    /// Parametri selezionabili: (chiave, supportato).
    pub opzionali: Vec<(String, bool)>,
}

impl PresetP0420 {
    pub fn calcola(supportati: &BTreeSet<u8>) -> Self {
        let primo = |candidati: &[(&str, u8)]| {
            candidati
                .iter()
                .find(|(_, pid)| supportati.contains(pid))
                .map(|(k, _)| k.to_string())
        };
        let voce = |k: &str| {
            let pid = per_key(k).map(|x| x.pid).unwrap_or(0);
            (k.to_string(), supportati.contains(&pid))
        };
        PresetP0420 {
            // A monte: lambda se esposto, altrimenti tensione.
            b1s1: primo(&[
                ("o2s1_lambda", 0x24),
                ("o2s1_lambda_i", 0x34),
                ("o2s1_tensione", 0x14),
            ]),
            // A valle: tensione se esposta, altrimenti il segnale disponibile.
            b1s2: primo(&[
                ("o2s2_tensione", 0x15),
                ("o2s2_tensione_wr", 0x25),
                ("o2s2_lambda_i", 0x35),
            ]),
            base: [
                "stft_b1",
                "ltft_b1",
                "giri",
                "velocita",
                "temp_refrigerante",
            ]
            .iter()
            .map(|k| voce(k))
            .collect(),
            opzionali: ["carico", "map"].iter().map(|k| voce(k)).collect(),
        }
    }

    /// Chiavi delle sonde, a cui il polling dà priorità.
    pub fn sonde(&self) -> Vec<String> {
        self.b1s1.iter().chain(self.b1s2.iter()).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(key: &str, d: &[u8]) -> f64 {
        match decodifica(per_key(key).unwrap(), d) {
            Decodifica::Valore(v) => v,
            altro => panic!("{key}: {altro:?}"),
        }
    }

    #[test]
    fn fermo_immagine_di_riferimento() {
        // Valori del caso demo: 2.128 giri/min, 69 km/h, 83 °C, +0,78 %, +8,59 %.
        assert_eq!(val("giri", &[0x21, 0x40]), 2128.0);
        assert_eq!(val("velocita", &[0x45]), 69.0);
        assert_eq!(val("temp_refrigerante", &[0x7B]), 83.0);
        assert!((val("stft_b1", &[0x81]) - 0.78125).abs() < 1e-9);
        assert!((val("ltft_b1", &[0x8B]) - 8.59375).abs() < 1e-9);
    }

    #[test]
    fn sonde() {
        assert!((val("o2s2_tensione", &[0x5A, 0xFF]) - 0.45).abs() < 1e-9);
        assert_eq!(
            decodifica(per_key("o2s2_stft").unwrap(), &[0x5A, 0xFF]),
            Decodifica::NonApplicabile
        );
        assert!((val("o2s1_lambda", &[0x80, 0x00, 0x40, 0x00]) - 1.0).abs() < 1e-9);
        assert!((val("o2s1_tensione_wr", &[0x80, 0x00, 0x40, 0x00]) - 2.0).abs() < 1e-9);
        assert!((val("o2s1_corrente", &[0x80, 0x00, 0x80, 0x00])).abs() < 1e-9);
        assert_eq!(
            decodifica(per_key("o2s1_lambda").unwrap(), &[0x80]),
            Decodifica::Corta
        );
    }

    #[test]
    fn altri() {
        assert!((val("carico", &[0xFF]) - 100.0).abs() < 1e-9);
        assert_eq!(val("map", &[0x2D]), 45.0);
        assert_eq!(val("anticipo", &[0x90]), 8.0);
        assert!((val("tensione_modulo", &[0x36, 0xB0]) - 14.0).abs() < 1e-9);
        assert_eq!(
            stato_carburante(&[0x02, 0x00]).unwrap(),
            "Anello chiuso: regolazione con sonda lambda"
        );
    }

    #[test]
    fn chiavi_uniche() {
        let mut viste = BTreeSet::new();
        for x in catalogo() {
            assert!(viste.insert(x.key.clone()), "chiave duplicata {}", x.key);
        }
    }

    #[test]
    fn preset_banda_larga_a_monte() {
        let s: BTreeSet<u8> = [0x05, 0x06, 0x07, 0x0B, 0x0C, 0x0D, 0x15, 0x24]
            .into_iter()
            .collect();
        let pr = PresetP0420::calcola(&s);
        assert_eq!(pr.b1s1.as_deref(), Some("o2s1_lambda"));
        assert_eq!(pr.b1s2.as_deref(), Some("o2s2_tensione"));
        assert!(pr.base.iter().all(|(_, ok)| *ok));
        assert_eq!(
            pr.opzionali,
            vec![("carico".to_string(), false), ("map".to_string(), true)]
        );
    }

    #[test]
    fn preset_banda_stretta_e_assenze() {
        let s: BTreeSet<u8> = [0x0C, 0x14].into_iter().collect();
        let pr = PresetP0420::calcola(&s);
        assert_eq!(pr.b1s1.as_deref(), Some("o2s1_tensione"));
        assert_eq!(pr.b1s2, None);
        assert!(!pr.base.iter().find(|(k, _)| k == "stft_b1").unwrap().1);
    }
}
