# Ricerca: leggere i dati dalla linea K

Stato al 23 novembre 2026. Obiettivo: capire che cosa serve a OBDio per
leggere i veicoli che parlano sulla linea K (ISO 9141-2 e ISO 14230-4
«KWP2000») invece che su CAN, restando in sola lettura e con l'adattatore
ELM327 già usato.

Ogni affermazione porta la fonte. Dove la fonte manca è scritto
«da verificare».

## In breve

- Con un adattatore ELM327 la linea K **non richiede di parlare il protocollo
  a basso livello**: inizializzazione, tempi, checksum e messaggi di
  mantenimento li fa l'adattatore. All'app arrivano righe di testo come su
  CAN, in un formato diverso.
- OBDio ha già una parte del lavoro: riconosce i protocolli 3, 4 e 5,
  interpreta le righe con tre byte di intestazione e checksum, legge i codici
  guasto senza il byte di conteggio. Mancano il VIN, i dati del servizio 06,
  alcuni messaggi di errore e una cadenza di lettura adatta a un bus lento.
- **Nulla di tutto questo è mai stato provato su un veicolo a linea K**, e non
  è confermato che l'adattatore Carista (quello bianco, non l'EVO) la
  supporti. È la prima cosa da verificare, e basta un'auto a linea K per farlo.
- La diagnosi «del costruttore» sulla linea K (KW1281 del gruppo VW, varianti
  proprietarie) è un'altra cosa: l'ELM327 non la copre o la copre male, e
  richiederebbe un'altra interfaccia. Vedi [Fuori dall'OBD](#fuori-dallobd).

## Quali veicoli

- ISO 9141-2 e ISO 14230-4 usano il piedino 7 (K) e, sulle auto più vecchie,
  il 15 (L) del connettore OBD. [Wikipedia, On-board diagnostics]
- EOBD è obbligatorio per le auto a benzina immatricolate dal 1º gennaio 2001
  e diesel dal 1º gennaio 2004; negli Stati Uniti dal 2008 tutte le auto
  devono usare ISO 15765-4 (CAN). [Wikipedia, On-board diagnostics]
- In pratica la linea K riguarda le auto europee e asiatiche dei primi anni
  2000, fino al passaggio a CAN. L'intervallo esatto per marca e la data
  dell'obbligo CAN in Europa sono **da verificare**: le fonti lette non li
  riportano.
- La Peugeot 208 del primo caso d'uso è CAN: questa ricerca non la riguarda.

## Come funziona la linea K

### Livello fisico

- Un solo filo bidirezionale, 0 V / tensione di batteria, 10 400 baud, 8N1.
  [m0agx.eu, «Reading OBD2 data without ELM327, part 2»]
- ISO 14230-1 e ISO 9141-2 hanno lo stesso livello fisico. [Wikipedia, KWP2000]

### Inizializzazione

Prima di qualsiasi richiesta il bus va «svegliato». Due modi:

- **Lenta, a 5 baud** (ISO 9141-2 e KWP): l'indirizzo 33h trasmesso a 5 bit al
  secondo. La centralina risponde 55h e due byte chiave; il tester rimanda il
  secondo invertito e la centralina risponde con l'indirizzo invertito.
  Dura 2–3 secondi. [ISO 14230-4 §4.4; m0agx.eu; datasheet ELM327 p. 33]
- **Rapida** (solo KWP): 25 ms bassa, 25 ms alta, poi il messaggio
  StartCommunication `C1 33 F1 81 66`. [m0agx.eu]
- I byte chiave dicono il protocollo: `08 08` o `94 94` = ISO 9141-2, altro =
  KWP2000. [m0agx.eu] Per l'OBD su KWP sono ammesse le parole chiave 2025,
  2027, 2029 e 2031, sempre usate come 2025. [ISO 14230-4 §4.4]
- Una centralina OBD supporta **uno solo** dei due modi; lo strumento deve
  provarli entrambi. [ISO 14230-4 §4.4]

### Formato dei messaggi

| | Richiesta | Risposta |
|---|---|---|
| ISO 9141-2 | `68 6A F1` + dati + checksum | `48 6B <ECU>` + dati + checksum |
| KWP2000 | `Cx 33 F1` + dati + checksum | `8x F1 <ECU>` + dati + checksum |

- Checksum: somma a 8 bit di tutti i byte. [m0agx.eu]
- KWP: i 6 bit bassi del primo byte sono la lunghezza dei dati; per l'OBD
  l'intestazione è sempre di tre byte, senza byte di lunghezza aggiuntivo,
  con al massimo **7 byte di dati**. [m0agx.eu; ISO 14230-4 §4.2]
- ISO 9141-2 non trasmette la lunghezza: chi riceve la deduce dal checksum e
  dalle pause. [m0agx.eu] Con l'ELM327 non è un problema dell'app.
- Esempi (giri motore, PID 0C): richiesta `68 6A F1 01 0C D0`, risposta
  `48 6B 11 41 0C 1F 40 70`; in KWP `C2 33 F1 01 0C F3` e
  `84 F1 11 41 0C 1F 40 32`. [m0agx.eu]

### Risposte su più messaggi

Non esiste ISO-TP. Una risposta lunga arriva come più messaggi indipendenti:

- **VIN (09 02)**: cinque messaggi su ISO 9141-2, ciascuno
  `49 02 <n> <4 byte>`, con `<n>` da 01 a 05 e il primo riempito con zeri a
  sinistra. [datasheet ELM327 p. 42; m0agx.eu]
- **Codici guasto (03, 07)**: più messaggi da tre codici l'uno, senza byte di
  conteggio; le coppie `00 00` sono riempitivo. Così li tratta già
  `parse_dtc_message`; il dettaglio non compare nelle fonti lette ed è
  **da verificare** su un veicolo.
- Più centraline rispondono una dopo l'altra, una riga ciascuna.

### Tempi

- Il collegamento cade dopo 5 secondi senza traffico. [datasheet ELM327 p. 33;
  ISO 14230-4 §5.3]
- Valori di P1–P4 (per quanto ricordo: risposta della centralina entro
  25–50 ms, almeno 55 ms prima della richiesta successiva): **da verificare**,
  stanno in ISO 14230-2 e ISO 9141-2 che non ho potuto leggere.
- Conseguenza pratica: pochi valori al secondo, contro le decine di CAN. Una
  cifra misurata manca; dipende soprattutto da quanto l'adattatore aspetta
  eventuali altre risposte (vedi sotto).

## Che cosa fa l'ELM327

Fonte: datasheet ELM327DSH (firmware v1.4b), pagine indicate.

| Argomento | Comportamento |
|---|---|
| Protocolli | 3 = ISO 9141-2 (5 baud), 4 = KWP (5 baud), 5 = KWP (rapida), tutti a 10,4 kbaud (p. 24) |
| Inizializzazione | La fa l'adattatore alla prima richiesta. Con la ricerca automatica non stampa nulla; con un protocollo fissato stampa `BUS INIT: ...` poi `OK` o un errore. I puntini compaiono solo con quella lenta (p. 33) |
| Mantenimento | Manda da solo un messaggio ogni 3 secondi di silenzio; le risposte non si vedono mai. `AT SW hh` cambia l'intervallo (passi da 20 ms, 00 = spento), `AT WM` il messaggio (p. 25, 33) |
| Byte chiave | `AT KW` li mostra. `AT KW0` disattiva il controllo, per centraline non conformi (p. 18) |
| Inizializzazione a mano | `AT SI` (lenta) e `AT FI` (rapida, solo protocollo 5) (p. 16, 23) |
| Indirizzo e velocità | `AT IIA hh` cambia l'indirizzo a 5 baud (predefinito 33); `AT IB 10/48/96` la velocità (p. 16, 17) |
| Attesa delle risposte | Non sapendo quante risposte arriveranno, aspetta un tempo (`AT ST hh`, passi da 4 ms; adattivo con `AT AT1`) dopo l'ultima. Un numero in coda alla richiesta (`09 02 5`) dice quante righe aspettare e toglie l'attesa (p. 25, 29) |
| Errori | `BUS INIT: ...ERROR`, `UNABLE TO CONNECT`, `BUS BUSY`, `BUS ERROR`, `DATA ERROR`, `<DATA ERROR` (checksum sbagliato su quella riga) (p. 72) |

Con le intestazioni attive (`AT H1`, che OBDio usa) le righe sono complete di
intestazione e checksum, per esempio `48 6B 10 41 00 BE 3E B8 11 FA` (p. 42).

## L'adattatore Carista

- Carista dichiara l'adattatore standard «compatibile con lo standard ELM327
  v1.4» e l'EVO «basato su ELM327 v2.2». [carista.com/pages/faqs]
- La stessa pagina dice che solo l'EVO gestisce i modelli BMW a linea K (serie
  E fino al 2007) e che le varianti proprietarie della linea K di vari marchi
  non sono compatibili con i loro adattatori. Parla delle funzioni dell'app
  Carista, non dell'OBD generico.
- **Non ho trovato una dichiarazione sul supporto di ISO 9141-2 e ISO 14230-4
  generici.** La pagina di confronto di Carista non era leggibile (errore 403).
- Molti cloni ELM327 dichiarano una versione e non implementano tutti i
  protocolli. [cnx-software.com, «Beware of fake ELM327»] Vale come
  avvertenza generale, non come dato sul Carista.

Si verifica in un minuto su un'auto a linea K: collegarsi con OBDio e guardare
il protocollo riportato. Se la ricerca automatica finisce con
`UNABLE TO CONNECT` su un'auto che altri strumenti leggono, l'adattatore non
ha la linea K.

## Stato di OBDio

Aggiornato dopo il lavoro descritto in [NOVITA.md](NOVITA.md) («Linea K»).
Tutto è provato solo con veicoli simulati (`Copione::con_linea_k`).

| Parte | Stato |
|---|---|
| Riconoscimento del protocollo (`ATDPN` 3, 4, 5) | Presente (`src/elm.rs`) |
| Righe con tre byte di intestazione e checksum, una per messaggio | Presente (`src/obd/frames.rs`, `BusKind::Legacy`) |
| Codici guasto 03 e 07 senza byte di conteggio, su più messaggi | Presente (`src/obd/dtc.rs`) |
| Dati in tempo reale, prontezza, fermo immagine | Stesso contenuto di CAN in un altro involucro; prontezza provata in simulazione |
| VIN su più messaggi numerati | **Aggiunto** (`src/diagnosis.rs`), con la stessa riservatezza del VIN su CAN |
| `BUS INIT: ...OK`, `BUS INIT: ...ERROR`, `BUS BUSY`, `<DATA ERROR` | **Aggiunto**: l'inizializzazione riuscita non è un errore, quella fallita suggerisce di accendere il quadro, una riga con checksum errato viene scartata da sola |
| Cadenza di lettura | **Aggiunto**: dalla seconda lettura di un dato corrente la richiesta porta in coda il numero di risposte attese; se l'adattatore lo rifiuta («?») non viene più usato |
| Nota nell'interfaccia | **Aggiunto**: la scheda del collegamento avvisa che sulla linea K i valori si aggiornano più lentamente |
| Servizio 06 | Ancora mancante fuori da CAN (formato TID/CID) |
| Scelta manuale del protocollo, `AT KW0` | Non fatta: da decidere dopo la prova sul campo |
| Scenario demo a linea K | Non fatto: la demo simula solo CAN |
| Tempo della ricerca (25 s) | Da misurare su un veicolo |

## Proposta di lavoro

1. **Prova sul campo, prima di scrivere codice.** Un'auto a linea K, la
   versione attuale, e il registro dei collegamenti. Dice subito se il Carista
   la supporta e quali parti reggono già.
2. **Simulazione**: uno scenario ISO 9141-2 e uno KWP nel trasporto a copione
   e nella demo, con le righe degli esempi qui sopra. Permette le prove senza
   veicolo.
3. **VIN su più messaggi**: riordinare per contatore, togliere gli zeri di
   riempimento, stessa regola di riservatezza del VIN su CAN.
4. **Errori e messaggi**: testi propri per inizializzazione fallita e bus
   occupato; scarto della sola riga con checksum errato.
5. **Cadenza**: numero di risposte attese in coda alle richieste dopo il primo
   giro (una per ECU che ha risposto), meno valori per ciclo, e nell'interfaccia
   l'avviso che su questo bus l'aggiornamento è lento.
6. **Servizio 06 non CAN**: per ultimo, è il meno usato e il più diverso.
7. **Scelta manuale del protocollo** (`ATSP3/4/5`, `AT KW0`) nelle
   impostazioni avanzate, solo se la prova sul campo mostra che la ricerca
   automatica non basta.

Tutti i comandi nuovi sono di configurazione temporanea dell'adattatore o
richieste OBD di lettura: restano dentro i limiti del progetto.

## Fuori dall'OBD

Prima di EOBD, e accanto ad esso, molte centraline usano la linea K con
protocolli del costruttore.

- L'ELM327 può avvicinarsi ad alcune (`AT IIA` per un altro indirizzo,
  `AT KW0`, `AT IB 96`, `AT SH` per l'indirizzamento fisico), ma lavora solo
  sul piedino 7 e con i suoi tempi.
- KW1281 (gruppo VW) non rientra in questo schema: velocità variabili e
  conferma byte per byte. **Da verificare** nel dettaglio; non è nel datasheet.
- La strada usuale è un cavo «KKL» (un convertitore USB–linea K) e
  l'implementazione del protocollo nell'app. OBDio ha già il trasporto
  seriale, ma servirebbero la gestione dei tempi al millisecondo e richieste
  fuori dall'elenco chiuso di oggi.

È un progetto a parte, da decidere caso per caso in base all'auto.

## Fonti

- Datasheet ELM327DSH, Elm Electronics (v1.4b), copia su scantool.net:
  <https://www.scantool.net/scantool/downloads/103/elm327dsh.pdf>
- ISO 14230-4:2000, «Requirements for emission-related systems»:
  <https://law.resource.org/pub/us/cfr/ibr/inc/iso/iso.14230-4.2000.pdf>
- «Reading OBD2 data without ELM327, part 2 – K-Line» (m0agx.eu, ripubblicato):
  <https://matt2005.github.io/2019-12-28-repost-reading-obd2-data-without-elm327-part2>
- Wikipedia, «On-board diagnostics»:
  <https://en.wikipedia.org/wiki/On-board_diagnostics>
- Wikipedia, «ELM327»: <https://en.wikipedia.org/wiki/ELM327>
- Wikipedia, «Keyword Protocol 2000»: <https://www.wikipedia.com/wiki/KWP2000>
- Carista, domande frequenti: <https://carista.com/pages/faqs>
- CNX Software, «Beware of fake ELM327 OBD-II Bluetooth adapters»:
  <https://cnx-software.com/?p=103194>

Non letti, perché non accessibili: ISO 14230-2 e ISO 9141-2 (tempi P1–P4), la
pagina di confronto di Carista fra OBD ed EVO.
