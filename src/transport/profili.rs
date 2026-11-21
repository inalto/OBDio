//! Profili GATT «seriale su BLE» documentati per adattatori ELM327.
//!
//! L'app scrive su una caratteristica solo se servizio, caratteristica di
//! scrittura e caratteristica di notifica corrispondono a uno di questi
//! profili E hanno le proprietà GATT attese. In caso contrario mostra gli
//! UUID trovati e non scrive nulla.
//!
//! Nessuno di questi profili è stato finora provato da questa app su un
//! adattatore Carista reale: Carista non pubblica gli UUID dei propri
//! adattatori (vedi README, sezione «Stato delle verifiche»).

use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub struct ProfiloBle {
    pub nome: &'static str,
    pub servizio: Uuid,
    pub scrittura: Uuid,
    pub notifica: Uuid,
    /// Dove è documentato il profilo.
    pub fonte: &'static str,
}

/// UUID a 16 bit nella base Bluetooth standard.
const fn breve(v: u16) -> Uuid {
    Uuid::from_u128(((v as u128) << 96) | 0x0000_1000_8000_0080_5F9B_34FB)
}

pub const PROFILI: [ProfiloBle; 4] = [
    ProfiloBle {
        nome: "FFF0 (FFF1 notifica, FFF2 scrittura)",
        servizio: breve(0xFFF0),
        scrittura: breve(0xFFF2),
        notifica: breve(0xFFF1),
        fonte: "Note ufficiali OBDLink CX; diffuso tra gli adattatori ELM327 BLE",
    },
    ProfiloBle {
        nome: "18F0 (2AF0 notifica, 2AF1 scrittura)",
        servizio: breve(0x18F0),
        scrittura: breve(0x2AF1),
        notifica: breve(0x2AF0),
        fonte: "Vgate iCar Pro 2S (dump GATT pubblicato, progetto obd2_ble)",
    },
    ProfiloBle {
        nome: "FFE0 (FFE1 notifica e scrittura, tipo HM-10)",
        servizio: breve(0xFFE0),
        scrittura: breve(0xFFE1),
        notifica: breve(0xFFE1),
        fonte: "Moduli seriali HM-10 usati da adattatori ELM327 generici",
    },
    ProfiloBle {
        nome: "Nordic UART (6E400002 scrittura, 6E400003 notifica)",
        servizio: Uuid::from_u128(0x6E400001_B5A3_F393_E0A9_E50E24DCCA9E),
        scrittura: Uuid::from_u128(0x6E400002_B5A3_F393_E0A9_E50E24DCCA9E),
        notifica: Uuid::from_u128(0x6E400003_B5A3_F393_E0A9_E50E24DCCA9E),
        fonte: "Nordic UART Service (documentazione Nordic Semiconductor)",
    },
];

/// Caratteristica GATT come scoperta sul dispositivo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaratteristicaVista {
    pub servizio: Uuid,
    pub uuid: Uuid,
    pub scrivibile: bool,
    pub notificabile: bool,
}

/// Cerca il primo profilo documentato pienamente rispettato dal dispositivo.
pub fn riconosci(viste: &[CaratteristicaVista]) -> Option<&'static ProfiloBle> {
    PROFILI.iter().find(|p| {
        let ha = |uuid: Uuid, test: fn(&CaratteristicaVista) -> bool| {
            viste
                .iter()
                .any(|c| c.servizio == p.servizio && c.uuid == uuid && test(c))
        };
        ha(p.scrittura, |c| c.scrivibile) && ha(p.notifica, |c| c.notificabile)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(servizio: u16, uuid: u16, scrivibile: bool, notificabile: bool) -> CaratteristicaVista {
        CaratteristicaVista {
            servizio: breve(servizio),
            uuid: breve(uuid),
            scrivibile,
            notificabile,
        }
    }

    #[test]
    fn uuid_brevi() {
        assert_eq!(
            breve(0xFFF0).to_string(),
            "0000fff0-0000-1000-8000-00805f9b34fb"
        );
    }

    #[test]
    fn profilo_fff0() {
        let viste = [
            c(0x1800, 0x2A00, false, false),
            c(0xFFF0, 0xFFF1, false, true),
            c(0xFFF0, 0xFFF2, true, false),
        ];
        assert_eq!(riconosci(&viste).unwrap().servizio, breve(0xFFF0));
    }

    #[test]
    fn proprieta_sbagliate_o_servizio_ignoto() {
        // UUID giusti ma la caratteristica di scrittura non è scrivibile.
        let viste = [
            c(0xFFF0, 0xFFF1, false, true),
            c(0xFFF0, 0xFFF2, false, false),
        ];
        assert!(riconosci(&viste).is_none());
        // Servizio proprietario sconosciuto con una caratteristica scrivibile:
        // non si tenta nulla.
        let viste = [c(0xABCD, 0x0001, true, true)];
        assert!(riconosci(&viste).is_none());
    }

    #[test]
    fn profilo_hm10_caratteristica_unica() {
        let viste = [c(0xFFE0, 0xFFE1, true, true)];
        assert_eq!(riconosci(&viste).unwrap().servizio, breve(0xFFE0));
    }
}
