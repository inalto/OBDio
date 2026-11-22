//! File `SHA256SUMS.txt` nel formato di `sha256sum`: una riga per file,
//! `<64 cifre esadecimali><due spazi><nome>` (modo testo) oppure
//! `<64 cifre><spazio>*<nome>` (modo binario). Fine riga LF o CRLF.

use sha2::{Digest, Sha256};

use super::ErroreAggiornamento;

/// Somma SHA-256 in esadecimale minuscolo.
pub fn esadecimale(byte: &[u8]) -> String {
    byte.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 di un blocco di byte, in esadecimale minuscolo.
pub fn sha256(byte: &[u8]) -> String {
    esadecimale(&Sha256::digest(byte))
}

/// Calcolo incrementale (per i file scaricati a pezzi).
#[derive(Default)]
pub struct Calcolo(Sha256);

impl Calcolo {
    pub fn aggiungi(&mut self, byte: &[u8]) {
        self.0.update(byte);
    }

    pub fn fine(self) -> String {
        esadecimale(&self.0.finalize())
    }
}

/// Legge una riga; `None` se non è nel formato atteso.
fn riga(r: &str) -> Option<(String, &str)> {
    let r = r.trim_end_matches(['\r', '\n']);
    let somma = r.get(..64)?;
    if !somma.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let resto = r.get(64..)?;
    let nome = resto
        .strip_prefix("  ")
        .or_else(|| resto.strip_prefix(" *"))?;
    let nome = nome.strip_prefix("./").unwrap_or(nome).trim_end();
    if nome.is_empty() {
        return None;
    }
    Some((somma.to_ascii_lowercase(), nome))
}

/// Somma attesa per `nome` nel testo di `SHA256SUMS.txt`. Le righe che non
/// sono nel formato vengono ignorate; due somme diverse per lo stesso file
/// sono un errore.
pub fn somma_per(testo: &str, nome: &str) -> Result<String, ErroreAggiornamento> {
    let testo = testo.trim_start_matches('\u{feff}');
    let mut trovata: Option<String> = None;
    for r in testo.lines() {
        if let Some((somma, n)) = riga(r)
            && n == nome
        {
            match &trovata {
                Some(s) if *s != somma => {
                    return Err(ErroreAggiornamento::SommaMancante(format!(
                        "{nome} compare due volte con somme diverse"
                    )));
                }
                _ => trovata = Some(somma),
            }
        }
    }
    trovata.ok_or_else(|| ErroreAggiornamento::SommaMancante(nome.to_string()))
}

/// Confronta la somma calcolata con quella attesa.
pub fn verifica(attesa: &str, ottenuta: &str) -> Result<(), ErroreAggiornamento> {
    if attesa.eq_ignore_ascii_case(ottenuta) {
        Ok(())
    } else {
        Err(ErroreAggiornamento::SommaDiversa {
            attesa: attesa.to_ascii_lowercase(),
            ottenuta: ottenuta.to_ascii_lowercase(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae";
    const B: &str = "FCDE2B2EDBA56BF408601FB721FE9B5C338D10EE429EA04FAE5511B68FBF8FB9";

    #[test]
    fn sha256_di_riferimento() {
        // Valori noti: «foo» e la stringa vuota.
        assert_eq!(sha256(b"foo"), A);
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let mut c = Calcolo::default();
        c.aggiungi(b"f");
        c.aggiungi(b"oo");
        assert_eq!(c.fine(), A);
    }

    #[test]
    fn entrambi_i_separatori_e_crlf() {
        let testo = format!(
            "\u{feff}{A}  OBDio-0.4.0-windows-x64.zip\r\n{B} *OBDio-0.4.0-windows-x64.exe\r\n\r\n"
        );
        assert_eq!(somma_per(&testo, "OBDio-0.4.0-windows-x64.zip").unwrap(), A);
        // Maiuscole accettate, restituite minuscole.
        assert_eq!(
            somma_per(&testo, "OBDio-0.4.0-windows-x64.exe").unwrap(),
            B.to_ascii_lowercase()
        );
        let con_punto = format!("{A}  ./OBDio.exe\n");
        assert_eq!(somma_per(&con_punto, "OBDio.exe").unwrap(), A);
    }

    #[test]
    fn file_sconosciuto_o_righe_malformate() {
        let testo = format!(
            "# commento\n{A} OBDio.exe\n{A}\tOBDio.exe\nxyz  OBDio.exe\n{}  OBDio.exe\n{A}  altro.exe\n",
            "g".repeat(64)
        );
        assert!(matches!(
            somma_per(&testo, "OBDio.exe"),
            Err(ErroreAggiornamento::SommaMancante(_))
        ));
        assert!(somma_per("", "OBDio.exe").is_err());
        // Il nome deve coincidere per intero.
        assert!(somma_per(&format!("{A}  OBDio.exe.bak\n"), "OBDio.exe").is_err());
    }

    #[test]
    fn somme_in_conflitto() {
        let testo = format!("{A}  OBDio.exe\n{B}  OBDio.exe\n");
        assert!(somma_per(&testo, "OBDio.exe").is_err());
        let doppia = format!("{A}  OBDio.exe\n{A} *OBDio.exe\n");
        assert_eq!(somma_per(&doppia, "OBDio.exe").unwrap(), A);
    }

    #[test]
    fn verifica_della_somma() {
        assert!(verifica(A, &A.to_ascii_uppercase()).is_ok());
        assert!(matches!(
            verifica(A, B),
            Err(ErroreAggiornamento::SommaDiversa { .. })
        ));
    }
}
