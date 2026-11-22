//! Diagnosi Bluetooth LE da riga di comando, per il PC dove l'adattatore non
//! si collega:
//!
//! ```text
//! OBDio.exe --diagnosi-ble [nome o indirizzo]
//! ```
//!
//! Scansione di 10 secondi, connessione e scoperta dei servizi GATT con lo
//! stesso codice dell'app ([`BleManager::collega_e_scopri`]), poi
//! scollegamento. NON invia alcun comando ELM327 e non scrive nulla sul
//! dispositivo: nemmeno l'attivazione delle notifiche, che sarebbe una
//! scrittura. Il resoconto va a video e in un file di testo accanto
//! all'eseguibile (o nella cartella dei registri se lì non si può scrivere).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::registri::{self, Diario};
use crate::transport::ble::{BleDispositivo, BleErrore, BleManager};

/// Durata della scansione.
pub const DURATA_SCANSIONE: Duration = Duration::from_secs(10);

/// Esito della diagnosi.
pub struct Resoconto {
    pub testo: String,
    /// File in cui è stato salvato, se il salvataggio è riuscito.
    pub file: Option<PathBuf>,
    /// Vero se connessione e scoperta dei servizi sono riuscite.
    pub riuscita: bool,
}

/// Sceglie il dispositivo: quello il cui nome contiene `cercato` (o con
/// quell'indirizzo); senza `cercato`, il primo probabile adattatore OBD.
pub fn scegli<'a>(
    dispositivi: &'a [BleDispositivo],
    cercato: Option<&str>,
) -> Option<&'a BleDispositivo> {
    match cercato.map(str::to_lowercase) {
        Some(c) => dispositivi
            .iter()
            .find(|d| d.indirizzo.to_lowercase() == c || d.nome.to_lowercase().contains(&c)),
        None => dispositivi.iter().find(|d| d.probabile_obd),
    }
}

/// Esegue la diagnosi. `stampa` riceve ogni riga appena prodotta.
pub fn esegui(cercato: Option<&str>, stampa: Arc<dyn Fn(&str) + Send + Sync>) -> Resoconto {
    let righe: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let scrivi = {
        let righe = righe.clone();
        let stampa = stampa.clone();
        Arc::new(move |r: String| {
            stampa(&r);
            righe.lock().unwrap_or_else(PoisonError::into_inner).push(r);
        })
    };
    let scrivi2 = scrivi.clone();
    let annota = move |r: &str| scrivi2(r.to_string());

    annota(&format!(
        "Diagnosi Bluetooth LE di OBDio {} — {}",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    annota(&format!("Sistema: {}", registri::descrizione_sistema()));
    annota("Nessun comando ELM327 viene inviato e nulla viene scritto sull'adattatore.");
    annota(&format!(
        "Registro dei collegamenti: {}",
        registri::percorso_collegamento().display()
    ));
    annota("");

    let eco_scrivi = scrivi.clone();
    let diario = Diario::su_file().con_eco(Arc::new(move |r| eco_scrivi(format!("  {r}"))));
    let riuscita = passi(cercato, diario, &annota);

    annota("");
    annota(if riuscita {
        "Esito: connessione e scoperta dei servizi riuscite."
    } else {
        "Esito: la diagnosi non è arrivata alla fine (vedi le righe sopra)."
    });
    let testo = righe
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .join("\r\n");
    let file = salva(&testo);
    Resoconto {
        testo,
        file,
        riuscita,
    }
}

fn passi(cercato: Option<&str>, diario: Diario, annota: &dyn Fn(&str)) -> bool {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => Arc::new(rt),
        Err(e) => {
            annota(&format!("Runtime asincrono non avviato: {e}"));
            return false;
        }
    };
    let ble = match BleManager::new(rt) {
        Ok(b) => b.con_diario(diario),
        Err(e) => {
            annota(&format!("Bluetooth non utilizzabile: {}", e.testo()));
            return false;
        }
    };
    if let Err(e) = ble.avvia_scansione() {
        annota(&format!("Scansione non avviata: {}", e.testo()));
        return false;
    }
    annota(&format!(
        "Scansione per {} secondi…",
        DURATA_SCANSIONE.as_secs()
    ));
    let inizio = Instant::now();
    while inizio.elapsed() < DURATA_SCANSIONE {
        std::thread::sleep(Duration::from_millis(500));
    }
    let dispositivi = ble.dispositivi();
    ble.ferma_scansione();
    annota(&format!("Dispositivi visti: {}", dispositivi.len()));
    for d in &dispositivi {
        annota(&format!(
            "  {} «{}» RSSI {}{}",
            d.indirizzo,
            d.nome,
            d.rssi
                .map(|r| format!("{r} dBm"))
                .unwrap_or_else(|| "?".into()),
            if d.probabile_obd {
                "  (probabile adattatore OBD)"
            } else {
                ""
            }
        ));
    }
    let Some(d) = scegli(&dispositivi, cercato) else {
        annota(&match cercato {
            Some(c) => format!("Nessun dispositivo «{c}» fra quelli visti."),
            None => "Nessun probabile adattatore OBD fra quelli visti: indica nome o indirizzo \
                     (OBDio.exe --diagnosi-ble carista)."
                .into(),
        });
        return false;
    };
    annota(&format!("Collegamento a «{}» [{}]…", d.nome, d.indirizzo));
    let s = match ble.collega_e_scopri(&d.indirizzo) {
        Ok(s) => s,
        Err(e) => {
            annota(&format!("Collegamento non riuscito: {}", e.testo()));
            if matches!(e, BleErrore::AccessoNegato(_)) {
                annota(
                    "Suggerimento: associa l'adattatore in Windows (Impostazioni → Bluetooth e \
                     dispositivi → Aggiungi dispositivo) e riprova, oppure usa la porta seriale \
                     (COM) che lo stesso adattatore offre.",
                );
            }
            return false;
        }
    };
    annota("");
    annota("Servizi GATT:");
    for riga in s.info.testo().lines() {
        annota(&format!("  {riga}"));
    }
    match s.profilo {
        Some(p) => annota(&format!(
            "Profilo documentato riconosciuto: {} — l'app può usarlo.",
            p.nome
        )),
        None => annota(
            "Nessun profilo documentato: l'app non scriverà su questo dispositivo. Invia questo \
             resoconto per aggiungere il profilo.",
        ),
    }
    ble.scollega(&s.periferica);
    true
}

/// Salva il resoconto accanto all'eseguibile o, se lì non si può, nella
/// cartella dei registri.
fn salva(testo: &str) -> Option<PathBuf> {
    let nome = format!(
        "diagnosi_ble_{}.txt",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    let accanto = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(&nome)));
    accanto
        .into_iter()
        .chain(Some(registri::cartella().join(&nome)))
        .find(|percorso| std::fs::write(percorso, testo).is_ok())
}

/// Punto d'ingresso di `OBDio.exe --diagnosi-ble [nome]`. L'eseguibile di
/// rilascio non ha una console: si aggancia a quella del terminale da cui è
/// stato avviato, se c'è; il resoconto è comunque salvato su file.
pub fn da_riga_di_comando(cercato: Option<String>) -> i32 {
    aggancia_console();
    // Senza `println!`: se il terminale non c'è più non deve andare in panic.
    let stampa: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|r| {
        use std::io::Write;
        let _ = writeln!(std::io::stdout().lock(), "{r}");
    });
    let r = esegui(cercato.as_deref(), stampa.clone());
    match &r.file {
        Some(f) => stampa(&format!("\nResoconto salvato in: {}", f.display())),
        None => stampa("\nResoconto NON salvato: nessuna cartella scrivibile."),
    }
    if r.riuscita { 0 } else { 1 }
}

#[cfg(windows)]
fn aggancia_console() {
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn AttachConsole(processo: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: nessun argomento puntatore; se il processo ha già una console
    // (build di sviluppo) la chiamata fallisce senza effetti.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn aggancia_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(indirizzo: &str, nome: &str, obd: bool) -> BleDispositivo {
        BleDispositivo {
            indirizzo: indirizzo.into(),
            nome: nome.into(),
            rssi: Some(-55),
            probabile_obd: obd,
        }
    }

    #[test]
    fn scelta_del_dispositivo() {
        let v = [
            d("AA:BB:CC:DD:EE:FF", "Cuffie", false),
            d("00:11:22:33:44:55", "Carista", true),
        ];
        assert_eq!(scegli(&v, None).unwrap().nome, "Carista");
        assert_eq!(scegli(&v, Some("CARI")).unwrap().nome, "Carista");
        assert_eq!(
            scegli(&v, Some("aa:bb:cc:dd:ee:ff")).unwrap().nome,
            "Cuffie"
        );
        assert!(scegli(&v, Some("vlink")).is_none());
        assert!(scegli(&v[..1], None).is_none());
    }
}
