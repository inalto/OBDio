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
| `tempo_s` | Secondi trascorsi dall'inizio della registrazione: tre decimali, oppure tutte le cifre necessarie a rileggere il valore esatto (vedi sotto) |
| `ecu` | Centralina che ha risposto (es. `7E8`). Vuoto se nessuna ha risposto e per i marcatori |
| `parametro` | Chiave stabile del parametro (vedi sotto), oppure `marcatore` |
| `descrizione` | Nome leggibile del parametro; per i marcatori, il testo del marcatore |
| `valore` | Valore numerico. **Vuoto** quando lo stato non è `ok` e per i marcatori |
| `unita` | Unità di misura del valore |
| `stato` | Esito della lettura (vedi sotto) |
| `origine` | `veicolo` per dati letti da una centralina, `demo_sintetico` per dati simulati |

`valore` e `tempo_s` si scrivono con i decimali indicati (uno in più di quelli
mostrati dall'app per il valore, tre per il tempo) quando così si rilegge
esattamente lo stesso numero; altrimenti con tutte le cifre necessarie, mai in
notazione esponenziale (per esempio `2128.25` invece di `2128.2`). Così
esportare e reimportare un CSV non perde precisione.

Per un parametro libero (fuori dal catalogo, per esempio importato da Torque)
`descrizione` e `unita` vengono dall'intestazione della sessione.

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

Due campi facoltativi dell'intestazione compaiono solo nelle sessioni
importate (i file più vecchi, che non li hanno, si leggono come prima):

- `importata`: `{"file": "trackLog.csv", "formato": "CSV in stile Torque",
  "il": "<data dell'importazione>"}`;
- `parametri`: nome, unità ed eventualmente decimali dei parametri liberi,
  per chiave (`{"torque:gps_speed": {"nome": "GPS Speed", "unita": "Meters/second"}}`).

## CSV «largo» in stile Torque

Esportazione pensata per gli strumenti che leggono i registri di Torque
(`trackLog.csv`). Non esiste una specifica pubblica: il formato segue le
convenzioni documentate in [RICERCA.md](RICERCA.md), sezione 5, ed **è stato
verificato solo con file scritti dall'app stessa o a mano**, non con un
registro prodotto da Torque su un telefono.

- UTF-8 **senza** BOM, separatore virgola, punto decimale, fine riga LF.
- Prime due colonne: `GPS Time` (sempre vuota: OBDio non ha il GPS) e
  `Device Time` nella forma di Torque `dd-MMM-yyyy HH:mm:ss.SSS` con i mesi
  in inglese, per esempio `21-Nov-2026 19:37:41.512` (ora locale del campione).
- Poi una colonna per parametro, nell'ordine di prima comparsa, con
  intestazione `Nome(unità)`. Per i parametri della tabella qui sotto si usa il
  nome di Torque (`Engine RPM(rpm)`); per gli altri il nome di OBDio
  (`Distanza parziale calcolata(km)`), con la chiave fra quadre quando due
  parametri del catalogo hanno lo stesso nome
  (`Sonda B1S1 (a monte) — lambda [o2s1_lambda](λ)`).
- **Una riga per istante di campionamento**: i campioni con lo stesso tempo
  (tutte le grandezze di una stessa risposta, per esempio lambda e tensione
  della sonda) stanno sulla stessa riga. Le righe non sono a passo costante.
- Cella con un numero: letto in quell'istante. Cella `-`: letto in
  quell'istante ma senza valore (risposta mancante, non supportato, errore;
  **mai zero**). Cella vuota: non letto in quell'istante; i valori non vengono
  ripetuti dalle righe precedenti.
- Di un parametro letto da più centraline nello stesso istante resta la prima
  risposta (Torque non ha una colonna per la centralina).
- Marcatori e diagnosi non sono esportati: Torque non li prevede.

## Importazione

Il formato si riconosce dal contenuto, non dall'estensione:

| Contenuto | Formato |
|---|---|
| Prima riga JSON (`{…`) | Sessione `.obdio.jsonl` |
| Intestazione con `tempo_s`, `parametro`, `valore`, `stato` | CSV OBDio |
| Intestazione con `Device Time` o `GPS Time` | CSV in stile Torque |

Regole comuni:

- UTF-8 con o senza BOM; se il file non è UTF-8 viene letto come Latin-1
  (Windows) e lo si dice nel riepilogo. CR LF e LF sono accettati.
- Separatore: virgola, punto e virgola o tabulazione, scelto contando quelli
  della riga d'intestazione. Con il **punto e virgola** (come salva Excel in
  italiano) la virgola è il separatore decimale e il punto quello delle
  migliaia: `2.128,5` vale 2128,5.
- Campi fra virgolette doppie come nell'esportazione; una virgoletta mai
  chiusa termina alla fine del file. Righe con colonne in più o in meno sono
  importate per quanto leggibili e contate come «irregolari».
- Prima di salvare si vede un riepilogo: formato, campioni, marcatori,
  parametri riconosciuti, parametri conservati come liberi, righe saltate,
  intestazioni ripetute, valori non numerici, conversioni di unità.
- La sessione importata è salvata nella cartella delle sessioni come
  `<data della sessione>[_DEMO]_IMPORTATA_<nome del file>.obdio.jsonl`, senza
  mai sovrascrivere; il file d'origine non viene toccato.

CSV OBDio: andata e ritorno esatti di campioni (tempo, data e ora, centralina,
chiave, valore, unità, stato), marcatori e origine (`demo_sintetico` rende la
sessione «demo»). Una riga con `tempo_s` illeggibile o uno `stato`
sconosciuto è saltata e contata; un `ok` senza numero leggibile diventa
`errore_comunicazione` senza valore; il valore di una riga con stato diverso
da `ok` viene ignorato. La diagnosi non fa parte del CSV.

CSV in stile Torque:

- Il tempo di ogni riga viene da `Device Time` (anche nelle forme che scrive
  Excel in italiano, `21-nov-2026 19:37:41,512` o `21/11/2026 19:37:41`); `t`
  riparte da zero alla prima riga leggibile. Le righe con un'ora illeggibile
  sono saltate e contate. `GPS Time` è ignorata.
- Ogni intestazione si divide sull'**ultimo** gruppo fra parentesi:
  `Throttle Position(Manifold)(%)` ha nome `Throttle Position(Manifold)` e
  unità `%`.
- Un'intestazione ripetuta a metà file (Torque la riscrive quando cambia il
  profilo) è riconosciuta: le colonne si rileggono e le stesse colonne
  conservano la stessa chiave. Un file con la sola intestazione viene
  rifiutato con un messaggio.
- Cella `-`: campione «risposta mancante» senza valore. Cella vuota: nessun
  campione. Testo non numerico (`N/A`): campione «errore di comunicazione»
  senza valore, contato.
- Torque non dice quale centralina ha risposto: la colonna `ecu` resta vuota.

Colonne riconosciute (il confronto ignora maiuscole e spazi doppi):

| Colonna di Torque | Chiave OBDio |
|---|---|
| `Engine RPM` | `giri` |
| `Speed (OBD)` | `velocita` |
| `Engine Coolant Temperature` | `temp_refrigerante` |
| `Engine Load` | `carico` |
| `Intake Manifold Pressure` | `map` |
| `Intake Air Temperature` | `temp_aspirazione` |
| `Throttle Position(Manifold)` | `farfalla` |
| `Fuel Trim Bank 1 Short Term` / `Long Term` | `stft_b1` / `ltft_b1` |
| `Fuel Trim Bank 2 Short Term` / `Long Term` | `stft_b2` / `ltft_b2` |
| `O2 Volts Bank 1 sensor 1` / `sensor 2` | `o2s1_tensione` / `o2s2_tensione` |
| `Mass Air Flow Rate` | `maf` |
| `Timing Advance` | `anticipo` |
| `Voltage (Control Module)` | `tensione_modulo` |

Sono riconosciuti anche i nomi dei parametri di OBDio (quelli che l'app
scrive esportando), con la chiave fra quadre quando servono. Le sonde della
bancata 2 non sono nella tabella: la loro numerazione dipende dalla
configurazione delle sonde del veicolo (PID 13 o 1D), che il file non dice.

Conversioni di unità, solo per le colonne riconosciute:

| Unità nel file | Unità OBDio | Conversione |
|---|---|---|
| `°F` | °C | (F − 32) × 5/9 |
| `mph` | km/h | × 1,609344 |
| `psi` | kPa | × 6,894757 |
| `bar` | kPa | × 100 |
| `rpm` | giri/min | nessuna |

Se l'unità di una colonna riconosciuta non è né quella di OBDio né una di
queste (per esempio `Intake Manifold Pressure(inHg)`), la colonna non viene
convertita a caso: resta un parametro libero con la sua unità.

Colonne sconosciute: nulla viene scartato. Ogni colonna diventa un
**parametro libero** con chiave stabile `torque:<nome normalizzato>` (minuscolo,
`_` al posto di tutto ciò che non è lettera o cifra: `GPS Speed (Meters/second)`
diventa `torque:gps_speed` con unità `Meters/second`), nome e unità salvati
nell'intestazione della sessione. I parametri liberi compaiono nella pagina
Sensori (gruppo «Altro»), nei grafici, nell'elenco dei parametri del cruscotto
e nelle esportazioni; un veicolo collegato non li offre mai.
