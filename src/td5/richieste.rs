//! Elenco chiuso delle richieste che OBDio invia alla centralina Td5.
//!
//! Sono tutte di lettura, più quelle necessarie ad aprire e tenere aperta la
//! sessione (sessione diagnostica, accesso di sicurezza, mantenimento). Non
//! esiste un modo di inviare byte arbitrari: l'interfaccia e il server MCP
//! scelgono al più una voce di [`IdLocale`].
//!
//! Volutamente ASSENTI (ricerca/td5/Requests.java): la cancellazione dei
//! guasti (`31 DD …`), le prove degli attuatori (`30 …`, `31 C2 …`) e ogni
//! servizio di scrittura o programmazione.

use super::chiave::chiave_da_seme;
use super::frame;

/// StartCommunication con intestazione KWP2000 (centralina 13h, tester F7h),
/// l'unico messaggio con intestazione (ricerca/td5/proto.md,
/// «Start Communication Request»). Lo invia l'adattatore durante
/// l'inizializzazione rapida.
pub const INIZIO_COMUNICAZIONE: [u8; 5] = [0x81, 0x13, 0xF7, 0x81, 0x0C];

/// Dati in tempo reale letti con il servizio 21h. Sono esclusi gli
/// identificativi segnalati come inaffidabili in ricerca/td5/pid.txt e
/// proto.md (07, 08, 0A, 0B, 0F, 17, 20, 2B, 2C) e lo 0x21, di significato
/// incerto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IdLocale {
    /// 09: giri motore.
    Giri,
    /// 0D: velocità.
    Velocita,
    /// 10: tensione della batteria.
    Batteria,
    /// 1A: temperature (liquido, aria aspirata, gasolio).
    Temperature,
    /// 1B: tracce del pedale dell'acceleratore e alimentazione dei sensori.
    Pedale,
    /// 1C: pressione nel collettore.
    Collettore,
    /// 1E: interruttori (freni, frizione, cruise, A/C, riduttore).
    Interruttori,
    /// 23: pressione ambiente.
    PressioneAmbiente,
    /// 37: posizione della valvola EGR.
    Egr,
    /// 38: posizione della wastegate.
    Wastegate,
    /// 40: correzione degli iniettori per cilindro.
    Iniettori,
}

impl IdLocale {
    pub const TUTTI: [IdLocale; 11] = [
        IdLocale::Giri,
        IdLocale::Velocita,
        IdLocale::Batteria,
        IdLocale::Temperature,
        IdLocale::Pedale,
        IdLocale::Collettore,
        IdLocale::Interruttori,
        IdLocale::PressioneAmbiente,
        IdLocale::Egr,
        IdLocale::Wastegate,
        IdLocale::Iniettori,
    ];

    pub fn codice(self) -> u8 {
        match self {
            IdLocale::Giri => 0x09,
            IdLocale::Velocita => 0x0D,
            IdLocale::Batteria => 0x10,
            IdLocale::Temperature => 0x1A,
            IdLocale::Pedale => 0x1B,
            IdLocale::Collettore => 0x1C,
            IdLocale::Interruttori => 0x1E,
            IdLocale::PressioneAmbiente => 0x23,
            IdLocale::Egr => 0x37,
            IdLocale::Wastegate => 0x38,
            IdLocale::Iniettori => 0x40,
        }
    }

    pub fn da_codice(codice: u8) -> Option<IdLocale> {
        IdLocale::TUTTI.into_iter().find(|i| i.codice() == codice)
    }

    pub fn nome(self) -> &'static str {
        match self {
            IdLocale::Giri => "giri motore",
            IdLocale::Velocita => "velocità",
            IdLocale::Batteria => "tensione batteria",
            IdLocale::Temperature => "temperature",
            IdLocale::Pedale => "pedale acceleratore",
            IdLocale::Collettore => "pressione collettore",
            IdLocale::Interruttori => "interruttori",
            IdLocale::PressioneAmbiente => "pressione ambiente",
            IdLocale::Egr => "posizione EGR",
            IdLocale::Wastegate => "posizione wastegate",
            IdLocale::Iniettori => "correzione iniettori",
        }
    }
}

/// Chiave dell'accesso di sicurezza. Si ottiene solo dal seme mandato dalla
/// centralina: non è un modo di inviare due byte qualsiasi.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chiave(u16);

impl Chiave {
    pub fn da_seme(seme: u16) -> Self {
        Chiave(chiave_da_seme(seme))
    }

    pub fn valore(self) -> u16 {
        self.0
    }
}

/// Le sole richieste ammesse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RichiestaTd5 {
    /// `10 A0`: avvio della sessione diagnostica.
    SessioneDiagnostica,
    /// `27 01`: richiesta del seme.
    RichiestaSeme,
    /// `27 02 <chiave>`: invio della chiave.
    InvioChiave(Chiave),
    /// `3E 01`: TesterPresent, come in Td5OpenDiag (ricerca/td5/Requests.java,
    /// KEEP_ALIVE). proto.md riporta invece `01 3E 3F`: vedi
    /// docs/RICERCA_TD5.md.
    Mantenimento,
    /// `21 <id>`: dati in tempo reale.
    Dati(IdLocale),
    /// `21 3B`: mappa di bit dei codici guasto (Requests.java, FAULT_CODES).
    CodiciGuasto,
    /// `1A 87`: numero di telaio. La risposta non va mai nei registri.
    Vin,
    /// `1A 9A`: tipo di centralina.
    TipoCentralina,
    /// `21 32`: variante della mappa.
    VarianteMappa,
}

impl RichiestaTd5 {
    /// Una richiesta per ogni variante (per le verifiche sull'elenco chiuso).
    pub fn esempi() -> Vec<RichiestaTd5> {
        // Il `match` obbliga ad aggiornare questo elenco con ogni variante nuova.
        let tutte = |r: RichiestaTd5| match r {
            RichiestaTd5::SessioneDiagnostica
            | RichiestaTd5::RichiestaSeme
            | RichiestaTd5::InvioChiave(_)
            | RichiestaTd5::Mantenimento
            | RichiestaTd5::Dati(_)
            | RichiestaTd5::CodiciGuasto
            | RichiestaTd5::Vin
            | RichiestaTd5::TipoCentralina
            | RichiestaTd5::VarianteMappa => r,
        };
        let mut v: Vec<RichiestaTd5> = [
            RichiestaTd5::SessioneDiagnostica,
            RichiestaTd5::RichiestaSeme,
            RichiestaTd5::InvioChiave(Chiave::da_seme(0x34A5)),
            RichiestaTd5::Mantenimento,
            RichiestaTd5::CodiciGuasto,
            RichiestaTd5::Vin,
            RichiestaTd5::TipoCentralina,
            RichiestaTd5::VarianteMappa,
        ]
        .into_iter()
        .map(tutte)
        .collect();
        v.extend(IdLocale::TUTTI.into_iter().map(RichiestaTd5::Dati));
        v
    }

    /// Servizio e dati, senza lunghezza né checksum.
    pub fn corpo(self) -> Vec<u8> {
        match self {
            RichiestaTd5::SessioneDiagnostica => vec![0x10, 0xA0],
            RichiestaTd5::RichiestaSeme => vec![0x27, 0x01],
            RichiestaTd5::InvioChiave(c) => {
                let [alto, basso] = c.valore().to_be_bytes();
                vec![0x27, 0x02, alto, basso]
            }
            RichiestaTd5::Mantenimento => vec![0x3E, 0x01],
            RichiestaTd5::Dati(id) => vec![0x21, id.codice()],
            RichiestaTd5::CodiciGuasto => vec![0x21, 0x3B],
            RichiestaTd5::Vin => vec![0x1A, 0x87],
            RichiestaTd5::TipoCentralina => vec![0x1A, 0x9A],
            RichiestaTd5::VarianteMappa => vec![0x21, 0x32],
        }
    }

    /// Messaggio completo, con lunghezza e checksum.
    pub fn trama(self) -> Vec<u8> {
        frame::componi(&self.corpo())
    }

    pub fn servizio(self) -> u8 {
        self.corpo()[0]
    }

    /// La risposta non va nei registri (numero di telaio).
    pub fn riservata(self) -> bool {
        self == RichiestaTd5::Vin
    }

    /// Nome per i messaggi e il registro.
    pub fn nome(self) -> String {
        match self {
            RichiestaTd5::SessioneDiagnostica => "avvio della sessione diagnostica".into(),
            RichiestaTd5::RichiestaSeme => "richiesta del seme".into(),
            RichiestaTd5::InvioChiave(_) => "invio della chiave".into(),
            RichiestaTd5::Mantenimento => "mantenimento della sessione".into(),
            RichiestaTd5::Dati(id) => format!("lettura {} ({:02X})", id.nome(), id.codice()),
            RichiestaTd5::CodiciGuasto => "lettura dei codici guasto".into(),
            RichiestaTd5::Vin => "lettura del numero di telaio".into(),
            RichiestaTd5::TipoCentralina => "lettura del tipo di centralina".into(),
            RichiestaTd5::VarianteMappa => "lettura della variante della mappa".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trame_della_documentazione() {
        let h = |r: RichiestaTd5| frame::esadecimale(&r.trama());
        assert_eq!(h(RichiestaTd5::SessioneDiagnostica), "02 10 A0 B2");
        assert_eq!(h(RichiestaTd5::RichiestaSeme), "02 27 01 2A");
        assert_eq!(
            h(RichiestaTd5::InvioChiave(Chiave::da_seme(0x34A5))),
            "04 27 02 54 D3 54"
        );
        assert_eq!(h(RichiestaTd5::Dati(IdLocale::Giri)), "02 21 09 2C");
        assert_eq!(h(RichiestaTd5::Dati(IdLocale::Temperature)), "02 21 1A 3D");
        assert_eq!(h(RichiestaTd5::Dati(IdLocale::Pedale)), "02 21 1B 3E");
        assert_eq!(h(RichiestaTd5::Dati(IdLocale::Collettore)), "02 21 1C 3F");
        assert_eq!(h(RichiestaTd5::Dati(IdLocale::Batteria)), "02 21 10 33");
        assert_eq!(
            h(RichiestaTd5::Dati(IdLocale::PressioneAmbiente)),
            "02 21 23 46"
        );
        assert_eq!(h(RichiestaTd5::Dati(IdLocale::Iniettori)), "02 21 40 63");
        assert_eq!(h(RichiestaTd5::Mantenimento), "02 3E 01 41");
        assert_eq!(h(RichiestaTd5::CodiciGuasto), "02 21 3B 5E");
        assert_eq!(frame::checksum(&INIZIO_COMUNICAZIONE[..4]), 0x0C);
    }

    /// Nessuna richiesta dell'elenco chiuso usa servizi che modificano la
    /// centralina: 14 e 31 (cancellazioni), 30 (attuatori), 2E/3B (scrittura),
    /// 34–37 (programmazione), 11 (riavvio).
    #[test]
    fn solo_servizi_di_lettura_e_di_sessione() {
        let ammessi = [0x10, 0x27, 0x3E, 0x21, 0x1A];
        for r in RichiestaTd5::esempi() {
            let s = r.servizio();
            assert!(ammessi.contains(&s), "{r:?} usa il servizio {s:02X}");
            assert!(!matches!(
                s,
                0x11 | 0x14 | 0x30 | 0x31 | 0x2E | 0x3B | 0x34..=0x37
            ));
            // Solo la sessione diagnostica A0 e l'accesso di livello 01/02.
            match r {
                RichiestaTd5::SessioneDiagnostica => assert_eq!(r.corpo(), [0x10, 0xA0]),
                RichiestaTd5::RichiestaSeme => assert_eq!(r.corpo(), [0x27, 0x01]),
                RichiestaTd5::InvioChiave(_) => assert_eq!(r.corpo()[..2], [0x27, 0x02]),
                _ => {}
            }
        }
        // Tutte le varianti e tutti gli identificativi sono coperti.
        assert_eq!(RichiestaTd5::esempi().len(), 8 + IdLocale::TUTTI.len());
    }

    #[test]
    fn identificativi_inaffidabili_esclusi() {
        for codice in [0x07, 0x08, 0x0A, 0x0B, 0x0F, 0x17, 0x20, 0x21, 0x2B, 0x2C] {
            assert!(IdLocale::da_codice(codice).is_none(), "{codice:02X}");
        }
        for id in IdLocale::TUTTI {
            assert_eq!(IdLocale::da_codice(id.codice()), Some(id));
        }
    }
}
