//! Codici guasto (DTC): decodifica e descrizioni (italiane curate e archivio inglese completo).

use std::sync::OnceLock;

use super::frames::BusKind;

/// Decodifica due byte in un codice tipo `P0420`. `0000` significa «nessun codice».
pub fn decode_dtc(a: u8, b: u8) -> Option<String> {
    if a == 0 && b == 0 {
        return None;
    }
    let lettera = ['P', 'C', 'B', 'U'][(a >> 6) as usize];
    Some(format!(
        "{}{}{:X}{:02X}",
        lettera,
        (a >> 4) & 0x03,
        a & 0x0F,
        b
    ))
}

/// Estrae i codici da un messaggio dei servizi 03, 07 o 0A.
/// Su CAN il secondo byte è il numero di codici; sugli altri protocolli no.
pub fn parse_dtc_message(data: &[u8], kind: BusKind) -> Vec<String> {
    let (inizio, massimo) = if kind.is_can() {
        (2, data.get(1).map(|n| *n as usize))
    } else {
        (1, None)
    };
    let mut out = Vec::new();
    if data.len() <= inizio {
        return out;
    }
    for coppia in data[inizio..].chunks_exact(2) {
        if let Some(max) = massimo
            && out.len() >= max
        {
            break;
        }
        if let Some(c) = decode_dtc(coppia[0], coppia[1]) {
            out.push(c);
        }
    }
    out
}

/// Descrizione italiana dei codici generici più comuni; `None` se non in elenco.
pub fn descrizione(codice: &str) -> Option<&'static str> {
    Some(match codice {
        "P0010" => "Attuatore variatore di fase aspirazione, bancata 1: circuito",
        "P0011" => "Fasatura albero a camme aspirazione, bancata 1: anticipo eccessivo",
        "P0030" => "Riscaldatore sonda lambda B1S1: circuito di comando",
        "P0036" => "Riscaldatore sonda lambda B1S2: circuito di comando",
        "P0100" => "Misuratore massa/portata aria: circuito",
        "P0101" => "Misuratore massa/portata aria: valore non plausibile",
        "P0105" => "Sensore pressione collettore/barometrica: circuito",
        "P0106" => "Sensore pressione collettore: valore non plausibile",
        "P0110" => "Sensore temperatura aria aspirata: circuito",
        "P0115" => "Sensore temperatura refrigerante: circuito",
        "P0116" => "Sensore temperatura refrigerante: valore non plausibile",
        "P0120" => "Sensore posizione farfalla/pedale A: circuito",
        "P0125" => "Temperatura refrigerante insufficiente per la regolazione in anello chiuso",
        "P0128" => "Termostato: temperatura refrigerante sotto la soglia di regolazione",
        "P0130" => "Sonda lambda B1S1: circuito",
        "P0131" => "Sonda lambda B1S1: tensione bassa",
        "P0132" => "Sonda lambda B1S1: tensione alta",
        "P0133" => "Sonda lambda B1S1: risposta lenta",
        "P0134" => "Sonda lambda B1S1: nessuna attività rilevata",
        "P0135" => "Sonda lambda B1S1: circuito del riscaldatore",
        "P0136" => "Sonda lambda B1S2: circuito",
        "P0137" => "Sonda lambda B1S2: tensione bassa",
        "P0138" => "Sonda lambda B1S2: tensione alta",
        "P0139" => "Sonda lambda B1S2: risposta lenta",
        "P0140" => "Sonda lambda B1S2: nessuna attività rilevata",
        "P0141" => "Sonda lambda B1S2: circuito del riscaldatore",
        "P0170" => "Correzione carburante, bancata 1: anomalia",
        "P0171" => "Miscela troppo magra, bancata 1",
        "P0172" => "Miscela troppo ricca, bancata 1",
        "P0300" => "Mancate accensioni casuali o su più cilindri",
        "P0301" => "Mancate accensioni, cilindro 1",
        "P0302" => "Mancate accensioni, cilindro 2",
        "P0303" => "Mancate accensioni, cilindro 3",
        "P0304" => "Mancate accensioni, cilindro 4",
        "P0325" => "Sensore di detonazione 1: circuito",
        "P0335" => "Sensore posizione albero motore: circuito",
        "P0340" => "Sensore posizione albero a camme: circuito",
        "P0400" => "Ricircolo gas di scarico (EGR): portata anomala",
        "P0401" => "Ricircolo gas di scarico (EGR): portata insufficiente",
        "P0420" => "Efficienza del catalizzatore sotto la soglia, bancata 1",
        "P0421" => "Efficienza del catalizzatore in riscaldamento sotto la soglia, bancata 1",
        "P0430" => "Efficienza del catalizzatore sotto la soglia, bancata 2",
        "P0440" => "Sistema di controllo evaporazione: anomalia",
        "P0441" => "Sistema di controllo evaporazione: flusso di spurgo non corretto",
        "P0442" => "Sistema di controllo evaporazione: piccola perdita rilevata",
        "P0443" => "Elettrovalvola di spurgo canister: circuito",
        "P0455" => "Sistema di controllo evaporazione: grande perdita rilevata",
        "P0500" => "Sensore velocità veicolo: anomalia",
        "P0505" => "Regolazione del minimo: anomalia",
        "P0506" => "Regime del minimo inferiore al previsto",
        "P0507" => "Regime del minimo superiore al previsto",
        "P0560" => "Tensione di sistema: anomalia",
        "P0562" => "Tensione di sistema bassa",
        "P0563" => "Tensione di sistema alta",
        "P0600" => "Collegamento di comunicazione seriale: anomalia",
        "P0606" => "Centralina: errore del processore",
        _ => return None,
    })
}

/// Elenco dei codici generici (`CODICE<TAB>titolo inglese`), incorporato nel programma.
/// Origine: OBDex, licenza CC0-1.0; si rigenera con `strumenti/genera_dtc.py`.
const DATI_GENERICI: &str = include_str!("../../data/dtc_generici.tsv");

/// Elenco ordinato per codice, costruito una sola volta.
fn elenco() -> &'static [(&'static str, &'static str)] {
    static ELENCO: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    ELENCO.get_or_init(|| {
        let mut v: Vec<(&'static str, &'static str)> = DATI_GENERICI
            .lines()
            .filter_map(|riga| riga.split_once('\t'))
            .collect();
        v.sort_unstable_by_key(|(codice, _)| *codice);
        v
    })
}

/// Titolo inglese del codice nell'elenco incorporato, se presente.
fn titolo_inglese(codice: &str) -> Option<&'static str> {
    let e = elenco();
    e.binary_search_by(|(c, _)| (*c).cmp(codice))
        .ok()
        .map(|i| e[i].1)
}

/// Traduzione italiana degli stessi titoli (`CODICE<TAB>titolo italiano`).
/// È una traduzione automatica eseguita con un modello linguistico e un
/// glossario di officina, non rivista da un tecnico: per questo il titolo
/// inglese originale resta sempre disponibile accanto.
const DATI_ITALIANI: &str = include_str!("../../data/dtc_generici_it.tsv");

fn elenco_italiano() -> &'static [(&'static str, &'static str)] {
    static ELENCO: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    ELENCO.get_or_init(|| {
        let mut v: Vec<(&'static str, &'static str)> = DATI_ITALIANI
            .lines()
            .filter_map(|riga| riga.split_once('\t'))
            .collect();
        v.sort_unstable_by_key(|(codice, _)| *codice);
        v
    })
}

/// Titolo italiano tradotto automaticamente, se presente.
fn titolo_italiano(codice: &str) -> Option<&'static str> {
    let e = elenco_italiano();
    e.binary_search_by(|(c, _)| (*c).cmp(codice))
        .ok()
        .map(|i| e[i].1)
}

/// Numero di codici presenti nell'elenco incorporato.
pub fn numero_codici() -> usize {
    elenco().len()
}

/// Indica se il codice è generico (uguale per tutte le marche) secondo SAE J2012.
///
/// Regola, sul secondo carattere del codice:
/// - `P`: generici `P0xxx`, `P2xxx` e `P34xx`-`P39xx`; del costruttore `P1xxx` e `P30xx`-`P33xx`;
/// - `B`, `C`, `U`: generici `0xxx` e `3xxx` (i `B3`/`C3` sono «riservati», trattati come generici);
///   del costruttore `1xxx` e `2xxx`.
///
/// Un codice con forma non valida non è considerato generico.
pub fn e_generico(codice: &str) -> bool {
    let b = codice.as_bytes();
    if b.len() != 5 {
        return false;
    }
    match (b[0], b[1]) {
        (b'P', b'0') | (b'P', b'2') => true,
        (b'P', b'3') => matches!(b[2], b'4'..=b'9'),
        (b'B' | b'C' | b'U', b'0') | (b'B' | b'C' | b'U', b'3') => true,
        _ => false,
    }
}

/// Lingua in cui è scritta una descrizione.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinguaDescrizione {
    Italiano,
    Inglese,
}

/// Descrizione di un codice guasto, con l'indicazione della sua provenienza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescrizioneDtc {
    /// Testo da mostrare.
    pub testo: String,
    /// Lingua del testo; `None` se è solo una nota (codice sconosciuto).
    pub lingua: Option<LinguaDescrizione>,
    /// `true` se il codice è generico (non specifico del costruttore).
    pub generico: bool,
    /// Titolo inglese originale dell'elenco, quando esiste.
    pub originale: Option<&'static str>,
    /// `true` se il testo italiano è una traduzione automatica non rivista.
    pub traduzione_automatica: bool,
}

/// Descrizione di un codice: italiana curata se esiste, altrimenti la
/// traduzione automatica, altrimenti il titolo inglese, altrimenti una nota.
pub fn descrivi(codice: &str) -> DescrizioneDtc {
    let generico = e_generico(codice);
    let originale = titolo_inglese(codice);
    if let Some(d) = descrizione(codice) {
        return DescrizioneDtc {
            testo: d.to_string(),
            lingua: Some(LinguaDescrizione::Italiano),
            generico,
            originale,
            traduzione_automatica: false,
        };
    }
    if let Some(t) = titolo_italiano(codice) {
        return DescrizioneDtc {
            testo: t.to_string(),
            lingua: Some(LinguaDescrizione::Italiano),
            generico,
            originale,
            traduzione_automatica: true,
        };
    }
    if let Some(t) = originale {
        return DescrizioneDtc {
            testo: t.to_string(),
            lingua: Some(LinguaDescrizione::Inglese),
            generico,
            originale,
            traduzione_automatica: false,
        };
    }
    let testo = if generico {
        "Codice non presente nell'elenco"
    } else {
        "Codice specifico del costruttore: significato non incluso (varia secondo la marca)"
    };
    DescrizioneDtc {
        testo: testo.to_string(),
        lingua: None,
        generico,
        originale: None,
        traduzione_automatica: false,
    }
}

/// Descrizione da mostrare sempre, anche per i codici non in elenco.
pub fn descrizione_o_nota(codice: &str) -> String {
    let d = descrivi(codice);
    match d.lingua {
        Some(LinguaDescrizione::Inglese) => format!("{} (descrizione in inglese)", d.testo),
        _ => d.testo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codici() {
        assert_eq!(decode_dtc(0x04, 0x20).as_deref(), Some("P0420"));
        assert_eq!(decode_dtc(0x01, 0x71).as_deref(), Some("P0171"));
        assert_eq!(decode_dtc(0xC1, 0x00).as_deref(), Some("U0100"));
        assert_eq!(decode_dtc(0x44, 0x1A).as_deref(), Some("C041A"));
        assert_eq!(decode_dtc(0x9A, 0x12).as_deref(), Some("B1A12"));
        assert_eq!(decode_dtc(0, 0), None);
    }

    #[test]
    fn dtc_multipli_can() {
        let d = [0x43, 0x03, 0x04, 0x20, 0x01, 0x71, 0x03, 0x00];
        assert_eq!(
            parse_dtc_message(&d, BusKind::Can11),
            vec!["P0420", "P0171", "P0300"]
        );
    }

    #[test]
    fn nessun_dtc_can() {
        assert!(parse_dtc_message(&[0x43, 0x00], BusKind::Can11).is_empty());
        // Riempimento dopo il conteggio: non deve produrre codici.
        assert!(parse_dtc_message(&[0x43, 0x00, 0xAA, 0xAA], BusKind::Can11).is_empty());
    }

    #[test]
    fn dtc_non_can() {
        let d = [0x43, 0x04, 0x20, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(parse_dtc_message(&d, BusKind::Legacy), vec!["P0420"]);
    }

    #[test]
    fn descrizioni() {
        assert!(descrizione("P0420").unwrap().contains("catalizzatore"));
        assert!(descrizione_o_nota("P1336").contains("costruttore"));
    }

    #[test]
    fn descrivi_italiano_curato() {
        let d = descrivi("P0420");
        assert_eq!(d.lingua, Some(LinguaDescrizione::Italiano));
        assert!(d.testo.contains("catalizzatore"));
        assert!(d.generico);
    }

    #[test]
    fn descrivi_solo_in_archivio() {
        assert!(descrizione("P0C73").is_none());
        let d = descrivi("P0C73");
        // Nessuna descrizione curata: vale la traduzione automatica, con
        // l'originale inglese sempre accanto.
        assert_eq!(d.lingua, Some(LinguaDescrizione::Italiano));
        assert!(d.traduzione_automatica);
        assert_eq!(
            d.originale,
            Some("Motor Electronics Coolant Pump A Control Performance")
        );
        assert_ne!(Some(d.testo.as_str()), d.originale);
        assert_eq!(descrizione_o_nota("P0C73"), d.testo);
        // Le descrizioni curate non sono marcate come automatiche.
        assert!(!descrivi("P0420").traduzione_automatica);
        assert!(descrivi("P0420").originale.is_some());
    }

    #[test]
    fn traduzione_allineata_all_originale() {
        // Stessi codici, nello stesso ordine, e nessun titolo vuoto o rimasto
        // identico all'inglese.
        let en: Vec<(&str, &str)> = DATI_GENERICI
            .lines()
            .filter_map(|r| r.split_once('\t'))
            .collect();
        let it: Vec<(&str, &str)> = DATI_ITALIANI
            .lines()
            .filter_map(|r| r.split_once('\t'))
            .collect();
        assert_eq!(en.len(), it.len());
        for ((ce, te), (ci, ti)) in en.iter().zip(it.iter()) {
            assert_eq!(ce, ci);
            assert!(!ti.trim().is_empty(), "{ci}");
            assert!(!ti.contains('\t') && !ti.contains('"'), "{ci}");
            assert_ne!(te, ti, "{ci} non tradotto");
        }
    }

    #[test]
    fn descrivi_costruttore_e_sconosciuto() {
        let d = descrivi("P1336");
        assert!(!d.generico);
        assert_eq!(d.lingua, None);
        assert!(d.testo.starts_with("Codice specifico del costruttore"));
        let n = descrivi("P0FFE");
        assert!(n.generico);
        assert_eq!(n.lingua, None);
        assert_eq!(n.testo, "Codice non presente nell'elenco");
    }

    #[test]
    fn classificazione_generico() {
        for c in [
            "P0000", "P0420", "P2000", "P3400", "P3999", "B0001", "C0300", "U0100", "B3000",
            "C3000", "U3000",
        ] {
            assert!(e_generico(c), "{c}");
        }
        for c in [
            "P1000", "P1336", "P3000", "P3100", "P3399", "B1000", "B2000", "C1000", "C2000",
            "U1000", "U2000",
        ] {
            assert!(!e_generico(c), "{c}");
        }
        assert!(!e_generico("P04"));
        assert!(!e_generico(""));
    }

    #[test]
    fn archivio_completo_e_valido() {
        assert!(numero_codici() > 9000);
        let mut n = 0;
        for riga in DATI_GENERICI.lines() {
            let (codice, titolo) = riga.split_once('\t').expect(riga);
            let b = codice.as_bytes();
            assert!(
                b.len() == 5
                    && b"PCBU".contains(&b[0])
                    && (b'0'..=b'3').contains(&b[1])
                    && b[2..]
                        .iter()
                        .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(c)),
                "codice non valido: {codice}"
            );
            assert!(!titolo.trim().is_empty(), "titolo vuoto: {codice}");
            n += 1;
        }
        assert_eq!(n, numero_codici());
    }
}
