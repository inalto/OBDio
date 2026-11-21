//! Codici guasto (DTC): decodifica e descrizioni dei codici generici più comuni.

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

/// Descrizione da mostrare sempre, anche per i codici non in elenco.
pub fn descrizione_o_nota(codice: &str) -> String {
    if let Some(d) = descrizione(codice) {
        return d.to_string();
    }
    let costruttore = matches!(codice.as_bytes().get(1), Some(b'1') | Some(b'3'));
    if costruttore {
        "Codice specifico del costruttore: descrizione non inclusa".to_string()
    } else {
        "Codice generico: descrizione non inclusa nell'elenco dell'app".to_string()
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
        assert!(descrizione_o_nota("P0999").contains("generico"));
    }
}
