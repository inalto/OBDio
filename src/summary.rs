//! Riepilogo testuale della diagnosi, da copiare negli appunti.

use crate::diagnosis::{DiagnosisReport, Lettura};
use crate::model::{Stato, num_it, num_it_segno};
use crate::obd::pids;

fn nota_lettura(l: &Lettura) -> String {
    if l.stato == Stato::Ok {
        if l.dettaglio.is_empty() {
            "letto".into()
        } else {
            format!("letto ({})", l.dettaglio)
        }
    } else if l.dettaglio.is_empty() {
        l.stato.etichetta().to_lowercase()
    } else {
        format!("{} — {}", l.stato.etichetta().to_lowercase(), l.dettaglio)
    }
}

/// Formatta il solo numero secondo i decimali e il segno del parametro.
pub fn numero_param(key: &str, valore: f64) -> String {
    let (decimali, segno) = pids::per_key(key)
        .map(|p| (p.decimali, p.con_segno))
        .unwrap_or((2, false));
    if segno {
        num_it_segno(valore, decimali)
    } else {
        num_it(valore, decimali)
    }
}

/// Formatta un valore con l'unità secondo il parametro.
pub fn valore_con_unita(key: &str, valore: f64, unita: &str) -> String {
    let numero = numero_param(key, valore);
    if unita.is_empty() {
        numero
    } else {
        format!("{numero} {unita}")
    }
}

/// Costruisce il riepilogo. Il VIN compare solo se `includi_vin` è vero.
pub fn riepilogo(r: &DiagnosisReport, includi_vin: bool) -> String {
    let mut s = String::new();
    let mut riga = |t: String| {
        s.push_str(&t);
        s.push('\n');
    };
    riga("OBDio — Riepilogo diagnosi".into());
    if r.demo {
        riga("*** MODALITÀ DEMO: dati simulati, non provenienti da un veicolo ***".into());
    }
    riga(format!("Lettura del: {}", r.letto_il));
    riga(format!("Adattatore: {}", r.adattatore));
    riga(format!("Collegamento: {}", r.collegamento));
    riga(format!("Protocollo: {}", r.protocollo));
    riga(format!("Centraline che rispondono: {}", r.ecu.join(", ")));
    if let Some(t) = &r.tensione {
        riga(format!("Tensione al connettore (adattatore): {t}"));
    }
    if let Some(c) = &r.tipo_carburante {
        riga(format!("Tipo di carburante dichiarato: {c}"));
    }
    if let Some(n) = &r.norma_obd {
        riga(format!("Norma OBD dichiarata: {n}"));
    }
    match (&r.vin, includi_vin) {
        (Some(v), true) => riga(format!("VIN: {v}")),
        (Some(_), false) => riga("VIN: escluso dal riepilogo".into()),
        (None, _) => riga("VIN: non letto".into()),
    }
    if let Some(t) = &r.td5 {
        riga(
            "Centralina Land Rover Td5 (modalità di sola lettura, mai provata su un veicolo)"
                .into(),
        );
        let o = |v: &Option<String>| v.clone().unwrap_or_else(|| "non fornito".into());
        riga(format!("Tipo di centralina: {}", o(&t.tipo_centralina)));
        riga(format!("Variante della mappa: {}", o(&t.variante_mappa)));
        if let Some(v) = t.tensione_batteria {
            riga(format!(
                "Tensione della batteria (centralina): {} V",
                num_it(v, 2)
            ));
        }
        riga("I codici guasto sono nella numerazione del Td5, non codici OBD-II.".into());
    }

    riga(String::new());
    riga(format!(
        "Spia MIL e prontezza: {}",
        nota_lettura(&r.lettura_prontezza)
    ));
    for p in &r.prontezza {
        riga(format!(
            "- ECU {}: MIL {}, DTC dichiarati {}",
            p.ecu,
            if p.mil_accesa { "ACCESA" } else { "spenta" },
            p.numero_dtc
        ));
        for m in p.monitor.iter().filter(|m| m.disponibile) {
            riga(format!(
                "    {}: {}",
                m.nome,
                if m.completato {
                    "completato"
                } else {
                    "non completato"
                }
            ));
        }
    }

    riga(String::new());
    riga("Codici guasto:".into());
    if r.dtc.is_empty() {
        riga("- nessun codice riportato".into());
    }
    for d in &r.dtc {
        let stati: Vec<&str> = d.stati.iter().map(|x| x.etichetta()).collect();
        riga(format!(
            "- {} — {} — ECU {} — {}",
            d.codice,
            d.descrizione,
            d.ecu,
            stati.join(", ")
        ));
    }
    riga(format!(
        "  Confermati: {}",
        nota_lettura(&r.lettura_confermati)
    ));
    riga(format!(
        "  In sospeso: {}",
        nota_lettura(&r.lettura_in_sospeso)
    ));
    riga(format!(
        "  Permanenti: {}",
        nota_lettura(&r.lettura_permanenti)
    ));

    riga(String::new());
    if r.fermo.presente {
        riga(format!(
            "Fermo immagine (dati memorizzati, non in tempo reale) — DTC {} — ECU {}:",
            r.fermo.dtc.as_deref().unwrap_or("?"),
            r.fermo.ecu
        ));
        for x in &r.fermo.righe {
            let valore = match (x.stato, x.valore, &x.testo) {
                (Stato::Ok, Some(v), _) => valore_con_unita(&x.key, v, &x.unita),
                (Stato::Ok, None, Some(t)) => t.clone(),
                (stato, _, _) => format!("[{}]", stato.etichetta()),
            };
            riga(format!("- {}: {}", x.nome, valore));
        }
    } else {
        riga(format!(
            "Fermo immagine: non disponibile ({})",
            nota_lettura(&r.fermo.lettura)
        ));
    }

    riga(String::new());
    riga(format!(
        "Monitor catalizzatore, Mode 06 MID 21: {}",
        nota_lettura(&r.mode06.lettura)
    ));
    for t in &r.mode06.test {
        let grezzo = format!(
            "grezzo valore {:04X}, min {:04X}, max {:04X}, scala {:02X}",
            t.grezzo_valore, t.grezzo_minimo, t.grezzo_massimo, t.uas_id
        );
        match &t.scalato {
            Some(x) => riga(format!(
                "- TID {:02X}: valore {} (min {}, max {}) {} — {}",
                t.tid,
                num_it(x.valore, 4),
                num_it(x.minimo, 4),
                num_it(x.massimo, 4),
                x.unita,
                grezzo
            )),
            None => riga(format!(
                "- TID {:02X}: {} (scala non in tabella: solo dato grezzo)",
                t.tid, grezzo
            )),
        }
    }

    riga(String::new());
    riga("Parametri disponibili (PID del servizio 01):".into());
    for (ecu, lista) in &r.pid_supportati {
        let testo: Vec<String> = lista.iter().map(|p| format!("{p:02X}")).collect();
        riga(format!("- ECU {}: {}", ecu, testo.join(" ")));
    }
    riga(String::new());
    riga(
        "Nota: questo riepilogo riporta i dati letti dalla centralina. Non stabilisce quale \
         componente sia guasto."
            .into(),
    );
    s
}
