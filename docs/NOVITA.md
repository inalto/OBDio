# OBDio — novità dopo la prima versione

Riepilogo per fase del piano (`docs/PIANO.md`). Ogni voce riprende quanto
descritto nel [README](../README.md) e nel piano; i limiti di verifica sono
nel README, in [Stato delle verifiche](../README.md#stato-delle-verifiche).

## Fase 1 — Database DTC e cancellazione codici

- Descrizioni dei 9.533 codici generici (P0, P2, P3, B0, C0, U0, U3), in
  inglese, con circa 55 descrizioni in italiano curate a mano.
- I codici del costruttore mostrano solo il codice.
- Azione **Cancella codici guasto** (servizio 04) separata dalla lettura, con
  finestra di conferma.
- Copia della diagnosi salvata prima della cancellazione; rilettura automatica
  ed esito per ogni centralina.
- Il simulatore segue lo standard: dopo la cancellazione i codici permanenti
  restano e i monitor tornano «non completati».

## Fase 2 — Sensori e PID estesi

- Pagina **Sensori** con tutti i parametri che la centralina dichiara di
  supportare, ricerca e selezioni rapide.
- Nuovi parametri: contachilometri, pedale dell'acceleratore, coppia,
  consumo istantaneo, pressioni del carburante, tipo di carburante, norma OBD.
- Per ogni sensore: minimo, medio e massimo, frequenza effettiva e un grafico
  dedicato.
- Distanza parziale calcolata integrando la velocità, per le auto che non
  espongono il contachilometri.

## Fase 3 — Cruscotto

- Pagine di strumenti personalizzabili con sette tipi di strumento.
- Modo Modifica con griglia, ispettore, soglie di attenzione e critiche, e
  pagine modello pronte.
- Salvataggio dei cruscotti in JSON, con importazione ed esportazione.

## Fase 4 — Importazione, esportazione e riproduzione

- Esportazione in formato OBDio (sessione e CSV), in stile Torque e diagnosi
  in JSON.
- Importazione di sessioni, CSV OBDio e CSV in stile Torque, con riepilogo
  prima del salvataggio.
- Riproduzione di una sessione registrata, con barra di trasporto, velocità da
  0,5× a 10×, ripetizione e salto ai marcatori.

## Fase 5 — MQTT

- Pubblicazione dei dati su un broker MQTT, con stato in linea e ultima volontà
  `offline`.
- Un argomento per sensore più un JSON con tutti i valori e uno per la
  diagnosi.
- Discovery per Home Assistant, attivabile.

## Fase 6 — Navigazione e layout da telefono

- Sei pagine: Connessione, Diagnosi, Cruscotto, Sensori, Sessioni, Impostazioni.
- Barra laterale in finestra larga; barra in basso con layout a colonna unica
  sotto i 900 px.
- Barra di stato su ogni pagina e scorciatoie Ctrl+1…6.
- Conferma alla chiusura con modifiche al cruscotto non salvate.

## Fase 7 — Predisposizione Android

- Il crate è configurato anche come libreria (`cdylib`) con il punto d'ingresso
  `android_main`; la seriale e le finestre di sistema restano solo su desktop.
  Questa parte non è mai stata compilata per Android.
- Cartella dei dati configurabile e modulo `piattaforma` per le differenze fra
  i sistemi.
- Non è stato costruito alcun APK e il codice Android non è mai stato
  compilato: vedi [ANDROID.md](ANDROID.md).

## Server MCP per assistenti AI

- Un assistente AI che parla il protocollo MCP (per esempio Claude Code) può
  collegarsi all'app aperta, su `http://127.0.0.1:8765/mcp`, e leggere stato,
  sensori con la loro storia recente, diagnosi e sessioni registrate.
- Interruttore in cima alle **Impostazioni**; spento finché non lo si
  accende. Indicatore «MCP» nella barra di stato.
- Solo su questo computer, con un token casuale per ogni richiesta; due
  livelli, **Sola lettura** e **Lettura e azioni**.
- Ogni risposta dichiara l'origine dei dati (veicolo, demo, riproduzione); il
  VIN non esce mai e i codici guasto non si possono cancellare da qui.
- Non è stato provato con un assistente AI reale: vedi [MCP.md](MCP.md).

## Finestre di sistema per i file

- Importa ed esporta (sessioni e cruscotti) aprono la finestra di sistema per
  aprire e salvare, con filtri in italiano e l'ultima cartella ricordata per
  ogni scopo; la finestra gira in un thread a parte e l'app non si ferma. Su
  Android resta la finestra interna. Non ancora provata a mano.

## Descrizioni italiane dei codici guasto

- Tutti i 9.533 codici generici hanno ora una descrizione italiana.
- Le circa 55 descrizioni curate a mano restano prioritarie; le altre sono una traduzione automatica fatta con un glossario di officina.
- La traduzione non è stata rivista da un tecnico: la pagina Diagnosi mostra accanto il titolo inglese originale, che vale in caso di dubbio.
- Dettagli in [FONTI_DATI.md](FONTI_DATI.md).

## Aggiornamenti da GitHub

- Scheda **Aggiornamenti** nelle Impostazioni: versione installata, ultimo
  controllo con il suo esito, **Controlla ora** e l'interruttore del controllo
  automatico (acceso, al più una volta ogni 24 ore, qualche secondo dopo
  l'avvio). La scheda dice che cosa viene contattato (solo GitHub) e che
  nulla su di te o sul veicolo viene inviato.
- Una versione nuova compare come avviso nella barra di stato e come riquadro
  con le note di rilascio, mai come finestra: **Scarica e installa**, **Apri
  la pagina della versione**, **Ignora questa versione**, **Più tardi**.
- Su Windows x64 l'installazione scarica l'eseguibile, ne verifica dimensione
  e somma SHA-256 e lo mette al posto di quello in uso con ritorno indietro in
  caso di errore; la versione precedente si elimina al prossimo avvio. Poi
  **Riavvia ora** o **Al prossimo avvio**. Rifiutata durante una
  registrazione o una cancellazione dei codici e dalle build di sviluppo.
- La verifica non protegge da un account GitHub compromesso e l'eseguibile non
  è firmato. Mai provato con un rilascio vero: vedi
  [AGGIORNAMENTI.md](AGGIORNAMENTI.md), che descrive anche i nomi dei file che
  un rilascio deve avere.
- Su Android il controllo non esiste: l'app si aggiorna con il pacchetto.

## 1.0.1 — Chiusure improvvise e diagnosi dei collegamenti

Segnalazione: con la 1.0.0, su un altro PC Windows 11, collegando un Carista
via Bluetooth LE la finestra spariva dopo circa 13 secondi, senza messaggi.
Non è stato possibile riprodurla qui (manca l'adattatore): la correzione
**non è confermata sul Carista**.

- La libreria Bluetooth (btleplug 0.13.5) è inclusa nel progetto con le
  correzioni di OBDio: niente più `unwrap()` dentro i gestori di eventi di
  Windows, dove un errore chiude l'app all'istante. È la causa sospettata
  (dettagli in `vendor/btleplug/PATCH_OBDIO.md`).
- **Registro degli errori** (`registri\registro_errori.log`): ogni errore
  interno con versione, sistema, punto del codice e backtrace, scritto sul
  disco anche quando l'app sta per essere chiusa. Al riavvio compare l'avviso
  «L'ultima volta OBDio si è chiuso per un errore».
- **Registro dei collegamenti** (`registri\registro_collegamento.log`): ogni
  passo di ogni collegamento con l'orario al millisecondo, compresi i servizi
  GATT e gli scambi ELM327 iniziali (mai il VIN). Gli stessi passi compaiono
  dal vivo nel registro degli scambi della pagina Connessione.
- Nuova scheda **Registri** nelle Impostazioni: percorsi, **Apri la
  cartella**, **Copia l'ultimo errore**.
- Un errore interno durante un collegamento o una lettura non lascia più
  l'app in attesa per sempre: il collegamento si chiude, la registrazione in
  corso viene salvata e si può riprovare. Se il thread di lavoro si fermasse
  comunque, l'app lo dice e ne avvia uno nuovo.
- Collegamento Bluetooth più robusto: tempo massimo e messaggio propri per
  ogni passo (connessione radio, servizi GATT, flusso e attivazione delle
  notifiche), un nuovo tentativo automatico dopo un errore transitorio, la
  scansione fermata sempre prima di collegarsi. Un adattatore che rifiuta
  l'accesso GATT viene riconosciuto e l'app suggerisce di associarlo in
  Windows o di usare la porta seriale.
- **Diagnosi Bluetooth da terminale**: `OBDio.exe --diagnosi-ble [nome]` fa
  scansione, connessione ed elenco dei servizi GATT, senza inviare comandi né
  scrivere sull'adattatore, e salva un resoconto accanto all'eseguibile.
  Istruzioni in [DIAGNOSI_PROBLEMI.md](DIAGNOSI_PROBLEMI.md).

## 1.0.2 — Causa trovata della chiusura al collegamento Bluetooth

I registri della 1.0.1 raccolti sul PC della segnalazione mostrano la causa
vera: il collegamento al Carista riesce (profilo FFF0, notifiche attive in
meno di 3 secondi), ma il primo comando inviato all'adattatore andava in
panic con «there is no reactor running». Il tempo massimo della scrittura
BLE (`tokio::time::timeout`) veniva creato dal thread di lavoro, fuori dal
runtime asincrono.

- Nella 1.0.0 lo stesso errore si ripeteva nello scollegamento durante il
  panic: un doppio panic, cioè l'abort del processo. Da qui la finestra che
  spariva. Nella 1.0.1 l'errore veniva intercettato e mostrato come errore
  interno, ma il collegamento restava impossibile.
- Corretto: il tempo massimo viene creato dentro il runtime
  (`blocca_con_limite` in `src/transport/ble.rs`), con una prova di
  regressione che non richiede hardware.
- L'ipotesi della 1.0.1 (un `unwrap()` di btleplug in un gestore WinRT) non
  era la causa di questa segnalazione; le correzioni in `vendor/btleplug`
  restano come protezione.
- Non ancora provato sull'adattatore: lo scambio dei comandi ELM327 via BLE
  dopo questa correzione.

## 1.1.0 — Linea K (ISO 9141-2 e KWP2000)

Ricerca in [RICERCA_K_LINE.md](RICERCA_K_LINE.md). Con un ELM327 la linea K
arriva all'app come righe di testo in un altro formato; OBDio ne gestiva già
una parte.

- Il VIN si legge anche fuori da CAN, dove arriva in più messaggi numerati.
  Resta escluso da registri, esportazioni e MQTT.
- I messaggi dell'inizializzazione riuscita (`BUS INIT: ...OK`) non sono più
  scambiati per errori; quella fallita suggerisce di accendere il quadro.
- Una riga con checksum errato (`<DATA ERROR`) viene scartata senza perdere
  le altre.
- Dalla seconda lettura di un dato corrente la richiesta dice all'adattatore
  quante risposte aspettare, per non perdere tempo su un bus lento. Se
  l'adattatore non conosce la funzione, non viene più usata.
- La scheda del collegamento avvisa che sulla linea K i valori si aggiornano
  più lentamente.
- Provato solo con veicoli simulati (ISO 9141-2 e KWP2000). **Mai provato su
  un veicolo a linea K**, e non è confermato che l'adattatore Carista la
  supporti. Mancano ancora il servizio 06 fuori da CAN e la scelta manuale
  del protocollo.

## Td5 (Land Rover) — non ancora in un rilascio

Ricerca e stato in [RICERCA_TD5.md](RICERCA_TD5.md). **Mai provata su un
adattatore o su un veicolo**: tutto è verificato con un OBDLink e una
centralina Td5 simulati.

- Nella pagina Connessione si sceglie il tipo di veicolo: «OBD-II standard»
  (come prima) o «Land Rover Td5 (OBDLink CX/LX)». La scelta resta salvata.
- Con il Td5 il collegamento BLE (OBDLink CX) o seriale (OBDLink LX) usa i
  comandi ST del chip STN: tre modi di far passare i messaggi senza
  intestazione, provati in ordine; ogni comando e ogni risposta vanno nel
  registro dei collegamenti.
- Accesso di sicurezza con l'algoritmo di td5keygen (licenza BSD, in
  [LICENSE-THIRD-PARTY.md](../LICENSE-THIRD-PARTY.md)); sessione tenuta viva
  dall'app e riaperta da sola, una volta, se cade.
- Dati in tempo reale su cruscotto, sensori, grafici, registrazione, CSV,
  riproduzione, MQTT e MCP: giri, velocità, temperature del liquido,
  dell'aria e del gasolio, pressione del collettore e ambiente, tensione,
  tracce del pedale, EGR, wastegate, correzione degli iniettori, interruttori.
- Diagnosi con i codici guasto del Td5 (nomi dalla tabella di Td5OpenDiag),
  tipo di centralina, variante della mappa e tensione. Sola lettura: niente
  cancellazione dei codici né prove degli attuatori, per scelta.
- Un adattatore di tipo ELM327 viene riconosciuto e rifiutato con una
  spiegazione.
- «Avvia la demo Td5» per vedere la modalità senza adattatore.
