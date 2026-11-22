//! Registri su disco per capire che cosa è successo sul PC dell'utente:
//!
//! - `registro_errori.log`: ogni panic, con versione, sistema, thread,
//!   posizione e backtrace. Il gestore installato da [`installa_gestore_panic`]
//!   scrive e sincronizza il file prima di qualunque altra cosa, quindi la
//!   voce resta su disco anche quando il panic sta per diventare un abort del
//!   processo (panic dentro un callback `extern "system"`, per esempio di WinRT).
//! - `registro_collegamento.log`: i passi di ogni tentativo di collegamento
//!   (scansione, connessione, servizi GATT, notifiche, sequenza ELM327), con
//!   l'orario al millisecondo. Ogni riga è scritta subito con una chiamata di
//!   sistema (nessun buffer in memoria): se il processo muore, l'ultima riga
//!   scritta è già nel file. Oltre 256 kB il file passa a
//!   `registro_collegamento.1.log` e se ne apre uno nuovo.
//!
//! Entrambi stanno nella cartella `registri` dei dati
//! ([`crate::config::cartella_dati`]); se non è scrivibile, nella cartella
//! temporanea. La variabile d'ambiente `OBDIO_REGISTRI=<cartella>` la sostituisce
//! (verifiche automatiche e diagnosi).
//!
//! Il VIN non entra mai nei registri: le righe ELM327 arrivano dal registro
//! degli scambi, che omette già la risposta al VIN.

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Nome del registro degli errori fatali.
pub const FILE_ERRORI: &str = "registro_errori.log";
/// Nome del registro dei passi di collegamento.
pub const FILE_COLLEGAMENTO: &str = "registro_collegamento.log";
/// Copia precedente del registro dei collegamenti, dopo la rotazione.
pub const FILE_COLLEGAMENTO_PRECEDENTE: &str = "registro_collegamento.1.log";
/// Lunghezza del registro degli errori all'ultima chiusura regolare (o
/// all'ultima volta che l'utente ha chiuso l'avviso).
const FILE_SEGNO: &str = "errori_visti.txt";
/// Oltre questa misura il registro dei collegamenti ruota.
pub const LIMITE_COLLEGAMENTO: u64 = 256 * 1024;
/// Oltre questa misura il registro degli errori ruota (una copia `.1`).
const LIMITE_ERRORI: u64 = 1024 * 1024;
/// Limiti di ciò che il gestore dei panic scrive per ogni voce.
const MAX_MESSAGGIO: usize = 4 * 1024;
const MAX_BACKTRACE: usize = 64 * 1024;
/// Intestazione di ogni voce del registro degli errori.
const INIZIO_VOCE: &str = "=== OBDio ";

static CARTELLA: OnceLock<PathBuf> = OnceLock::new();
static SISTEMA: OnceLock<String> = OnceLock::new();
static SCRITTURA_ERRORI: Mutex<()> = Mutex::new(());
/// Registro dei collegamenti aperto, con il suo percorso.
static COLLEGAMENTO: Mutex<Option<(PathBuf, File)>> = Mutex::new(None);

thread_local! {
    /// Vero mentre questo thread sta scrivendo una voce di panic: un panic
    /// annidato non riscrive (e non si avvita).
    static IN_CORSO: Cell<bool> = const { Cell::new(false) };
    /// Messaggio dell'ultimo panic di questo thread (per l'avviso a video).
    static PRECEDENTE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Cartella dei registri, creata se manca.
pub fn cartella() -> PathBuf {
    CARTELLA
        .get_or_init(|| {
            if let Some(d) = std::env::var_os("OBDIO_REGISTRI").map(PathBuf::from)
                && fs::create_dir_all(&d).is_ok()
            {
                return d;
            }
            let d = crate::config::cartella_dati("registri");
            if d.is_dir() { d } else { cartella_temporanea() }
        })
        .clone()
}

fn cartella_temporanea() -> PathBuf {
    let d = std::env::temp_dir().join("OBDio").join("registri");
    let _ = fs::create_dir_all(&d);
    d
}

/// Percorso del registro degli errori.
pub fn percorso_errori() -> PathBuf {
    cartella().join(FILE_ERRORI)
}

/// Percorso del registro dei collegamenti.
pub fn percorso_collegamento() -> PathBuf {
    cartella().join(FILE_COLLEGAMENTO)
}

fn orario() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string()
}

/// Tronca `s` a non più di `max` byte, su un confine di carattere.
pub fn tronca(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut fine = max;
    while !s.is_char_boundary(fine) {
        fine -= 1;
    }
    &s[..fine]
}

// ---------------------------------------------------------------------------
// Errori fatali
// ---------------------------------------------------------------------------

/// Installa il gestore dei panic. Va chiamata per prima cosa in `main`: da lì
/// in poi ogni panic, su qualunque thread, finisce nel registro degli errori
/// prima di passare al gestore precedente (che stampa il messaggio come
/// sempre). Chiamate successive non aggiungono un secondo gestore.
pub fn installa_gestore_panic() {
    static FATTO: std::sync::Once = std::sync::Once::new();
    FATTO.call_once(|| {
        // Calcolato ora, fuori dal gestore.
        let _ = SISTEMA.get_or_init(versione_sistema);
        let precedente = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            registra_panic(info);
            precedente(info);
        }));
    });
}

/// Scrive la voce del panic. Non deve mai andare in panic a sua volta: ogni
/// operazione che può fallire viene ignorata.
fn registra_panic(info: &std::panic::PanicHookInfo<'_>) {
    if IN_CORSO.with(|c| c.replace(true)) {
        return;
    }
    let thread = std::thread::current();
    let nome_thread = thread.name().map(str::to_string);
    let messaggio = info
        .payload_as_str()
        .unwrap_or("(messaggio non testuale)")
        .to_string();
    let posizione = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "sconosciuta".into());
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let voce = componi_voce(&VocePanic {
        orario: orario(),
        sistema: SISTEMA.get().map(String::as_str).unwrap_or("sconosciuto"),
        thread: nome_thread
            .as_deref()
            .unwrap_or("(senza nome: thread di sistema)"),
        posizione: &posizione,
        messaggio: &messaggio,
        base: indirizzo_base(),
        backtrace: &backtrace,
    });
    let percorso = scrivi_voce_errore(&voce);
    // Avviso a video solo quando il processo sta per essere abortito: il
    // runtime lo annuncia con un secondo panic, «panic in a function that
    // cannot unwind», subito dopo quello vero (vedi `avviso_di_sistema`).
    if messaggio.contains("cannot unwind") {
        let originale = PRECEDENTE.with(|p| p.take());
        let dettaglio = if originale.is_empty() {
            messaggio.as_str()
        } else {
            originale.as_str()
        };
        avviso_di_sistema(dettaglio, percorso.as_deref());
    } else {
        let breve = tronca(&messaggio, 300).to_string();
        PRECEDENTE.with(|p| p.replace(breve));
    }
    IN_CORSO.with(|c| c.set(false));
}

/// Contenuto di una voce del registro degli errori.
pub struct VocePanic<'a> {
    pub orario: String,
    pub sistema: &'a str,
    pub thread: &'a str,
    pub posizione: &'a str,
    pub messaggio: &'a str,
    pub base: Option<usize>,
    pub backtrace: &'a str,
}

/// Testo della voce, con messaggio e backtrace di misura limitata.
pub fn componi_voce(v: &VocePanic<'_>) -> String {
    let mut s = String::with_capacity(1024 + v.backtrace.len().min(MAX_BACKTRACE));
    s.push_str(&format!(
        "{INIZIO_VOCE}{} — errore interno (panic) — {} ===\n",
        env!("CARGO_PKG_VERSION"),
        v.orario
    ));
    s.push_str(&format!("Sistema: {}\n", v.sistema));
    s.push_str(&format!("Thread: {}\n", v.thread));
    s.push_str(&format!("Posizione: {}\n", v.posizione));
    s.push_str("Messaggio: ");
    s.push_str(tronca(v.messaggio, MAX_MESSAGGIO));
    if v.messaggio.len() > MAX_MESSAGGIO {
        s.push_str(" […troncato]");
    }
    s.push('\n');
    if v.messaggio.contains("cannot unwind") {
        // Seconda voce del runtime di Rust dopo un panic in un callback di
        // sistema: subito dopo il processo viene abortito.
        s.push_str(
            "Esito: panic dentro una funzione che non può propagarlo (callback di sistema): il \
             processo viene chiuso subito (abort). La voce precedente indica dove è nato.\n",
        );
    }
    if let Some(b) = v.base {
        // Con l'indirizzo di caricamento i frame senza nome si risolvono a
        // posteriori con il file .pdb della stessa versione.
        s.push_str(&format!(
            "Indirizzo di caricamento dell'eseguibile: {b:#x}\n"
        ));
    }
    s.push_str("Backtrace:\n");
    s.push_str(tronca(v.backtrace, MAX_BACKTRACE));
    if v.backtrace.len() > MAX_BACKTRACE {
        s.push_str("\n[…backtrace troncato]");
    }
    s.push_str("\n=== fine ===\n\n");
    s
}

/// Aggiunge la voce al registro degli errori, con flush e sincronizzazione
/// su disco. Se la cartella dei registri non è scrivibile, prova la cartella
/// temporanea. Restituisce il file effettivamente scritto.
fn scrivi_voce_errore(voce: &str) -> Option<PathBuf> {
    let _guardia = SCRITTURA_ERRORI
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for dir in [cartella(), cartella_temporanea()] {
        let percorso = dir.join(FILE_ERRORI);
        if let Ok(m) = fs::metadata(&percorso)
            && m.len() > LIMITE_ERRORI
        {
            let _ = fs::rename(&percorso, dir.join("registro_errori.1.log"));
        }
        let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&percorso) else {
            continue;
        };
        if f.write_all(voce.as_bytes()).is_ok() {
            let _ = f.flush();
            let _ = f.sync_all();
            return Some(percorso);
        }
    }
    None
}

/// Testo dell'ultima voce del registro degli errori, se c'è.
pub fn ultimo_errore() -> Option<String> {
    ultima_voce(&percorso_errori())
}

/// Ultima voce del registro in `percorso` (al più gli ultimi 128 kB).
pub fn ultima_voce(percorso: &Path) -> Option<String> {
    let mut f = File::open(percorso).ok()?;
    let lunghezza = f.metadata().ok()?.len();
    let da = lunghezza.saturating_sub(128 * 1024);
    f.seek(SeekFrom::Start(da)).ok()?;
    let mut byte = Vec::new();
    f.read_to_end(&mut byte).ok()?;
    let testo = String::from_utf8_lossy(&byte);
    let inizio = testo.rfind(INIZIO_VOCE)?;
    Some(testo[inizio..].trim_end().to_string())
}

/// Stato del registro degli errori rispetto all'ultima chiusura regolare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErroriNuovi {
    pub file: PathBuf,
}

/// `Some` se il registro degli errori è cresciuto (o è cambiato) dall'ultima
/// chiusura regolare dell'app: l'ultima volta OBDio si è chiuso per un errore.
pub fn errori_dall_ultima_chiusura() -> Option<ErroriNuovi> {
    errori_nuovi_in(&cartella())
}

/// Come [`errori_dall_ultima_chiusura`], nella cartella data.
pub fn errori_nuovi_in(dir: &Path) -> Option<ErroriNuovi> {
    let file = dir.join(FILE_ERRORI);
    let attuale = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    if attuale == 0 {
        return None;
    }
    let visto = fs::read_to_string(dir.join(FILE_SEGNO))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok());
    (visto != Some(attuale)).then_some(ErroriNuovi { file })
}

/// Annota che il contenuto attuale del registro degli errori è stato visto:
/// da chiamare alla chiusura regolare dell'app e quando l'utente chiude
/// l'avviso.
pub fn segna_errori_visti() {
    segna_errori_visti_in(&cartella());
}

/// Come [`segna_errori_visti`], nella cartella data.
pub fn segna_errori_visti_in(dir: &Path) {
    let attuale = fs::metadata(dir.join(FILE_ERRORI))
        .map(|m| m.len())
        .unwrap_or(0);
    let _ = fs::write(dir.join(FILE_SEGNO), attuale.to_string());
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    pub struct OsVersionInfoW {
        pub size: u32,
        pub major: u32,
        pub minor: u32,
        pub build: u32,
        pub platform: u32,
        pub csd: [u16; 128],
    }

    #[link(name = "ntdll", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn GetModuleHandleW(nome: *const u16) -> *mut core::ffi::c_void;
    }

    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn MessageBoxW(
            finestra: *mut core::ffi::c_void,
            testo: *const u16,
            titolo: *const u16,
            tipo: u32,
        ) -> i32;
    }
}

/// Versione del sistema operativo (per esempio «Windows 10.0.26100 (x86_64)»).
pub fn descrizione_sistema() -> String {
    SISTEMA.get_or_init(versione_sistema).clone()
}

/// Versione del sistema operativo, letta una volta all'installazione del
/// gestore.
fn versione_sistema() -> String {
    #[cfg(windows)]
    {
        let mut v = win::OsVersionInfoW {
            size: std::mem::size_of::<win::OsVersionInfoW>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            csd: [0; 128],
        };
        // SAFETY: `v` è una OSVERSIONINFOW valida con `size` impostato.
        let stato = unsafe { win::RtlGetVersion(&mut v) };
        if stato == 0 {
            return format!(
                "Windows {}.{}.{} ({})",
                v.major,
                v.minor,
                v.build,
                std::env::consts::ARCH
            );
        }
    }
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
}

/// Indirizzo a cui è caricato l'eseguibile (serve a risolvere i frame del
/// backtrace con il file .pdb, che non viene distribuito).
fn indirizzo_base() -> Option<usize> {
    #[cfg(windows)]
    {
        // SAFETY: con un puntatore nullo restituisce il modulo del processo.
        let h = unsafe { win::GetModuleHandleW(std::ptr::null()) };
        (!h.is_null()).then_some(h as usize)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Finestra di sistema che avvisa dell'errore, solo su Windows e solo quando
/// l'abort è certo: un panic dentro una funzione che non può propagarlo (i
/// callback WinRT del Bluetooth), che il runtime annuncia con il secondo
/// panic «cannot unwind». Il registro è già scritto e sincronizzato: anche se
/// la finestra non comparisse, non si perde nulla.
///
/// `MessageBoxW` da un thread qualunque è ammessa e crea un proprio ciclo di
/// messaggi; il thread in panic non tiene lock di OBDio (quello del registro
/// è già rilasciato), quindi l'attesa del clic non blocca nulla che serva a
/// chiudere. Non la si mostra per i panic che si possono recuperare (thread
/// di lavoro, vedi `worker`). Se l'abort nascesse sul thread dell'interfaccia
/// il ciclo di messaggi della finestra potrebbe rientrare in winit e causare
/// un secondo panic: il caso peggiore è che la finestra non compaia, con il
/// registro già sul disco. Con `OBDIO_SENZA_AVVISO` impostata non compare
/// (verifiche automatiche).
fn avviso_di_sistema(messaggio: &str, file: Option<&Path>) {
    #[cfg(windows)]
    {
        if std::env::var_os("OBDIO_SENZA_AVVISO").is_some() {
            return;
        }
        let dove = file
            .map(|f| f.display().to_string())
            .unwrap_or_else(|| "(registro non scrivibile)".into());
        let testo = format!(
            "OBDio ha avuto un errore interno in un componente di sistema (per esempio il \
             Bluetooth) e potrebbe chiudersi.\n\nDettaglio: {}\n\nRegistro degli errori:\n{dove}",
            tronca(messaggio, 300)
        );
        let a_utf16 = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let testo = a_utf16(&testo);
        let titolo = a_utf16("OBDio — errore interno");
        // MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST
        const TIPO: u32 = 0x0000_0010 | 0x0001_0000 | 0x0004_0000;
        // SAFETY: stringhe UTF-16 terminate da zero, finestra madre nulla.
        unsafe {
            win::MessageBoxW(std::ptr::null_mut(), testo.as_ptr(), titolo.as_ptr(), TIPO);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (messaggio, file);
    }
}

// ---------------------------------------------------------------------------
// Passi di collegamento
// ---------------------------------------------------------------------------

/// Aggiunge una riga al registro dei collegamenti (con orario al
/// millisecondo) e la scrive subito sul file. Non va mai in panic: è chiamata
/// anche dai thread di sistema (tramite il registratore di `log`).
pub fn riga_collegamento(testo: &str) {
    scrivi_collegamento_in(&cartella(), &format!("{}  {testo}\n", orario()));
}

fn scrivi_collegamento_in(dir: &Path, riga: &str) {
    let mut aperto = COLLEGAMENTO.lock().unwrap_or_else(PoisonError::into_inner);
    let percorso = dir.join(FILE_COLLEGAMENTO);
    // Il file aperto è di un'altra cartella (verifiche) o è troppo grande.
    let da_chiudere = aperto.as_ref().is_some_and(|(p, f)| {
        *p != percorso || f.metadata().is_ok_and(|m| m.len() > LIMITE_COLLEGAMENTO)
    });
    if da_chiudere {
        // Il file viene chiuso qui: su Windows un file aperto non si rinomina.
        let vecchio = aperto.take().map(|(p, _)| p);
        if vecchio.as_ref() == Some(&percorso) {
            let _ = fs::rename(&percorso, dir.join(FILE_COLLEGAMENTO_PRECEDENTE));
        }
    }
    if aperto.is_none()
        && let Ok(f) = OpenOptions::new().create(true).append(true).open(&percorso)
    {
        *aperto = Some((percorso, f));
    }
    if let Some((_, f)) = aperto.as_mut() {
        // `File` non ha buffer: la riga va al sistema con questa chiamata.
        if f.write_all(riga.as_bytes()).is_err() {
            *aperto = None;
        } else {
            let _ = f.flush();
        }
    }
}

/// Funzione che riceve una copia di ogni passo, per mostrarlo nell'app.
pub type Eco = Arc<dyn Fn(String) + Send + Sync>;

/// Diario di un collegamento: ogni passo va nel registro su disco e, se c'è,
/// all'eco (il registro degli scambi nell'app).
#[derive(Clone, Default)]
pub struct Diario {
    su_file: bool,
    eco: Option<Eco>,
}

impl Diario {
    /// Diario che scrive nel registro dei collegamenti su disco.
    pub fn su_file() -> Self {
        Diario {
            su_file: true,
            eco: None,
        }
    }

    /// Diario muto (nessun file, nessuna eco).
    pub fn muto() -> Self {
        Diario::default()
    }

    /// Aggiunge l'eco dei passi.
    pub fn con_eco(mut self, eco: Eco) -> Self {
        self.eco = Some(eco);
        self
    }

    /// Annota un passo.
    pub fn passo(&self, testo: impl AsRef<str>) {
        let testo = testo.as_ref();
        if self.su_file {
            riga_collegamento(testo);
        }
        if let Some(eco) = &self.eco {
            let ora = chrono::Local::now().format("%H:%M:%S%.3f");
            eco(format!("{ora}  {testo}"));
        }
    }
}

/// Registratore per il crate `log`: inoltra al registro dei collegamenti i
/// messaggi di btleplug (avvisi e dettagli dei passi WinRT). Non va mai in
/// panic: può essere chiamato dai callback WinRT.
struct RegistratoreBle;

impl log::Log for RegistratoreBle {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.target().starts_with("btleplug") && m.level() <= log::Level::Debug
    }

    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            // Chiamata anche dai callback WinRT, dove un panic abortirebbe il
            // processo: qualunque imprevisto viene trattenuto qui.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let testo = format!("[btleplug {}] {}", r.level(), r.args());
                riga_collegamento(tronca(&testo, 2048));
            }));
        }
    }

    fn flush(&self) {}
}

/// Inoltra i messaggi di btleplug al registro dei collegamenti. Da chiamare
/// all'avvio (non su Android, dove il registratore è quello di sistema).
pub fn inoltra_log_btleplug() {
    static REGISTRATORE: RegistratoreBle = RegistratoreBle;
    if log::set_logger(&REGISTRATORE).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cartella_prova(nome: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("obdio_registri_{nome}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn voce_completa_e_limitata() {
        let lungo = "x".repeat(10_000);
        let bt = "frame\n".repeat(20_000);
        let v = componi_voce(&VocePanic {
            orario: "2026-11-22 10:00:00.000".into(),
            sistema: "Windows 10.0.26100 (x86_64)",
            thread: "obdio-lavoro",
            posizione: "src/worker.rs:1:1",
            messaggio: &lungo,
            base: Some(0x7ff6_0000_0000),
            backtrace: &bt,
        });
        assert!(v.starts_with(INIZIO_VOCE));
        assert!(v.contains(env!("CARGO_PKG_VERSION")));
        assert!(v.contains("Thread: obdio-lavoro"));
        assert!(v.contains("Posizione: src/worker.rs:1:1"));
        assert!(v.contains("0x7ff600000000"));
        assert!(v.contains("[…troncato]") && v.contains("[…backtrace troncato]"));
        assert!(v.len() < MAX_MESSAGGIO + MAX_BACKTRACE + 1024);
        assert!(v.ends_with("=== fine ===\n\n"));
    }

    #[test]
    fn tronca_rispetta_i_caratteri() {
        assert_eq!(tronca("àèì", 3), "à");
        assert_eq!(tronca("abc", 10), "abc");
    }

    #[test]
    fn ultima_voce_e_avviso_dopo_un_errore() {
        let dir = cartella_prova("voci");
        let file = dir.join(FILE_ERRORI);
        assert_eq!(errori_nuovi_in(&dir), None);
        fs::write(
            &file,
            "=== OBDio 1 — a ===\nuno\n\n=== OBDio 1 — b ===\ndue\n",
        )
        .unwrap();
        assert_eq!(ultima_voce(&file).unwrap(), "=== OBDio 1 — b ===\ndue");
        // Mai una chiusura regolare dopo l'errore: avviso.
        assert!(errori_nuovi_in(&dir).is_some());
        segna_errori_visti_in(&dir);
        assert_eq!(errori_nuovi_in(&dir), None);
        // Una voce nuova dopo l'ultima chiusura regolare: di nuovo l'avviso.
        let mut f = OpenOptions::new().append(true).open(&file).unwrap();
        f.write_all(b"=== OBDio 1 - c ===\ntre\n").unwrap();
        assert!(errori_nuovi_in(&dir).is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diario_con_eco_e_rotazione() {
        let dir = cartella_prova("collegamento");
        // Rotazione: oltre il limite il file passa a `.1`.
        let riga = format!("{}\n", "y".repeat(1023));
        for _ in 0..(LIMITE_COLLEGAMENTO / 1024 + 4) {
            scrivi_collegamento_in(&dir, &riga);
        }
        let precedente = fs::metadata(dir.join(FILE_COLLEGAMENTO_PRECEDENTE)).unwrap();
        let attuale = fs::metadata(dir.join(FILE_COLLEGAMENTO)).unwrap();
        assert!(precedente.len() > LIMITE_COLLEGAMENTO);
        assert!(attuale.len() < LIMITE_COLLEGAMENTO);

        let viste = Arc::new(Mutex::new(Vec::new()));
        let v = viste.clone();
        let d = Diario::muto().con_eco(Arc::new(move |r| v.lock().unwrap().push(r)));
        d.passo("Collegamento: inizio");
        let viste = viste.lock().unwrap();
        assert_eq!(viste.len(), 1);
        assert!(viste[0].ends_with("  Collegamento: inizio"));
        // Chiude il file aperto nella cartella di prova prima di cancellarla.
        *COLLEGAMENTO.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let _ = fs::remove_dir_all(&dir);
    }
}
