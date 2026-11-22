//! Il btleplug incluso in `vendor/btleplug` non deve tornare ad avere
//! `.unwrap()`/`.expect(` (né `panic!` e simili) nel codice Windows fuori dai
//! test. Molte di quelle chiamate girano nei gestori di eventi WinRT, dove un
//! panic abortisce l'intero processo (vedi vendor/btleplug/PATCH_OBDIO.md):
//! se un aggiornamento di btleplug ne reintroduce una, questa prova fallisce
//! e la indica, con file e riga.

use std::path::{Path, PathBuf};

/// File controllati: tutto il codice WinRT di btleplug.
const FILE: [&str; 9] = [
    "src/winrtble/adapter.rs",
    "src/winrtble/manager.rs",
    "src/winrtble/peripheral.rs",
    "src/winrtble/utils.rs",
    "src/winrtble/ble/characteristic.rs",
    "src/winrtble/ble/descriptor.rs",
    "src/winrtble/ble/device.rs",
    "src/winrtble/ble/service.rs",
    "src/winrtble/ble/watcher.rs",
];

/// Parole vietate fuori dai test.
const VIETATE: [&str; 6] = [
    ".unwrap()",
    ".expect(",
    "panic!(",
    "unreachable!(",
    "todo!(",
    "unimplemented!(",
];

/// Eccezioni ammesse: (file, testo della riga senza spazi ai lati, motivo).
/// Oggi nessuna: ogni nuova voce va giustificata qui.
const AMMESSE: [(&str, &str, &str); 0] = [];

fn radice() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/btleplug")
}

/// Righe (numero, testo) fuori dai blocchi `#[cfg(test)]`, senza commenti.
fn righe_di_codice(testo: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut salta_prossimo = false;
    let mut profondita: Option<i64> = None;
    for (i, riga) in testo.lines().enumerate() {
        let codice = match riga.find("//") {
            Some(p) => &riga[..p],
            None => riga,
        };
        if let Some(p) = profondita.as_mut() {
            *p += codice.matches('{').count() as i64 - codice.matches('}').count() as i64;
            if *p <= 0 {
                profondita = None;
            }
            continue;
        }
        if codice.trim_start().starts_with("#[cfg(test)]")
            || codice.trim_start().starts_with("#[cfg(all(test")
        {
            salta_prossimo = true;
            continue;
        }
        if salta_prossimo {
            if codice.trim_start().starts_with("#[") {
                // Altri attributi dello stesso elemento.
                continue;
            }
            salta_prossimo = false;
            let aperte = codice.matches('{').count() as i64 - codice.matches('}').count() as i64;
            if aperte > 0 {
                profondita = Some(aperte);
            }
            // Elemento su una riga sola (`use …;`, `mod tests;`): saltato.
            continue;
        }
        out.push((i + 1, codice.to_string()));
    }
    out
}

#[test]
fn nessun_panic_nel_codice_winrt_di_btleplug() {
    let mut trovate = Vec::new();
    for f in FILE {
        let percorso = radice().join(f);
        let testo = std::fs::read_to_string(&percorso)
            .unwrap_or_else(|e| panic!("{} non leggibile: {e}", percorso.display()));
        for (n, codice) in righe_di_codice(&testo) {
            for v in VIETATE {
                let ammessa = AMMESSE
                    .iter()
                    .any(|(af, ar, _)| *af == f && codice.trim() == *ar);
                if codice.contains(v) && !ammessa {
                    trovate.push(format!("{f}:{n}: {}", codice.trim()));
                }
            }
        }
    }
    assert!(
        trovate.is_empty(),
        "chiamate che possono andare in panic nel codice WinRT di btleplug (vedi \
         vendor/btleplug/PATCH_OBDIO.md):\n{}",
        trovate.join("\n")
    );
}

#[test]
fn il_filtro_dei_test_funziona() {
    let esempio = "fn a() {\n    x.unwrap_or(0);\n}\n// y.unwrap()\n#[cfg(test)]\nmod tests {\n    \
                   fn b() { z.unwrap(); }\n}\n#[cfg(test)]\nuse std::str::FromStr;\nfn c() { w.unwrap() }\n";
    let righe = righe_di_codice(esempio);
    let testo: Vec<&str> = righe.iter().map(|(_, r)| r.as_str()).collect();
    assert!(testo.iter().all(|r| !r.contains("z.unwrap")));
    assert!(testo.iter().all(|r| !r.contains("FromStr")));
    assert!(testo.iter().all(|r| !r.contains("y.unwrap")));
    assert!(testo.iter().any(|r| r.contains("w.unwrap()")));
    assert_eq!(righe.last().unwrap().0, 11);
}

#[test]
fn cargo_usa_il_btleplug_incluso() {
    let manifesto =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(manifesto.contains("[patch.crates-io]"));
    assert!(manifesto.contains(r#"btleplug = { path = "vendor/btleplug" }"#));
    assert!(radice().join("PATCH_OBDIO.md").is_file());
    assert!(radice().join("LICENSE.md").is_file());
}
