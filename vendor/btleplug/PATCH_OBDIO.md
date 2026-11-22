# btleplug 0.13.5 con le correzioni di OBDio

Copia di [btleplug](https://github.com/deviceplug/btleplug) 0.13.5 presa dal
registro di crates.io (`~/.cargo/registry/src/*/btleplug-0.13.5`, commit
indicato in `.cargo_vcs_info.json`), con le licenze originali (`LICENSE.md`),
il `Cargo.toml` del pacchetto e tutti i sorgenti per tutte le piattaforme
(nulla è stato tolto). OBDio la usa al posto di quella di crates.io con

```toml
[patch.crates-io]
btleplug = { path = "vendor/btleplug" }
```

Al 22 novembre 2026 la 0.13.5 è l'ultima versione pubblicata (`cargo search
btleplug`, `cargo info btleplug`) e il ramo principale del repository ha
ancora le stesse chiamate `unwrap()`: aggiornare non bastava.

## Perché

Da Rust 1.81 un panic che prova a risalire fuori da una funzione
`extern "system"` non si propaga: il processo viene **abortito**, senza
messaggi né finestre. I gestori di eventi WinRT (`TypedEventHandler::new`)
sono proprio funzioni di questo tipo, chiamate da thread di sistema. Un
`unwrap()` su una chiamata WinRT che fallisce dentro uno di questi gestori fa
quindi sparire l'intera applicazione. È il sintomo segnalato con un Carista
su Windows 11: dopo circa 13 secondi di «Collegamento all'adattatore…» la
finestra di OBDio scompare.

Tutte le modifiche seguono la stessa regola: niente panic dove una chiamata
WinRT può fallire; si salta l'aggiornamento (o la voce) e, dove serve, lo si
annota con `log::debug!`/`log::warn!`. Ogni punto è marcato nel codice con un
commento `// OBDio: …`.

Le righe «prima» si riferiscono ai file originali della 0.13.5.

## Modifiche raggiungibili da un callback WinRT (abort del processo)

| File | Funzione | Prima | Dopo |
|---|---|---|---|
| `src/winrtble/ble/device.rs:78` | gestore `MaxPduSizeChanged` in `BLEDevice::new` | `max_pdu_size_changed(sender.MaxPduSize().unwrap())` | `match sender.MaxPduSize()`: con un errore l'aggiornamento dell'MTU viene saltato (`debug!`) |
| `src/winrtble/peripheral.rs:170` | `update_properties` (gestore degli annunci, durante la scansione) | `args.Advertisement().unwrap()` | annuncio ignorato con `debug!` se illeggibile |
| `src/winrtble/peripheral.rs:182` | `update_properties` | `manufacturer_data.Size().unwrap() > 0` | `Size().unwrap_or(0) > 0` |
| `src/winrtble/peripheral.rs:188-189` | `update_properties` | `d.CompanyId().unwrap()`, `d.Data().unwrap()` in un `map` | `filter_map` con `.ok()?`: voce illeggibile saltata |
| `src/winrtble/peripheral.rs:220, 227` | `update_properties` (nome completo/breve) | `section.Data().unwrap()` | `if let Ok(data)` / `.ok().and_then(…)`: nessun nome da quella sezione |
| `src/winrtble/peripheral.rs:179, 184, 254, 263, 267, 291, 304, 323, 337, 342` | `update_properties` | `RwLock::read()/write().unwrap()` | `read_lock()`/`write_lock()` (nuove, in cima al file): `unwrap_or_else(PoisonError::into_inner)` |
| `src/winrtble/utils.rs:54-57` | `to_uuid` (usata da `update_properties`) | `Uuid::from_str(&format!("{:?}", guid)).unwrap()` | `Uuid::from_fields(data1, data2, data3, &data4)`: diretta e senza errori possibili |
| `src/winrtble/utils.rs:59-65` | `to_vec` (usata da `update_properties`) | tre `unwrap()` su `DataReader` | vettore vuoto (con `debug!`) se WinRT non legge il buffer; la variante `try_to_vec` restituisce l'errore |
| `src/winrtble/ble/watcher.rs:154` | gestore `Received` con filtro di servizi | `matching_devices.lock().unwrap()` | `unwrap_or_else(PoisonError::into_inner)` (OBDio non usa filtri, ma il gestore è lo stesso) |

## Modifiche nel codice chiamato dal thread di OBDio

Qui un panic non abortisce il processo ma uccide il thread che chiama (il
thread di lavoro di OBDio, che ora lo intercetta: vedi `src/worker.rs`).

| File | Funzione | Prima | Dopo |
|---|---|---|---|
| `src/winrtble/ble/device.rs:74` | `BLEDevice::new` | `max_pdu_size_changed(gatt_session.MaxPduSize().unwrap())` | `match`: con un errore resta l'MTU predefinita (`debug!`) |
| `src/winrtble/peripheral.rs:559` | `discover_services` | `utils::to_uuid(&service.Uuid().unwrap())` | servizio saltato con `warn!` |
| `src/winrtble/peripheral.rs:149-165, 419, 528, 711` | `derive_properties`, `Display`, `connect`, `read_rssi` | `RwLock::read()/write().unwrap()` | `read_lock()`/`write_lock()` |
| `src/winrtble/ble/characteristic.rs:198` | `BLECharacteristic::uuid` | `Uuid().unwrap()` | `Uuid().unwrap_or_default()` (UUID nullo, che nessun profilo usa) |
| `src/winrtble/ble/characteristic.rs:204` | `to_characteristic` | `CharacteristicProperties().unwrap()` | `unwrap_or_default()` (nessuna proprietà: la caratteristica non viene usata) |
| `src/winrtble/ble/descriptor.rs:37` | `BLEDescriptor::uuid` | `Uuid().unwrap()` | `Uuid().unwrap_or_default()` |

## Già senza panic (controllati, non modificati)

- `ble/characteristic.rs`, gestore `ValueChanged` delle notifiche: usa `?`,
  cioè restituisce l'errore a WinRT.
- `ble/device.rs`, gestore `ConnectionStatusChanged`: usa `.ok()`.
- `adapter.rs`, gestore `StateChanged` della radio e gestore degli annunci in
  `start_scan`: usano `?`.
- `common/adapter_manager.rs` (`emit`, `peripheral_mut`): nessun panic.
- Fuori da btleplug, `windows-future` 0.3.2 (`future.rs:75`) fa
  `shared_waker.lock().unwrap()` nel gestore `Completed` di ogni operazione
  asincrona WinRT: il lock non può essere avvelenato (sotto lock c'è solo il
  `clone_from` di un `Waker`), quindi non è stato toccato.

## Verifiche

- `cargo test --manifest-path vendor/btleplug/Cargo.toml --lib` (dalla
  cartella di OBDio, con `--target-dir target/btleplug-test`): passano le
  prove originali più `obdio_to_uuid_matches_string_conversion` e
  `obdio_to_vec_reads_winrt_buffers` (un vero `IBuffer` WinRT, nessun
  hardware).
- `tests/btleplug_senza_panic.rs` in OBDio fallisce se nei file Windows di
  questa copia ricompare `.unwrap()`, `.expect(`, `panic!` & co. fuori dai
  test.

## Come riapplicare su una versione nuova

1. Copiare la nuova versione da `~/.cargo/registry/src/*/btleplug-<x.y.z>`
   in `vendor/btleplug` (togliendo solo `.cargo-ok`).
2. Rieseguire `cargo test --test btleplug_senza_panic`: elenca file e riga di
   ogni chiamata da correggere; per ognuna seguire le tabelle sopra.
3. Riaggiungere le due prove di `utils.rs` e questo file.

Le correzioni sono piccole e indipendenti da OBDio: si possono proporre a
monte così come sono.
