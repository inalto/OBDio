# OBDio su Android — stato e percorso di compilazione

Fase 7 del piano (`docs/PIANO.md`). **Non esiste un APK**: in questo progetto
non ci sono SDK, NDK né un telefono. Il codice è predisposto; nulla di ciò che
riguarda Android è stato compilato, installato o provato su un dispositivo.

Fonti: `docs/RICERCA.md`, sezione 6, che rimanda all'esempio ufficiale
[`hello_android`](https://github.com/emilk/egui/tree/main/examples/hello_android)
di egui. Le affermazioni su eframe 0.35, `btleplug`, `serialport`, `rfd` e
winit sono quelle verificate in quella sezione.

## Cosa è pronto nel codice

- **Libreria**: `Cargo.toml` ha `[lib] crate-type = ["cdylib", "lib"]`. Il
  binario desktop (`src/main.rs`) continua a usare `obdio::ui::esegui()`.
- **Punto d'ingresso**: `android_main(app: AndroidApp)` in `src/lib.rs`,
  attivo solo con `target_os = "android"`, `#[unsafe(no_mangle)]`, con
  `NativeOptions { android_app: Some(app), .. }`. Inizializza `android_logger`
  e fissa la cartella dei dati su `app.internal_data_path()`.
- **App condivisa**: `ui::crea_app(cc)` costruisce l'app ed è usata sia da
  `ui::esegui()` (desktop) sia da `android_main`.
- **Dipendenze solo Android** (`[target.'cfg(target_os = "android")'.dependencies]`):
  `eframe` con la funzione `android-game-activity` (la `NativeActivity` non
  gestisce i metodi di immissione, quindi niente campi di testo), `android-activity`
  (tipo `AndroidApp`), `android_logger`, `log`. Sulla build per Windows non
  cambiano nulla; `Cargo.lock` può guadagnare voci.
- **Seriale solo desktop**: `serialport` è sotto
  `cfg(not(target_os = "android"))`. Su Android `transport/serial.rs` è
  sostituito da `transport/serial_android.rs` (stessa interfaccia, nessuna
  porta), il comando `ElencaSeriali` non viene inviato né eseguito e la pagina
  Connessione mostra «Su Android il collegamento seriale non è disponibile: usa
  il Bluetooth LE».
- **Modulo `piattaforma`** (`src/piattaforma.rs`): `SERIALE_DISPONIBILE`,
  `APRI_CARTELLA_DISPONIBILE`, apertura della cartella (Esplora risorse su
  Windows, nulla altrove), cartella dei Documenti (`USERPROFILE` solo su
  Windows, con ripiego neutro).
- **Cartella dei dati**: `config::imposta_cartella_base(..)` (una sola volta,
  `OnceLock`). Se fissata, `config::cartella_dati` e
  `recording::cartella_sessioni` la usano; altrimenti il comportamento
  desktop è invariato (accanto all'eseguibile, poi `Documenti\OBDio`).
- **BLE**: non ristrutturato. Un commento `// ANDROID:` in
  `src/transport/ble.rs`, prima di `Manager::new()`, indica dove andrà
  `btleplug::platform::init(&mut env)`. Nessun codice JNI.

## Cosa NON è fatto

- Compilazione per Android e APK. I target Rust Android (solo libreria standard)
  risultano installati, ma `cargo check --lib --target aarch64-linux-android`
  si ferma nella compilazione del codice C di una dipendenza (`cc-rs` non trova
  `clang++`: manca l'NDK). Quindi il codice `cfg(target_os = "android")`
  (`android_main`, `serial_android.rs`) non è mai stato compilato.
- Progetto Gradle con `GameActivity` (manifesto, tema, icona, firma).
- Parte Java di `btleplug` (`jni-utils`) e regole di conservazione R8/ProGuard
  per le sue classi; chiamata di `btleplug::platform::init`.
- Permessi a runtime `BLUETOOTH_SCAN` e `BLUETOOTH_CONNECT` (e relativa
  richiesta all'utente); dichiarazione nel manifesto.
- Bluetooth Classic (SPP/ELM327 classici): richiede l'API Java, non
  `serialport`. Gli adattatori solo classici non funzioneranno.
- Accesso ai file con lo *scoped storage*: oggi i dati stanno nella cartella
  privata dell'app; importazione ed esportazione verso cartelle condivise
  richiedono il selettore di sistema (Storage Access Framework). `rfd` non
  supporta Android: sul desktop si usano le finestre di sistema, su Android
  resta la finestra di scelta file interna (`scelta_file`, selezionata alla
  compilazione da `piattaforma::DIALOGO_DI_SISTEMA`), che parte dalla
  cartella corrente, non da Documenti.
- Inserti di sicurezza dello schermo (notch, barre di sistema): winit non li
  riporta ancora; l'interfaccia potrebbe finire sotto di essi.
- Tastiera a schermo: il comportamento dei campi di testo egui con
  `GameActivity` non è verificato.
- La persistenza di eframe è disattivata su Android; le impostazioni di OBDio
  usano i propri file JSON nella cartella dei dati.
- Dimensioni, tocco e layout della UI pensata per finestre da 980×620 e oltre.

## Percorso di compilazione consigliato

Livello API minimo: **24** (richiesto da `btleplug` su Android).

Strumenti: Android SDK e NDK, Rust con i target Android, `cargo-ndk`, Gradle.

Tutti i comandi seguenti sono **non provati in questo progetto**.

1. Target Rust (non provato in questo progetto):
   `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`
2. Installare `cargo-ndk` (non provato in questo progetto):
   `cargo install cargo-ndk`
3. Compilare la libreria condivisa (non provato in questo progetto):
   `cargo ndk -t arm64-v8a -o app/src/main/jniLibs build --release`
   (con `ANDROID_NDK_HOME` impostata; il nome della libreria è `libobdio.so`).
4. Creare un progetto Gradle con `GameActivity` (libreria
   `androidx.games:games-activity`), `minSdk 24`, `android.app.lib_name`
   uguale a `obdio` nel manifesto (non provato in questo progetto).
5. Aggiungere la parte Java di `btleplug`, le regole R8 e i permessi nel
   manifesto (`BLUETOOTH_SCAN`, `BLUETOOTH_CONNECT`); chiamare
   `btleplug::platform::init` prima di creare il `Manager` (non provato in
   questo progetto).
6. `./gradlew assembleDebug` e installazione con
   `adb install app/build/outputs/apk/debug/app-debug.apk` (non provato in
   questo progetto).

Prova rapida senza campi di testo: `cargo-apk` con `NativeActivity`
(`cargo install cargo-apk`, poi `cargo apk run --lib`), con la funzione
`android-native-activity` al posto di `android-game-activity`. Non adatta
all'app completa, perché la `NativeActivity` non gestisce l'immissione di
testo (non provato in questo progetto).

## Da provare per primo su un telefono

1. L'app si avvia, mostra la pagina Connessione e non si chiude.
2. La demo funziona (nessun Bluetooth coinvolto): cruscotto, sensori, grafici.
3. Rotazione dello schermo e ripresa dopo che l'app va in secondo piano.
4. Dimensioni del testo e dei bottoni al tocco; elementi sotto notch o barre.
5. Tastiera a schermo nei campi di testo (impostazioni MQTT, nome sessione).
6. Salvataggio di una sessione e sua riapertura: i file finiscono in
   `internal_data_path()`.
7. Richiesta dei permessi Bluetooth, ricerca BLE, collegamento a un
   adattatore e lettura dei dati.
8. Comportamento a schermo spento durante una registrazione.
9. Esportazione verso una cartella condivisa (richiede lavoro sul
   selettore di file).
