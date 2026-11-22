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

## Finestre di sistema per i file

- Importa ed esporta (sessioni e cruscotti) aprono la finestra di sistema per
  aprire e salvare, con filtri in italiano e l'ultima cartella ricordata per
  ogni scopo; la finestra gira in un thread a parte e l'app non si ferma. Su
  Android resta la finestra interna. Non ancora provata a mano.
