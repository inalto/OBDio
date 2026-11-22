# OBDio — piano di evoluzione

Obiettivo: da lettore P0420 a strumento da officina completo. Leggere e
cancellare i codici, vedere tutti i sensori, un cruscotto di strumenti
personalizzabile, registrare, importare, esportare e rivivere una sessione,
pubblicare i dati su MQTT. Base pronta per Android.

Questo piano nasce dalla ricerca riassunta in [RICERCA.md](RICERCA.md) e segue
la specifica di design in [DESIGN.md](DESIGN.md).

## Scelte di fondo

| Tema | Scelta | Motivo |
|---|---|---|
| Linguaggio e GUI | Restano Rust ed egui/eframe 0.35 | eframe ha un percorso Android ufficiale; un solo codice per PC e telefono |
| Strumenti del cruscotto | Disegnati a mano con il `Painter` di egui | Nessun crate di strumenti è compatibile con egui 0.35 e offre un quadrante a 270° con zone |
| Descrizioni DTC | Database OBDex (9.533 codici generici, licenza CC0) incorporato | Unico insieme completo con licenza che ne consente la ridistribuzione |
| Cancellazione codici | Azione separata dai comandi di lettura, con conferma e copia di sicurezza | È l'unica operazione che modifica la centralina e non si annulla |
| Configurazione | File JSON in una cartella `configurazione` | La persistenza di eframe è disattivata su Android; i JSON servono anche per importare ed esportare |
| MQTT | `rumqttc` 0.25, senza TLS nella prima versione | Rust puro, usabile da un thread normale, riconnessione automatica |
| Finestre di scelta file | `egui-file-dialog` 0.14 (rinviata) | `rfd` non supporta Android; questa è disegnata dentro egui. Ma la 0.14 dipende da `sysinfo` 0.39, che richiede Rust 1.95: con Rust 1.93 l'importazione e l'esportazione dei cruscotti chiedono il percorso in un campo di testo, con l'elenco dei JSON della cartella |
| Riproduzione | I campioni registrati rientrano dallo stesso canale degli eventi dal vivo | Cruscotto, grafici e MQTT funzionano senza distinguere la sorgente |

## Fasi

Ogni fase termina con `cargo fmt`, `cargo clippy` senza avvisi, tutti i test
superati, il collaudo automatico dell'interfaccia in demo e un commit.

| Fase | Contenuto | Modello | Stato |
|---|---|---|---|
| 0 | Ricerca, specifica di design, questo piano | Sonnet (ricerca) | fatto |
| 1 | Database DTC completo; cancellazione codici con conferma e copia di sicurezza; simulatore aggiornato | Opus (cancellazione), Sonnet (database) | |
| 2 | Catalogo PID esteso (contachilometri, pedale, coppia, consumo…); pagina **Sensori** con tutti i parametri supportati, ricerca, min/medio/max, un grafico per sensore; grandezze calcolate (distanza parziale) | Sonnet | fatto |
| 3 | **Cruscotto** personalizzabile: tema e token, sette tipi di strumento, griglia, modo modifica, ispettore, pagine, modelli pronti, salvataggio, import/export dei cruscotti | Opus | fatto |
| 4 | Importazione ed esportazione dei dati (sessione, CSV OBDio, CSV in formato Torque, diagnosi JSON) e **riproduzione** in tempo reale con pausa, posizione e velocità | Opus | fatto |
| 5 | Pubblicazione **MQTT** in tempo reale: impostazioni, stato in linea/non in linea, un argomento per sensore più uno JSON cumulativo, discovery per Home Assistant | Sonnet | fatto |
| 6 | Rifinitura: navigazione a sei destinazioni, layout stretto per telefono, barra di stato, pagina Impostazioni | Opus | fatto |
| 7 | Base Android (libreria `cdylib`, funzioni desktop dietro `cfg`), guida alla compilazione, README, pacchetto Windows | Haiku (documenti), Sonnet (codice) | |

### Fase 1 — Codici: descrizioni e cancellazione

- 9.533 descrizioni dei codici generici (P0, P2, P3, B0, C0, U0, U3) in
  inglese; le circa 55 descrizioni italiane curate hanno la precedenza.
- I codici del costruttore (P1xxx e simili) sono riconosciuti come tali: non
  esiste un insieme aperto affidabile, in particolare per Peugeot/Citroën.
- Servizio 04 dietro un tipo di conferma dedicato: l'elenco delle richieste di
  lettura resta chiuso e verificato da un test.
- Prima di cancellare: copia della diagnosi su file. Dopo: rilettura
  automatica. Esito distinto per centralina (accettata, rifiutata con motivo).
- Il simulatore rispetta lo standard: dopo la cancellazione i codici
  permanenti restano e i monitor tornano «non completati».

### Fase 2 — Tutti i sensori

- Nuovi parametri: contachilometri (PID A6), distanza e tempo dalla
  cancellazione, pressione carburante e rail, posizioni farfalla e pedale,
  coppia, consumo istantaneo, tipo di carburante, norma OBD.
- Pagina Sensori: elenco dei soli parametri dichiarati dalla centralina,
  campo di ricerca, spunta per attivare la lettura, valore con minimo, medio e
  massimo, frequenza effettiva, e un grafico espandibile per ogni sensore.
- La «Diagnosi P0420» diventa una selezione rapida dentro Sensori.
- Distanza parziale calcolata integrando la velocità, per le auto che non
  espongono il contachilometri (prima del 2019 è la norma).

### Fase 3 — Cruscotto

Vedi DESIGN.md, sezioni 3 e 4. Punti di verifica: nessuno strumento mostra
zero quando il dato manca; un cruscotto salvato si riapre identico; un file
esportato si reimporta; la griglia si ricompone da 12 a 4 colonne.

Fatto: `src/ui/tema.rs` (token applicati a tutta l'app), `src/ui/cruscotto/`
(modello, sorgenti, strumenti, editor), lettura automatica dei parametri della
pagina visibile, collaudo con schermate in `docs/schermate/cruscotto_*.png`.
Il layout stretto esiste già nella pagina ma la finestra resta larga almeno
980 px fino alla fase 6; trascinamenti e ridimensionamenti col mouse non sono
coperti dal collaudo automatico.

### Fase 4 — Importare, esportare, rivivere

- Esportazione: sessione `.obdio.jsonl`, CSV OBDio, CSV «largo» in stile
  Torque (`Nome(unità)` per colonna), diagnosi in JSON.
- Importazione: sessione, CSV OBDio, CSV in stile Torque (colonne
  riconosciute dal nome; le altre conservate come parametri liberi).
- Riproduzione come sorgente di collegamento: barra di trasporto, velocità da
  0,5× a 10×, ripetizione, salto ai marcatori. Fascia arancione sempre
  visibile. Registrazione e cancellazione codici disattivate.

Fatto: `src/riproduzione.rs` (motore puro della riproduzione, verificato con
passi di lunghezza casuale), `src/scambio.rs` (importazione ed esportazione,
regole in [FORMATO_CSV.md](FORMATO_CSV.md)), sorgente `Riproduzione` del
thread di lavoro con `Origine` veicolo/demo/riproduzione fino a MQTT,
parametri liberi (`pids::registra_libero`), barra di trasporto
(`src/ui/trasporto.rs`), scheda Sessioni con importazione ed esportazione
(`src/ui/sessioni.rs`) e finestra di scelta del file condivisa con il
cruscotto (`src/ui/scelta_file.rs`). Prove d'integrazione in
`tests/riproduzione.rs`; schermata in `docs/schermate/riproduzione.png`.
Non verificato: un registro prodotto dall'app Torque (solo file scritti da
OBDio o a mano) e il trascinamento del cursore col mouse.

### Fase 5 — MQTT

- Argomenti: `<prefisso>/<veicolo>/stato` (trattenuto, con ultima volontà
  `offline`), `<prefisso>/<veicolo>/sensori/<chiave>` (valore),
  `<prefisso>/<veicolo>/istantanea` (JSON con tutti i valori),
  `<prefisso>/<veicolo>/diagnosi` (JSON a ogni lettura).
- Discovery per Home Assistant, attivabile.
- Limitazione della frequenza per sensore; la coda non blocca mai il polling.
- Il VIN non viene mai pubblicato. La password non viene scritta su disco
  salvo scelta esplicita.

### Fase 6 — Rifinitura e telefono

Vedi DESIGN.md, sezioni 1 e 2.

Fatto: barra laterale (largo) o in basso (stretto) con icone disegnate
(`src/ui/navigazione.rs`, `src/ui/icone.rs`), barra di stato con titolo,
collegamento, registrazione e MQTT, elementi comuni in `src/ui/componenti.rs`
(schede, pulsanti per ruolo, righe etichetta/valore, stati vuoti). Finestra
minima 360 × 560; ogni pagina ha un layout stretto (vedi DESIGN.md, nota della
sezione 2). Ctrl+1…6, conferma della chiusura con il cruscotto non salvato,
`tema::riserva_superiore()` per Android. Il collaudo fotografa ogni pagina a
1280 × 820 e a 390 × 844 (`largo_*.png`, `stretto_*.png`); schermate scelte in
`docs/schermate/`. Non verificato: tocco su un telefono vero, gesti col dito,
scorciatoie premute a mano.

### Fase 7 — Android

Si prepara il codice (libreria con punto d'ingresso `android_main`, seriale e
finestre di sistema solo su desktop) e si documenta il percorso di
compilazione. **Non** si consegna un APK: richiede SDK e NDK Android, una
parte Java per il Bluetooth e prove su un telefono.

## Cosa resta fuori

- Codici e parametri specifici del costruttore (richiedono dati non aperti).
- Codifica, test attuatori, scrittura sulle centraline.
- TLS per MQTT (rinviato: la scelta della libreria crittografica condiziona
  la compilazione per Android).
- Mappa e GPS, cronometro di accelerazione, plugin.
