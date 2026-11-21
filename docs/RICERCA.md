# OBDio — sintesi della ricerca

Ricerca svolta su GitHub, crates.io e documentazione pubblica per decidere
come estendere l'app. Ogni voce indica la fonte; ciò che non è stato possibile
verificare è elencato in fondo.

## 1. Descrizioni dei codici guasto

| Insieme | Codici | Lingue | Licenza | Giudizio |
|---|---|---|---|---|
| [foerbsnavi/OBDex](https://github.com/foerbsnavi/OBDex) | 9.533 generici (P0, P2, P3, B0, C0, U0, U3) | inglese, tedesco | Dati **CC0-1.0** | **Adottato** |
| [Wal33D/dtc-database](https://github.com/Wal33D/dtc-database) | 12.128, con codici di 33 marche | inglese | MIT, provenienza dei testi non documentata | Non adottato: nessuna voce Peugeot/Citroën/Renault/Fiat |
| [fr3ts0n/AndrOBD](https://github.com/fr3ts0n/AndrOBD) | 3.584 | oltre 40 (l'italiano ha 10 voci) | GPL-3.0 | Non compatibile |
| [brendan-w/python-OBD](https://github.com/brendan-w/python-OBD) | 2.066 | inglese | GPL-2.0 | Non compatibile |
| xinings/DTC-Database, peyo/dtc-and-vin-data | — | inglese | Nessuna licenza | Non ridistribuibili |

Conclusioni: **non esiste un insieme italiano con licenza aperta**, né un
insieme aperto dei codici Peugeot/Citroën. Le descrizioni incorporate sono
quindi in inglese, con il sottoinsieme italiano curato a mano che ha la
precedenza. Il testo tedesco di OBDex è completo e potrebbe servire da base
per una traduzione italiana futura.

## 2. Cancellazione dei codici (servizio 04)

- Richiesta `04`, risposta positiva `44`; rifiuto `7F 04 22` quando le
  condizioni non sono corrette, tipicamente a motore acceso
  ([x-engineer](https://x-engineer.org/obd-diagnostic-service-mode-04-clear-reset-emission-related-diagnostic-information/),
  scheda tecnica ELM327 p. 43).
- Cancella: codici memorizzati e in sospeso, fermo immagine, dati dei test
  delle sonde, risultati Mode 06, stato dei monitor di prontezza, distanza e
  tempo con MIL accesa e dalla cancellazione.
- **Non** cancella i codici permanenti (servizio 0A): li rimuove solo la
  centralina, dopo che il monitor è passato.
- La scheda ELM327 ricorda che lo standard chiede allo strumento di far
  confermare l'operazione. python-OBD non lo fa; AndrOBD mostra un avviso e
  rilegge i codici subito dopo.
- Condizioni: quadro acceso, motore spento.

## 3. Cosa fanno gli strumenti migliori

| Strumento | Da riprendere |
|---|---|
| RealDash | Per ogni strumento: campo della scala più livelli di attenzione e critico che cambiano colore ([manuale](https://realdash.net/manuals/using_normal_warning_and_critical_levels.php)) |
| OBD Fusion | Cruscotti esportabili uno per uno; grafici multi-segnale; registrazione e riproduzione |
| DashCommand | Registri riproducibili sia nel cruscotto sia nella griglia dei dati; cruscotti che scalano a ogni schermo |
| OBD Auto Doctor | Valore attuale, minimo, medio e massimo per sensore; griglia di grafici |
| AndrOBD | Pubblicazione MQTT di un messaggio per parametro; registrazione e ricarica dei dati |
| WiCAN | Argomento di stato trattenuto con ultima volontà `offline`; istantanea JSON cumulativa ([documentazione](https://docs.meatpi.com/wican-obd/reference/mqtt-messages/)) |
| canvas-gauges | Modello di configurazione di un quadrante: angolo, tacche principali e secondarie, fasce colorate, riquadro del valore |

Sviluppo dei quadranti: 270° è lo standard degli strumenti automobilistici a
corsa completa. Per soglie e marcatore di picco non esiste una fonte
normativa: sono scelte di design di questo progetto.

## 4. MQTT

- Libreria: [`rumqttc`](https://github.com/bytebeamio/rumqtt) 0.25.1
  (Apache-2.0, Rust puro, versione minima di Rust 1.64). Il ciclo di
  connessione va fatto avanzare di continuo; in caso di errore basta
  continuare il ciclo per riconnettersi, aggiungendo una pausa.
- Home Assistant: discovery su
  `homeassistant/<componente>/<id>/config`, con `state_topic`,
  `unit_of_measurement`, `device_class`, `availability_topic`, `unique_id`,
  `device` ([documentazione](https://www.home-assistant.io/integrations/mqtt/)).
  La documentazione sconsiglia di trattenere i messaggi di stato.
- TLS: le funzioni predefinite di `rumqttc` portano con sé `aws-lc-rs`, che
  compila codice C; per Android servirebbe la variante con `ring`. Rinviato.

## 5. Formati di scambio

Non esiste uno standard. Il CSV di Torque (`trackLog.csv`) è il più letto da
strumenti terzi: una colonna per parametro, intestazioni `Nome(unità)`, prime
colonne `GPS Time` e `Device Time`, `-` per valore assente. Di OBD Fusion,
AndrOBD e RealDash non è documentato il formato delle colonne.

## 6. Android

- eframe 0.35 ha le funzioni `android-native-activity` e
  `android-game-activity`; esempio ufficiale
  [`hello_android`](https://github.com/emilk/egui/tree/main/examples/hello_android)
  con `crate-type = ["cdylib", "lib"]` e punto d'ingresso `android_main`.
- Per un'app con campi di testo è indicata `android-game-activity` con Gradle
  e `cargo-ndk`: `NativeActivity` non gestisce i metodi di immissione.
- `btleplug` supporta Android ma richiede una parte Java e i permessi
  `BLUETOOTH_SCAN` e `BLUETOOTH_CONNECT` richiesti dall'app.
- `serialport` su Android non elenca le porte; il Bluetooth classico passa
  dall'API Java.
- La persistenza di eframe è disattivata su Android; `rfd` non lo supporta.
- Gli inserti di sicurezza dello schermo non sono ancora gestiti da winit.

## 7. Parametri standard aggiuntivi

Formule da [Wikipedia, «OBD-II PIDs»](https://en.wikipedia.org/wiki/OBD-II_PIDs):
contachilometri (A6, obbligatorio solo dal 2019), distanza e tempo dalla
cancellazione (31, 4E), tempo con MIL accesa (4D), pressione carburante (0A)
e rail (23), farfalla e pedale (45, 47, 49, 4A, 4C, 5A), coppia (61–63),
consumo istantaneo (5E), tipo di carburante (51), norma OBD (1C).

## Non verificato

- Formule dei PID 49, 4A, 4C dedotte da righe accorpate della tabella di
  Wikipedia.
- Quali codici di rifiuto, oltre a `22`, siano ammessi per il servizio 04.
- Che ogni centralina azzeri i risultati Mode 06 con il servizio 04.
- Funzionamento reale della tastiera a schermo di egui su Android.
- Formato delle colonne dei CSV di OBD Fusion e AndrOBD.
- Dettagli di Torque Pro e RealDash su tipi di strumento e griglia: le pagine
  consultate non li riportano.
