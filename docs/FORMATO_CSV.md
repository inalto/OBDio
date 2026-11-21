# Formato del CSV esportato da OBDio

Il file è scritto accanto alla sessione, con lo stesso nome e l'estensione
`.csv`.

## Caratteristiche generali

- Codifica **UTF-8 con BOM** (il BOM serve a Excel per riconoscere la codifica).
- Separatore di campo: **virgola**. Fine riga: CR LF.
- Separatore decimale: **punto**.
- I campi che contengono virgole, virgolette o ritorni a capo sono racchiusi
  tra virgolette doppie; le virgolette interne sono raddoppiate.
- Formato «lungo»: **una riga per ogni campione di ogni parametro**, in ordine
  di tempo. I marcatori sono righe dello stesso file.
- La prima riga è l'intestazione.
- Il VIN non compare mai.

In Excel con impostazioni italiane, apri il file da *Dati → Da testo/CSV*
indicando virgola come delimitatore, così i decimali con il punto vengono
interpretati correttamente.

## Colonne

| Colonna | Contenuto |
|---|---|
| `timestamp` | Data e ora locali del singolo campione, ISO 8601 con millisecondi e fuso orario, es. `2026-11-21T19:37:41.512+01:00` |
| `tempo_s` | Secondi trascorsi dall'inizio della registrazione, tre decimali |
| `ecu` | Centralina che ha risposto (es. `7E8`). Vuoto se nessuna ha risposto e per i marcatori |
| `parametro` | Chiave stabile del parametro (vedi sotto), oppure `marcatore` |
| `descrizione` | Nome leggibile del parametro; per i marcatori, il testo del marcatore |
| `valore` | Valore numerico. **Vuoto** quando lo stato non è `ok` e per i marcatori |
| `unita` | Unità di misura del valore |
| `stato` | Esito della lettura (vedi sotto) |
| `origine` | `veicolo` per dati letti da una centralina, `demo_sintetico` per dati simulati |

## Valori di `stato`

| Stato | Significato |
|---|---|
| `ok` | Valore letto e decodificato |
| `non_supportato` | La centralina rifiuta o dichiara non applicabile il parametro |
| `risposta_mancante` | Parametro dichiarato dalla centralina, ma senza risposta a questa richiesta |
| `errore_comunicazione` | Timeout, errore di bus o risposta non interpretabile |
| `marcatore` | La riga è un marcatore inserito dall'utente |

Una lettura non riuscita non viene mai rappresentata con zero: la riga è
presente, con `valore` vuoto e lo stato che spiega il motivo.

## Chiavi dei parametri principali

| Chiave | PID | Unità | Significato |
|---|---|---|---|
| `o2s1_lambda` | 24 | λ | Sonda B1S1 (a monte), rapporto di equivalenza |
| `o2s1_tensione_wr` | 24 | V | Sonda B1S1, tensione (banda larga) |
| `o2s1_lambda_i` | 34 | λ | Sonda B1S1, rapporto di equivalenza (PID con corrente) |
| `o2s1_corrente` | 34 | mA | Sonda B1S1, corrente |
| `o2s1_tensione` | 14 | V | Sonda B1S1, tensione (banda stretta) |
| `o2s2_tensione` | 15 | V | Sonda B1S2 (a valle), tensione |
| `o2s2_stft` | 15 | % | Correzione associata alla sonda B1S2 (assente se la centralina indica «non usata») |
| `stft_b1` | 06 | % | Correzione carburante a breve termine, bancata 1 |
| `ltft_b1` | 07 | % | Correzione carburante a lungo termine, bancata 1 |
| `giri` | 0C | giri/min | Giri motore |
| `velocita` | 0D | km/h | Velocità |
| `temp_refrigerante` | 05 | °C | Temperatura refrigerante |
| `carico` | 04 | % | Carico motore calcolato |
| `map` | 0B | kPa | Pressione collettore |
| `distanza_parziale` | — | km | Distanza percorsa dalla connessione, **calcolata dall'app** integrando la velocità (nessun PID: non è letta dalla centralina) |
| `contachilometri` | A6 | km | Contachilometri, solo sui veicoli che rispondono (obbligatorio dal modello 2019) |

Un PID che contiene più grandezze produce più righe con lo stesso `timestamp`
(per esempio `o2s1_lambda` e `o2s1_tensione_wr`). L'elenco completo delle
chiavi è in `src/obd/pids.rs`.

`distanza_parziale` riparte da zero a ogni connessione; il suo `ecu` è quello
della velocità da cui deriva. Il tratto fra due letture della velocità
distanti più di 5 secondi non è conteggiato.

Le sonde vengono interrogate più spesso degli altri parametri, quindi le
righe non sono a passo costante: usa `timestamp` o `tempo_s` di ogni riga.

## Esempio

```
timestamp,tempo_s,ecu,parametro,descrizione,valore,unita,stato,origine
2026-11-21T19:37:41.512+01:00,0.104,7E8,o2s1_lambda,Sonda B1S1 (a monte) — lambda,1.0124,λ,ok,veicolo
2026-11-21T19:37:41.512+01:00,0.104,7E8,o2s1_tensione_wr,Sonda B1S1 (a monte) — tensione (banda larga),1.5291,V,ok,veicolo
2026-11-21T19:37:41.566+01:00,0.158,7E8,o2s2_tensione,Sonda B1S2 (a valle) — tensione,0.6250,V,ok,veicolo
2026-11-21T19:37:41.620+01:00,0.212,7E8,stft_b1,STFT bancata 1,0.781,%,ok,veicolo
2026-11-21T19:37:46.530+01:00,5.122,,marcatore,Minimo,,,marcatore,veicolo
2026-11-21T19:38:29.871+01:00,48.463,,o2s2_tensione,Sonda B1S2 (a valle) — tensione,,V,risposta_mancante,veicolo
```

## File di sessione

La sessione riapribile (`.obdio.jsonl`) è un file di testo con un oggetto JSON
per riga: un'intestazione, l'eventuale diagnosi, poi campioni e marcatori
nell'ordine di arrivo. Viene scritto in coda durante la registrazione, quindi
resta leggibile anche se la registrazione si interrompe bruscamente: un'ultima
riga incompleta viene ignorata all'apertura.
