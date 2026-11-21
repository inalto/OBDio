# OBDio

Lettore diagnostico OBD-II/EOBD per Windows 11 x64, pensato per adattatori
ELM327 Bluetooth come il Carista. Funziona solo in locale: nessun account,
nessun abbonamento, nessun servizio cloud.

**L'app lavora in lettura, con una sola eccezione.** Legge codici guasto,
fermo immagine, monitor e dati in tempo reale. L'unica azione che modifica la
centralina è **Cancella codici guasto** (servizio OBD 04), che va confermata
esplicitamente: vedi [Cancellare i codici guasto](#cancellare-i-codici-guasto).
Non scrive sulle centraline, non esegue codifiche né test attuatori e non
permette di inviare comandi liberi.

Versione 0.1.0. Primo caso d'uso: Peugeot 208 1.2 benzina 82 CV (motore HMZ)
con codice P0420.

> **Stato delle verifiche.** Tutto il flusso è stato verificato in modalità
> demo e con test automatici. **Non è ancora stato provato su un adattatore
> Carista né sull'auto**: vedi [Stato delle verifiche](#stato-delle-verifiche)
> e [Controlli da fare sull'auto](#controlli-da-fare-sullauto).

## Avvio

Il pacchetto è portabile: copia la cartella dove preferisci e avvia
`OBDio.exe`. Non serve installare nulla. Le sessioni vengono salvate nella
sottocartella `sessioni` accanto all'eseguibile (se la cartella non è
scrivibile, in `Documenti\OBDio\sessioni`).

L'eseguibile non è firmato: al primo avvio Windows SmartScreen può mostrare un
avviso («Ulteriori informazioni» → «Esegui comunque»).

## Provare la demo

1. Scheda **Connessione** → **Avvia la demo**.
2. Scheda **Codici guasto e fermo immagine** → **Leggi diagnosi**: compare
   P0420 con il fermo immagine a 69 km/h, 2.128 giri/min, 83 °C, STFT +0,78 %,
   LTFT +8,59 %.
3. Scheda **Sensori** → **Avvia lettura**, poi **Avvia registrazione**.
   Prova anche **Un grafico per sensore**, **Tutti** e il campo di ricerca.

In demo una fascia arancione **MODALITÀ DEMO** resta sempre visibile e ogni
curva porta la dicitura «(sintetico)». Tutti i valori sono generati da un
simulatore, segnali delle sonde lambda compresi: non descrivono un veicolo
reale. Il simulatore produce testo ELM327 grezzo, quindi i dati passano dallo
stesso decoder usato con l'adattatore vero. Ogni minuto simula 2,5 secondi di
mancata risposta della sonda a valle, per mostrare come appaiono i buchi.

## Collegare l'adattatore

Inserisci l'adattatore nella presa OBD, porta il quadro su ON (o avvia il
motore) e chiudi sul telefono le app collegate all'adattatore: un adattatore
già connesso al telefono non è raggiungibile dal PC.

### Bluetooth Low Energy (prima scelta)

1. Scheda **Connessione** → **Cerca dispositivi**. Non serve associare
   l'adattatore in Windows.
2. Seleziona l'adattatore nell'elenco (i probabili adattatori OBD sono
   marcati `[OBD]` e stanno in cima) → **Collega il dispositivo selezionato**.
3. L'app scopre i servizi GATT e apre il canale dati **solo** se corrispondono
   a uno dei profili seriali documentati (vedi sotto). Altrimenti mostra
   «Profilo BLE sconosciuto» con nome e UUID del dispositivo, senza scrivere
   nulla: usa **Copia elenco UUID** e conserva il testo.

### Porta seriale (Bluetooth Classic/SPP)

Per gli adattatori che Windows associa come dispositivo Bluetooth classico:

1. Associa l'adattatore da *Impostazioni → Bluetooth e dispositivi*.
2. In *Altre impostazioni Bluetooth → Porte COM* individua la porta **in
   uscita** associata all'adattatore.
3. In OBDio scegli quella porta COM → **Collega via seriale**. La velocità
   (baud) è irrilevante per le porte Bluetooth virtuali; 38400 va bene.

### Messaggi di errore

| Messaggio | Significato |
|---|---|
| Bluetooth disattivato | La radio Bluetooth di Windows è spenta. |
| Nessun adattatore Bluetooth nel PC | Windows non segnala alcuna radio. |
| Adattatore non raggiungibile | Il collegamento radio (o l'apertura della porta COM) non riesce: adattatore non alimentato, fuori portata o occupato dal telefono. |
| Profilo BLE sconosciuto | Collegato, ma i servizi GATT non sono fra quelli documentati. Nessuna scrittura eseguita. |
| L'adattatore non risponde ai comandi | Il canale è aperto ma il dispositivo non risponde come un ELM327. |
| La centralina non risponde | L'adattatore funziona, ma nessuna centralina risponde: quadro spento o protocollo non trovato. |
| Collegamento interrotto | Il collegamento è caduto. Il polling si ferma; la registrazione in corso viene chiusa e conservata. |

Il **Registro degli scambi con l'adattatore**, in fondo alla scheda
Connessione, mostra comandi e risposte ed è copiabile. La risposta al VIN non
viene mai registrata.

## Pagina Sensori

La scheda **Sensori** elenca **tutti** i parametri del catalogo che la
centralina dichiara di supportare, raggruppati in Motore, Carburante e
miscela, Sonde lambda, Temperature e pressioni, Distanze e tempi, Altro.

- **Ricerca** per nome, e selezioni rapide: **Diagnosi P0420** (sonde, STFT,
  LTFT, giri, velocità, temperatura), **Tutti**, **Nessuno**.
- La **spunta** accanto a un sensore lo mette nella lettura; se la lettura è
  già attiva, cambiare la selezione la aggiorna subito. Per ogni sensore
  spuntato l'elenco mostra il valore con l'unità (o lo stato della lettura, per
  esempio «Risposta mancante», mai zero) e la frequenza effettiva in Hz.
- A destra due viste. **Confronto**: i grafici impilati delle sonde, delle
  correzioni carburante e di un parametro aggiuntivo, con asse del tempo
  condiviso. **Un grafico per sensore**: una scheda per ogni sensore spuntato,
  con valore grande, **minimo, medio e massimo** dall'inizio della
  connessione, Hz e un piccolo grafico con le stesse fasce rosse (dato
  mancante), gli stessi marcatori e la stessa finestra temporale; le schede si
  dispongono in colonne secondo la larghezza della finestra.
  **Azzera min/max** riparte da zero con le statistiche.
- Senza collegamento l'elenco mostra i parametri presenti nei dati già letti.
- Sotto l'elenco restano **Avvia lettura**, la registrazione, i marcatori e
  l'ultima sessione salvata.

I grafici servono a confrontare i segnali: l'app non deduce se catalizzatore o
sonde siano guasti.

### Parametri aggiunti

Contachilometri (PID A6), distanza e tempo dalla cancellazione dei codici
(31, 4E), avviamenti a caldo dalla cancellazione (30), tempo con MIL accesa
(4D), pressione carburante (0A) e del rail (23), posizione della farfalla (45,
47, 4C), del pedale dell'acceleratore (49, 4A, 5A), coppia richiesta, effettiva
e di riferimento (61–63) e consumo istantaneo (5E). Tipo di carburante (51) e
norma OBD (1C) sono testi: si leggono una volta con la diagnosi e compaiono
nella pagina dei codici guasto e nel riepilogo. Le formule vengono da
Wikipedia, «OBD-II PIDs»; quelle dei PID 49, 4A e 4C sono dedotte da righe
accorpate della tabella e non verificate.

### Distanza parziale

Il **contachilometri** (PID A6) è obbligatorio solo dal modello 2019: molte
auto non rispondono. Per questo l'app calcola la **distanza parziale**: integra
la velocità della centralina principale nel tempo (regola del trapezio) a
partire dalla connessione. Se fra due letture della velocità passano più di 5
secondi, quel tratto non viene conteggiato, e una lettura mancante non aggiunge
nulla. È una stima, più precisa con una lettura frequente della velocità;
selezionarla fa leggere la velocità in automatico. Il valore viene registrato
come ogni altro sensore (chiave `distanza_parziale`, in km).

## Acquisire una sessione P0420

1. Collega l'adattatore; in **Connessione** controlla protocollo e centralina
   (sulla 208 sono attesi «ISO 15765-4 CAN, ID a 11 bit, 500 kbit/s» ed ECU
   `7E8`).
2. **Codici guasto e fermo immagine** → **Leggi diagnosi**. Se la leggi prima
   di registrare, la diagnosi viene salvata dentro la sessione.
3. **Sensori**: la selezione rapida **Diagnosi P0420** attiva i segnali che
   la centralina espone davvero:
   - sonda a monte (B1S1): lambda se disponibile, altrimenti tensione;
   - sonda a valle (B1S2): tensione, o l'altro segnale esposto;
   - STFT e LTFT bancata 1, giri, velocità, temperatura refrigerante;
   - carico e pressione collettore si aggiungono spuntandoli nell'elenco.

   Un parametro non dichiarato dalla centralina appare come «non supportato».
4. **Avvia registrazione** (avvia anche la lettura). A motore caldo conviene
   registrare tre fasi, premendo il marcatore corrispondente all'inizio di
   ognuna: **Minimo**, **2.500 giri**, **Velocità costante**. Puoi aggiungere
   marcatori con testo libero.
5. **Ferma registrazione**. Sessione e CSV vengono scritti subito.

Durante la lettura le sonde vengono interrogate a ogni giro e gli altri
parametri a rotazione: la colonna **Hz** indica la frequenza effettiva di
aggiornamento di ciascun parametro. La registrazione prosegue con la finestra
ridotta a icona. Se chiudi l'app o il collegamento cade, quanto già
registrato resta su disco.

### Leggere i grafici

- Le due sonde condividono l'asse del tempo; stanno in pannelli separati
  quando hanno unità diverse (per esempio λ e V).
- Le **fasce rosse** indicano intervalli senza dato valido (risposta mancante,
  errore, oppure più di 2,5 secondi senza campioni). La linea si interrompe:
  i dati mancanti non vengono mai disegnati come zero.
- Le linee gialle tratteggiate sono i marcatori, elencati sopra i grafici.
- **Finestra** sceglie l'ampiezza della vista scorrevole; «Tutto» mostra
  l'intera acquisizione e permette zoom e trascinamento liberi.

**I grafici servono a confrontare i segnali.** L'app non stabilisce se il
catalizzatore o una sonda siano guasti: né dal P0420, né dalla somiglianza
fra le curve.

## Recuperare il CSV e riaprire le sessioni

A ogni registrazione fermata l'app scrive nella cartella `sessioni` due file
con lo stesso nome (data e ora; le sessioni demo hanno il suffisso `_DEMO`):

- `AAAA-MM-GG_HH-MM-SS.obdio.jsonl` — la sessione, riapribile in OBDio;
- `AAAA-MM-GG_HH-MM-SS.csv` — l'esportazione.

**Apri la cartella** (in *Sensori* dopo una registrazione, oppure
in *Sessioni salvate*) la mostra in Esplora file. Nella scheda **Sessioni
salvate** scegli una sessione per rivedere grafici e diagnosi senza collegare
il veicolo; **Esporta CSV** riscrive il CSV.

Il formato del CSV è descritto in [docs/FORMATO_CSV.md](docs/FORMATO_CSV.md).
In breve: UTF-8, una riga per campione, colonne
`timestamp, tempo_s, ecu, parametro, descrizione, valore, unita, stato, origine`;
le letture mancanti hanno `valore` vuoto e uno `stato` esplicito.

### Riepilogo e VIN

**Copia riepilogo** copia negli appunti DTC, fermo immagine, protocollo,
monitor e parametri disponibili. Il VIN viene letto solo per essere mostrato a
schermo: è escluso dal riepilogo salvo spunta esplicita di «Includi il VIN nel
riepilogo», e non viene mai scritto nelle sessioni, nel CSV o nel registro.

## Cosa mostra la diagnosi

- **Codici guasto** confermati (servizio 03), in sospeso (07) e permanenti
  (0A), con centralina di origine. Lo stesso codice della stessa centralina
  occupa una sola riga con tutti gli stati rilevati.
- **Spia MIL** e **monitor di prontezza** (PID 01).
- **Fermo immagine** (servizio 02) in un riquadro separato dai dati dal vivo,
  con il DTC che la centralina dichiara come causa del fotogramma — anche se è
  diverso da quello atteso.
- **Monitor del catalizzatore** (Mode 06, MID 21) se esposto. Le unità
  derivano dalla tabella «Unit and Scaling ID» ripresa dal progetto
  python-OBD, **non confrontata con il testo SAE J1979**: accanto a ogni
  valore l'app mostra sempre il dato grezzo, e per gli identificativi fuori
  tabella mostra solo quello. L'app non giudica l'esito del test.

Per ogni dato lo stato è uno fra: **OK**, **Non supportato** (la centralina
non lo dichiara o lo rifiuta), **Risposta mancante** (dichiarato ma senza
risposta), **Errore di comunicazione** (timeout, errore di bus, risposta non
interpretabile).

## Cancellare i codici guasto

Nella scheda **Codici guasto e fermo immagine**, il pulsante rosso
**Cancella codici guasto…** (a destra, lontano da *Leggi diagnosi*) invia il
servizio OBD 04. È attivo solo con l'adattatore collegato e nessuna
operazione in corso, e apre una finestra di conferma: il pulsante di
cancellazione si abilita solo dopo la spunta «Ho letto e voglio cancellare i
codici». *Annulla* è il pulsante predefinito; anche Esc annulla.

Cosa succede, nell'ordine:

1. se in questo collegamento non è ancora stata letta la diagnosi, l'app la
   legge;
2. salva una copia della diagnosi nella cartella `sessioni`:
   `diagnosi_prima_cancellazione_AAAA-MM-GG_HH-MM-SS.json` (`_DEMO` in demo),
   senza VIN. **Se la copia non si può scrivere, la cancellazione non parte**;
3. invia `04` e mostra l'esito per ogni centralina: eseguita, rifiutata con il
   motivo (per esempio «condizioni non corrette: di solito il motore deve
   essere spento con il quadro acceso»), nessuna risposta, oppure errore di
   comunicazione (esito incerto);
4. rilegge la diagnosi e la mostra. Se è in corso una registrazione, vi
   scrive il marcatore «Codici guasto cancellati» e la nuova diagnosi.

Conseguenze della cancellazione (servizio 04):

- spariscono i codici memorizzati e in sospeso, il **fermo immagine** (resta
  solo nella copia salvata) e i risultati dei monitor (Mode 06); la spia MIL
  si spegne;
- i **monitor di prontezza** tornano «non completato» e si azzerano distanza e
  tempo dalla cancellazione: finché i monitor non si completano (servono uno o
  più cicli di guida) il veicolo può non superare la revisione;
- i **codici permanenti** (servizio 0A) **non** si cancellano: li elimina la
  centralina da sola dopo alcuni cicli senza il guasto;
- cancellare non ripara nulla: se la causa resta, il codice ricompare.

Condizioni: quadro acceso, motore spento. Con il motore acceso molte
centraline rifiutano la richiesta.

> **Verifica.** La cancellazione è stata provata solo con il simulatore demo
> e con risposte a copione (accettata, rifiutata `7F 04 22`, nessuna risposta,
> collegamento caduto). **Non è mai stata provata su un veicolo.**

## MQTT

OBDio può pubblicare i dati in tempo reale su un broker MQTT (per esempio per
Home Assistant): un argomento per sensore, un JSON con tutti i valori, la
diagnosi e gli eventi. Si attiva dalla scheda **Impostazioni**; la
pubblicazione non rallenta mai la lettura. Il VIN non viene mai pubblicato e la
connessione **non è cifrata** (TLS non è supportato in questa versione).
Argomenti, esempi, discovery e note di sicurezza: [docs/MQTT.md](docs/MQTT.md).

## Stato delle verifiche

### Adattatori Carista: cosa è documentato

Il modello esatto (Carista OBD bianco oppure Carista EVO) **va verificato**
sull'etichetta dell'adattatore.

| | Carista OBD (bianco) | Carista EVO |
|---|---|---|
| Compatibilità ELM327 | «ELM327 v1.4» secondo le FAQ Carista | «ELM327 v2.2» secondo le FAQ Carista |
| App di terze parti | Dichiarate supportate da Carista | Dichiarate supportate da Carista; nessuna prova indipendente trovata |
| Bluetooth LE | Indicato da più fornitori di app (OBD Auto Doctor, OBD Fusion) | Non documentato esplicitamente |
| Bluetooth Classic/SPP | Indicato come doppia modalità da OBD Auto Doctor (2017) | Non documentato |
| UUID GATT del canale dati | **Non pubblicati** | **Non pubblicati** |
| Uso su Windows | Non citato da Carista | Non citato da Carista |

Fonti: carista.com (FAQ e pagina delle app partner), obdautodoctor.com
(elenco adattatori supportati e articolo sugli adattatori BLE), obdsoftware.net.

Poiché Carista non pubblica gli UUID, l'app riconosce solo profili seriali
BLE documentati per adattatori ELM327 e non tenta altro:

| Profilo | Scrittura | Notifica | Fonte |
|---|---|---|---|
| Servizio `FFF0` | `FFF2` | `FFF1` | Note ufficiali OBDLink CX; diffuso fra gli adattatori ELM327 BLE |
| Servizio `18F0` | `2AF1` | `2AF0` | Vgate iCar Pro 2S (dump GATT pubblicato) |
| Servizio `FFE0` | `FFE1` | `FFE1` | Moduli seriali HM-10 |
| Nordic UART `6E400001-…` | `6E400002-…` | `6E400003-…` | Documentazione Nordic Semiconductor |

Una discussione su un forum suggerisce che il Carista usi `FFF0`, ma la fonte
non è verificabile: è un'ipotesi, non un dato.

### Cosa è stato provato

| Verifica | Esito |
|---|---|
| Test automatici (121): decoder con DTC multipli, fermo immagine assente o con DTC diverso, PID non supportati, risposte da più ECU, frammentazione dei dati, eco, NO DATA, timeout, perdita del collegamento, CSV, riapertura, cancellazione dei codici (accettata, rifiutata, senza risposta, caduta) | Superati |
| Flusso demo attraverso il thread di lavoro reale: collegamento, P0420, fermo immagine, polling con priorità alle sonde, registrazione, CSV, riapertura | Superato |
| Distanza parziale attraverso il thread di lavoro reale: campioni monotoni in km, registrati nella sessione e nel CSV | Superato |
| Cancellazione dei codici in demo attraverso il thread di lavoro reale: copia della diagnosi, servizio 04, rilettura con P0420 solo permanente, MIL spenta, prontezza azzerata | Superato |
| Finestra di conferma ed esito della cancellazione nell'interfaccia, in demo | Schermate dal collaudo automatico; clic, Esc e Invio a mano **non provati** |
| Cancellazione dei codici su un veicolo reale | **Non provata** |
| Conversazione MQTT reale fra il thread di lavoro (con la demo) e un broker di prova nello stesso processo (sette test): ultima volontà e credenziali nel CONNECT, stato in linea trattenuto, sensori numerici, istantanea con `null` per i valori assenti, diagnosi senza VIN, discovery, QoS 1, broker che cade e torna, chiusura con broker muto | Superato |
| Pubblicazione MQTT verso un broker vero (Mosquitto o altro) e discovery riconosciuta da Home Assistant | **Non provate** |
| Interfaccia in demo, con schermate: connessione, diagnosi, fermo immagine, grafici con buco del dato e marcatori, pagina Sensori (confronto e un grafico per sensore con dieci sensori), sessione riaperta | Superato |
| Nuovi PID (contachilometri, pedale, coppia, consumo, pressioni…) su un veicolo reale | **Non provati**: formule da Wikipedia, verificate solo con test sui valori; quelle dei PID 49, 4A e 4C sono dedotte |
| Scansione BLE reale su un PC Windows 11 | Funziona (dispositivi elencati); nessun Carista presente |
| Connessione BLE a un adattatore reale | **Non provata** |
| Connessione seriale/SPP a un adattatore reale | **Non provata** |
| Lettura da una centralina reale (208 o altro) | **Non provata** |

## Controlli da fare sull'auto

1. **Modello dell'adattatore**: annota se è Carista OBD o Carista EVO.
2. **Scansione BLE**: l'adattatore compare nell'elenco? Annota il nome con cui
   si annuncia.
3. **Connessione BLE**:
   - se appare «Profilo BLE sconosciuto», copia l'elenco degli UUID: serve
     per aggiungere il profilo corretto (file `src/transport/profili.rs`);
   - se si collega, annota il profilo riconosciuto mostrato in *Connessione*.
4. **Se il BLE non funziona**: prova l'associazione classica in Windows e la
   porta COM in uscita.
5. **Adattatore**: la stringa di identificazione (attesa `ELM327 v1.4` o
   `v2.2`) e l'assenza di errori nel registro durante l'inizializzazione.
6. **Protocollo e centralina**: attesi ISO 15765-4 CAN, 11 bit, 500 kbit/s,
   ECU `7E8`, come rilevato dal lettore già funzionante.
7. **Diagnosi**: P0420 presente e con quali stati; confronta fermo immagine,
   MIL e monitor con il lettore già funzionante.
8. **Segnali delle sonde**: quali PID espone davvero la centralina HMZ per
   B1S1 (lambda `24`/`34` oppure tensione `14`) e per B1S2 (`15` o altro).
9. **Frequenza di aggiornamento**: i valori Hz ottenuti via Bluetooth per le
   sonde e per gli altri parametri.
10. **Registrazione**: tre fasi con marcatori, poi verifica di CSV e
    riapertura della sessione.
11. **Distacco**: sfila l'adattatore (o spegni il quadro) durante una
    registrazione e verifica che il polling si fermi e il file resti leggibile.
12. **Mode 06**: se la centralina espone il MID 21, annota TID, scala e valori
    grezzi mostrati.
13. **PID 4F**: se in *Connessione* compare un avviso sui fondi scala delle
    sonde a banda larga, i valori λ/V/mA vanno considerati non verificati.

## Limiti noti

- Sui protocolli non CAN (J1850, ISO 9141-2, KWP2000) sono letti DTC,
  prontezza, fermo immagine e dati dal vivo; Mode 06 e VIN no. Questi
  protocolli sono coperti solo da test sul formato delle trame.
- Le descrizioni dei DTC coprono i codici generici più comuni; per gli altri
  viene mostrato il solo codice.
- Il PID 4F (fondi scala dichiarati dalla centralina) viene segnalato ma non
  applicato alle formule.
- Viene letto il solo fotogramma 0 del fermo immagine.
- Le formule dei PID seguono le definizioni pubbliche di SAE J1979 (come
  riportate da Wikipedia «OBD-II PIDs»).

## Comandi inviati

L'elenco è chiuso nel codice (`AtCommand` in `src/elm.rs`, `ObdRequest` in
`src/obd/mod.rs`) e un test verifica che le letture non possano generare altri
servizi, `04` compreso.

- **Adattatore** (configurazione che non tocca il veicolo): `ATZ`, `ATE0`,
  `ATL0`, `ATS0`, `ATH1`, `ATAT1`, `ATSP0`, `ATDPN`, `ATI`, `ATRV`.
  `ATSP0` imposta la scelta automatica del protocollo, che è anche il valore
  di fabbrica; le altre impostazioni si annullano togliendo alimentazione.
- **Veicolo**, servizi di lettura: `01` (dati correnti), `02` (fermo
  immagine), `03`, `07`, `0A` (lettura DTC), `06` (risultati dei monitor),
  `09 02` (VIN).
- **Veicolo, unica eccezione**: `04` (cancellazione DTC), solo dall'azione
  confermata *Cancella codici guasto*. Ha un percorso separato
  (`Elm::cancella_dtc`) che non si può chiamare senza un valore
  `ConfermaCancellazione`, creato soltanto dalla finestra di conferma.

Mai inviati: `08`, servizi UDS di scrittura, codifica o comando attuatori,
comandi liberi.

## Compilare dai sorgenti

Serve Rust stabile (verificato con 1.93.1, toolchain `x86_64-pc-windows-msvc`).

```
cargo test
cargo build --release
```

Dipendenze principali: `eframe`/`egui` 0.35 e `egui_plot` 0.36 (interfaccia e
grafici; sono le ultime versioni compatibili con Rust 1.93), `btleplug` 0.13
(Bluetooth LE tramite le API WinRT), `serialport` 4 (porte COM).

Struttura:

| Modulo | Compito |
|---|---|
| `transport/` | Canale di byte: `ble`, `serial`, `demo`, più i profili GATT ammessi |
| `elm.rs` | Dialogo ELM327: una richiesta alla volta, prompt, eco, righe multiple, errori |
| `obd/` | Decoder: trame e ISO-TP, PID, DTC, prontezza, Mode 06 |
| `diagnosis.rs` | Lettura di DTC, MIL, prontezza, fermo immagine, Mode 06 |
| `recording.rs` | Sessioni su disco e CSV |
| `mqtt.rs` | Pubblicazione MQTT: argomenti, limitazione di frequenza, coda non bloccante, discovery |
| `worker.rs` | Thread di lavoro con stati espliciti: scansione, connessione, diagnosi, cancellazione, polling, registrazione, disconnessione |
| `ui/` | Interfaccia egui: non esegue mai I/O verso l'adattatore |

Strumenti di sviluppo:

- `cargo run --example scansione_ble` elenca porte COM e dispositivi BLE
  senza collegarsi a nulla;
- `cargo build --features collaudo`, poi avvio con la variabile d'ambiente
  `OBDIO_COLLAUDO=<cartella>`: l'app percorre da sola il flusso demo e salva
  una schermata per ogni passo.
