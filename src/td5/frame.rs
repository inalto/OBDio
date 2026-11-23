//! Messaggi della centralina Td5 dopo l'inizializzazione: senza intestazione,
//! `[lunghezza] [servizio] [dati…] [checksum]`, dove la lunghezza conta i
//! byte fra sé e il checksum e il checksum è la somma a 8 bit di tutti i byte
//! che lo precedono (ricerca/td5/proto.md, «Message Format»).
//!
//! Qui si chiama «corpo» la parte fra lunghezza e checksum: servizio e dati.

/// Somma a 8 bit di tutti i byte.
pub fn checksum(byte: &[u8]) -> u8 {
    byte.iter().fold(0u8, |s, b| s.wrapping_add(*b))
}

/// Messaggio completo dal corpo (servizio e dati): lunghezza in testa,
/// checksum in coda. Il corpo è sempre breve (al più qualche byte): una
/// lunghezza oltre 255 non può capitare con le richieste dell'elenco chiuso.
pub fn componi(corpo: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(corpo.len() + 2);
    v.push(corpo.len().min(0xFF) as u8);
    v.extend_from_slice(corpo);
    v.push(checksum(&v));
    v
}

/// Perché un messaggio non è valido.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErroreTrama {
    /// Meno di tre byte (lunghezza, servizio, checksum).
    Corta,
    /// La lunghezza dichiarata non corrisponde ai byte ricevuti.
    Lunghezza { dichiarata: u8, presenti: usize },
    /// Checksum diverso dalla somma dei byte.
    Checksum { atteso: u8, ricevuto: u8 },
}

impl std::fmt::Display for ErroreTrama {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErroreTrama::Corta => write!(f, "messaggio troppo corto"),
            ErroreTrama::Lunghezza {
                dichiarata,
                presenti,
            } => write!(
                f,
                "lunghezza dichiarata {dichiarata}, byte presenti {presenti}"
            ),
            ErroreTrama::Checksum { atteso, ricevuto } => {
                write!(f, "checksum {ricevuto:02X} invece di {atteso:02X}")
            }
        }
    }
}

/// Verifica un messaggio completo e ne restituisce il corpo.
pub fn verifica(trama: &[u8]) -> Result<&[u8], ErroreTrama> {
    let [lunghezza, .., ricevuto] = trama else {
        return Err(ErroreTrama::Corta);
    };
    if trama.len() < 3 {
        return Err(ErroreTrama::Corta);
    }
    let presenti = trama.len() - 2;
    if *lunghezza as usize != presenti {
        return Err(ErroreTrama::Lunghezza {
            dichiarata: *lunghezza,
            presenti,
        });
    }
    let atteso = checksum(&trama[..trama.len() - 1]);
    if atteso != *ricevuto {
        return Err(ErroreTrama::Checksum {
            atteso,
            ricevuto: *ricevuto,
        });
    }
    Ok(&trama[1..trama.len() - 1])
}

/// Risposta della centralina a una richiesta, già separata dalla cornice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Risposta {
    /// Corpo della risposta positiva: servizio + 40h, poi i dati.
    Positiva(Vec<u8>),
    /// `7F <servizio> <codice>`.
    Negativa { servizio: u8, codice: u8 },
}

/// Trova la risposta alla richiesta `inviata` (messaggio completo) fra i
/// byte stampati dall'adattatore. È tollerante perché il formato esatto della
/// stampa di un OBDLink in questi protocolli non è verificato (vedi
/// docs/RICERCA_TD5.md): accetta l'eco della richiesta in testa, la risposta
/// con o senza checksum e, in ultima istanza, il solo corpo senza lunghezza.
/// `None` se nessuna lettura è plausibile.
pub fn estrai(byte: &[u8], inviata: &[u8]) -> Option<Risposta> {
    let servizio = *inviata.get(1)?;
    let positiva = servizio.wrapping_add(0x40);
    let mut b = byte;
    // Eco della richiesta (completa o senza checksum).
    for eco in [inviata, &inviata[..inviata.len().saturating_sub(1)]] {
        if !eco.is_empty() && b.len() > eco.len() && b.starts_with(eco) {
            b = &b[eco.len()..];
            break;
        }
    }
    let pertinente = |c: &[u8]| match c {
        [s, ..] if *s == positiva => true,
        [0x7F, s, _, ..] if *s == servizio => true,
        _ => false,
    };
    // 1. Un messaggio completo e valido, in qualunque posizione.
    for i in 0..b.len() {
        let n = b[i] as usize;
        if n == 0 || i + n + 2 > b.len() {
            continue;
        }
        if let Ok(corpo) = verifica(&b[i..i + n + 2])
            && pertinente(corpo)
        {
            return Some(classifica(corpo));
        }
    }
    // 2. Lunghezza in testa ma senza checksum (l'adattatore l'ha già tolto).
    if let [n, resto @ ..] = b
        && *n as usize == resto.len()
        && pertinente(resto)
    {
        return Some(classifica(resto));
    }
    // 3. Il solo corpo, senza lunghezza (con o senza checksum in coda: i
    //    decodificatori leggono a posizioni fisse e ignorano i byte in più).
    if pertinente(b) {
        return Some(classifica(b));
    }
    None
}

fn classifica(corpo: &[u8]) -> Risposta {
    match corpo {
        [0x7F, servizio, codice, ..] => Risposta::Negativa {
            servizio: *servizio,
            codice: *codice,
        },
        altro => Risposta::Positiva(altro.to_vec()),
    }
}

/// Byte in esadecimale separati da spazi («02 10 A0 B2»).
pub fn esadecimale(byte: &[u8]) -> String {
    byte.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Byte in esadecimale senza spazi («0210A0»), come li vuole l'adattatore.
pub fn esadecimale_compatto(byte: &[u8]) -> String {
    byte.iter().map(|b| format!("{b:02X}")).collect()
}

/// Interpreta una riga di cifre esadecimali, con o senza spazi. `None` se la
/// riga contiene altro (testo dell'adattatore come «OK» o «NO DATA»).
pub fn da_esadecimale(riga: &str) -> Option<Vec<u8>> {
    let cifre: String = riga.chars().filter(|c| !c.is_whitespace()).collect();
    if cifre.is_empty()
        || !cifre.len().is_multiple_of(2)
        || !cifre.chars().all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    (0..cifre.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(cifre.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Spiegazione di un codice di risposta negativa (ricerca/td5/proto.md,
/// «Common Error Codes», più i codici KWP2000 dell'accesso di sicurezza).
pub fn motivo_negativo(codice: u8) -> String {
    match codice {
        0x10 => "rifiuto generico (10)".into(),
        0x11 => "servizio non supportato (11)".into(),
        0x12 => "sottofunzione non supportata (12)".into(),
        0x22 => "condizioni non corrette (22)".into(),
        0x31 => "richiesta fuori intervallo (31)".into(),
        0x33 => "accesso di sicurezza negato (33)".into(),
        0x35 => "chiave non valida (35)".into(),
        0x36 => "troppi tentativi (36)".into(),
        0x37 => "attesa obbligatoria prima di riprovare (37)".into(),
        altro => format!("codice {altro:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        da_esadecimale(s).unwrap()
    }

    /// Esempi di ricerca/td5/proto.md con il checksum giusto.
    #[test]
    fn esempi_della_documentazione() {
        for (s, corpo) in [
            ("02 10 A0 B2", "10 A0"),
            ("02 50 A0 F2", "50 A0"),
            ("02 27 01 2A", "27 01"),
            ("02 67 02 6B", "67 02"),
            ("03 C1 57 8F AA", "C1 57 8F"),
            ("02 21 09 2C", "21 09"),
            ("02 21 1A 3D", "21 1A"),
            ("06 61 10 36 43 36 39 5F", "61 10 36 43 36 39"),
            ("01 3E 3F", "3E"),
            ("01 7E 7F", "7E"),
            // Dal README di td5keygen.
            ("04 67 01 34 A5 45", "67 01 34 A5"),
            ("04 27 02 54 D3 54", "27 02 54 D3"),
        ] {
            assert_eq!(verifica(&hex(s)).unwrap(), hex(corpo).as_slice(), "{s}");
            assert_eq!(componi(&hex(corpo)), hex(s), "{s}");
        }
        assert_eq!(
            verifica(&hex(
                "12 61 1A 0D 6E 02 BA 0B 94 0A B5 0B 74 0B 61 0D 2C 04 69 B3"
            ))
            .unwrap()
            .len(),
            0x12
        );
    }

    /// Tre esempi di proto.md hanno il checksum sbagliato (la somma dà 58, 6B
    /// e 11): vengono rifiutati, non accettati per errore.
    #[test]
    fn esempi_con_checksum_errato_nella_documentazione() {
        for (s, giusto) in [
            ("04 61 09 02 E8 57", 0x58),
            ("0C 61 1B 02 81 11 44 11 5F 00 00 13 88 48", 0x6B),
            ("0A 61 1C 27 E9 28 06 00 2C 00 20 F3", 0x11),
        ] {
            assert!(
                matches!(verifica(&hex(s)), Err(ErroreTrama::Checksum { atteso, .. }) if atteso == giusto),
                "{s}"
            );
        }
    }

    #[test]
    fn lunghezza_e_brevita() {
        assert_eq!(verifica(&[0x02, 0x50]), Err(ErroreTrama::Corta));
        assert!(matches!(
            verifica(&hex("03 50 A0 F3")),
            Err(ErroreTrama::Lunghezza { .. })
        ));
    }

    #[test]
    fn estrazione_tollerante() {
        let inviata = hex("02 10 A0 B2");
        let ok = Some(Risposta::Positiva(hex("50 A0")));
        // Messaggio completo; con eco; senza checksum; solo corpo.
        assert_eq!(estrai(&hex("02 50 A0 F2"), &inviata), ok);
        assert_eq!(estrai(&hex("02 10 A0 B2 02 50 A0 F2"), &inviata), ok);
        assert_eq!(estrai(&hex("02 10 A0 02 50 A0 F2"), &inviata), ok);
        assert_eq!(estrai(&hex("02 50 A0"), &inviata), ok);
        assert_eq!(estrai(&hex("50 A0"), &inviata), ok);
        // Negativa.
        assert_eq!(
            estrai(&hex("03 7F 10 12 A4"), &inviata),
            Some(Risposta::Negativa {
                servizio: 0x10,
                codice: 0x12
            })
        );
        // Risposta a un altro servizio, o rumore: niente.
        assert_eq!(estrai(&hex("02 67 02 6B"), &inviata), None);
        assert_eq!(estrai(&hex("FF FF"), &inviata), None);
        assert_eq!(estrai(&[], &inviata), None);
    }

    #[test]
    fn esadecimale_andata_e_ritorno() {
        assert_eq!(da_esadecimale("02 10 a0"), Some(vec![0x02, 0x10, 0xA0]));
        assert_eq!(da_esadecimale("0210A0"), Some(vec![0x02, 0x10, 0xA0]));
        assert_eq!(da_esadecimale("OK"), None);
        assert_eq!(da_esadecimale("NO DATA"), None);
        assert_eq!(da_esadecimale("021"), None);
        assert_eq!(esadecimale(&[0x02, 0xA0]), "02 A0");
        assert_eq!(esadecimale_compatto(&[0x02, 0xA0]), "02A0");
    }
}
