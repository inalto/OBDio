# Ricerca: leggere la centralina del Land Rover Td5

Stato al 23 novembre 2026. Riguarda Defender e Discovery 2 con motore Td5
(1998–2006). Nulla di quanto segue è stato provato su un veicolo da OBDio.

OBDio ha ora una modalità Td5 in sola lettura per gli adattatori OBDLink CX
e LX, provata **solo con un adattatore e una centralina simulati**: vedi
[Stato in OBDio](#stato-in-obdio).

## In breve

- Il Td5 parla sulla linea K (piedino 7) ma **non è OBD-II/EOBD**: usa un
  dialetto proprio di KWP2000. La lettura generica di OBDio, anche con il
  supporto alla linea K della 1.1.0, non può funzionare.
- Il protocollo è documentato bene da progetti aperti: inizializzazione,
  accesso di sicurezza, dati in tempo reale, codici guasto.
- **Un adattatore ELM327 (quindi anche il Carista) quasi certamente non può
  parlarlo**: il Td5 usa messaggi senza intestazione, mentre l'ELM327 ne
  aggiunge sempre una di tre byte. Vedi [Perché non con l'ELM327](#perché-non-con-lelm327).
- La strada usata da tutti i progetti aperti è un **cavo USB «KKL»** (linea K
  su porta seriale, di solito con chip FTDI), con il protocollo implementato
  nell'app. OBDio ha già il trasporto seriale.

## Il protocollo

Fonte principale: `TD5_PROTOCOL_REFERENCE.md` di SimonRafferty (licenza MIT,
aggiornato a settembre 2026 con verifiche su veicolo contro un Nanocom).

### Livello fisico e messaggi

- Linea K, 10 400 baud, 8N1, semiduplex: ogni byte trasmesso torna indietro
  come eco e va scartato.
- Messaggio: `[lunghezza] [servizio] [dati…] [checksum]`. La lunghezza conta i
  byte fra sé e il checksum; il checksum è la somma a 8 bit di tutto.
- Risposta positiva = servizio + 40h; negativa = `03 7F <servizio> <codice>`.

### Sequenza di collegamento

1. Inizializzazione rapida: linea alta 300 ms, bassa 25 ms, alta 25 ms.
2. `81 13 F7 81 0C` (StartCommunication alla centralina 13h dal tester F7h),
   risposta `03 C1 57 8F AA`. È l'unico messaggio con intestazione.
3. `02 10 A0 B2` (sessione diagnostica), risposta `02 50 A0 F2`.
4. `02 27 01 2A` (richiesta del seme), risposta `04 67 01 <seme alto> <seme basso> <cs>`.
5. `04 27 02 <chiave alta> <chiave bassa> <cs>`, risposta `02 67 02 6B`.

La chiave si calcola dal seme con l'algoritmo pubblicato in
`pajacobson/td5keygen` (licenza BSD a 2 clausole): pochi passaggi di un
registro a scorrimento a 16 bit. Si può riscrivere in Rust citando la fonte.

### Mantenimento e tempi

- `3E` (TesterPresent) almeno ogni 1,5 s; dopo circa 2 s di silenzio la
  centralina chiude la sessione e va rifatto tutto.
- Risposta entro 50–300 ms; almeno 25 ms (consigliati 100) fra una risposta e
  la richiesta successiva.

### Dati in tempo reale (servizio 21h)

Richiesta `02 21 <id> <cs>`, risposta `<lung> 61 <id> <dati…> <cs>`. Tutti i
valori sono big-endian.

| Id | Contenuto | Formula |
|---|---|---|
| 09 | Giri motore | valore |
| 0D | Velocità | km/h |
| 10 | Tensione batteria (e riferimento) | /1000 V |
| 1A | Temperatura liquido, pressione collettore | (v − 2732)/10 °C; /100 kPa |
| 1B | Tracce del pedale acceleratore e alimentazione sensori | /1000 V |
| 1C | Temperatura aria aspirata, temperatura gasolio | decodifica parziale |
| 1E | Interruttori (freno, frizione, cruise, A/C, riduttore) | bit, attivi bassi |
| 23 | Pressione ambiente | /100 kPa |
| 37, 38 | Posizione EGR, wastegate | /100 % |
| 40 | Correzione iniettori per cilindro (5 valori con segno) | valore |

Alcuni identificativi (07, 08, 0A, 0B, 0F, 17, 20) sono segnalati come
inaffidabili: la centralina a volte li rifiuta. La portata d'aria non ha un
identificativo confermato.

### Codici guasto e identificazione

Da `Requests.java` di Td5OpenDiag (licenza Apache 2.0):

- Codici guasto: `02 21 3B`, risposta di 39 byte (una mappa di bit); l'elenco
  dei significati è in `FaultCodes.java` dello stesso progetto.
- Numero di telaio `02 1A 87`, tipo di centralina `02 1A 9A`, variante della
  mappa `02 21 32`.
- Esistono anche la cancellazione dei guasti (`31 DD…`) e le prove degli
  attuatori (`30 …`, `31 C2 …`): **fuori dal perimetro di sola lettura**.

## Perché non con l'ELM327

- L'ELM327 trasmette sempre tre byte di intestazione (o quattro) prima dei
  dati, e ne vuole almeno uno di dati (datasheet ELM327DSH, comando `AT SH`).
  I messaggi del Td5 dopo il primo non hanno intestazione.
- Nel protocollo KWP l'ELM327 scrive da sé la lunghezza nel primo byte, e la
  calcola in modo diverso dal Td5.
- Il mantenimento ogni 1,5 s e l'eco da scartare richiedono il controllo
  diretto della linea.
- Non ho trovato nessun progetto o resoconto verificabile di un Td5 letto
  tramite ELM327. Le due discussioni dove se ne parla (aulro.com,
  landyzone.co.uk) non erano leggibili: protette da verifica anti-bot.

Conclusione: non dimostrato impossibile, ma senza una via documentata. Gli
adattatori con chip STN (OBDLink) hanno comandi per messaggi grezzi che
potrebbero bastare: **da verificare**.

## Quali adattatori funzionano

Dal produttore di un'app commerciale per il Td5 (td5monitor.com, pagina
«Hardware»; è la dichiarazione di un venditore, non una prova indipendente):

| Adattatore | Collegamento | Esito dichiarato |
|---|---|---|
| OBDLink CX | Bluetooth LE | Funziona (Android e iPhone) |
| OBDLink LX | Bluetooth classico | Funziona (Android) |
| Cavo USB «KKL» / «K+DCAN» con chip FTDI FT232RL autentico | USB | Funziona; il più economico |
| Cloni ELM327 economici, Vgate iCar / BLE 4.0, vLinker MC+, Veepeak OBDCheck BLE+ | — | Non funzionano: «il loro chip non riesce a svegliare la centralina» |

- Gli OBDLink usano un chip STN, che oltre ai comandi ELM327 ha protocolli
  propri: `STP 21` «ISO 9141 (no header, no autoinit)» e `STP 23` «ISO 14230
  (no autoinit)», senza mantenimento automatico (scheda «OBDLink MX Protocol
  Commands», scantool.net). È con ogni probabilità ciò che permette i messaggi
  senza intestazione del Td5. La sequenza esatta dei comandi per il Td5 non è
  pubblica: **da ricavare con l'adattatore in mano**.
- Per OBDio su Windows: l'OBDLink CX usa il profilo BLE che l'app riconosce
  già; l'OBDLink LX comparirebbe come porta COM. L'OBDLink MX+ ha lo stesso
  tipo di chip ma non è citato dalla fonte: non verificato.
- Il Carista si identifica come «ELM327 v1.5»: rientra fra gli adattatori
  ELM327 compatibili, non fra quelli con chip STN.

## Che cosa servirebbe

- Un cavo USB «KKL» (venduto come «VAG KKL 409.1»), meglio con chip FTDI:
  è quello che usano TD5Tester e Td5OpenDiag.
- In OBDio, un secondo «dialogo» accanto a quello ELM327, sulla porta seriale:
  1. inizializzazione rapida con i tempi della linea (segnale di break di
     25 ms) e velocità di 10 400 baud;
  2. messaggi con checksum e scarto dell'eco;
  3. accesso di sicurezza;
  4. mantenimento della sessione;
  5. elenco chiuso di letture (tabella sopra, codici guasto, identificazione);
  6. una centralina Td5 simulata per le prove.
- Da decidere: la lettura del Td5 richiede uno scambio di accesso di sicurezza
  e richieste fuori dall'elenco OBD standard. Resta sola lettura, ma allarga
  la regola iniziale del progetto.

Rischi da verificare sul veicolo: precisione dei 25 ms con un convertitore
USB (il chip FTDI ha una latenza predefinita di 16 ms), velocità non standard
sui cavi con chip CH340, varianti di centralina (MSB/NNN) con risposte diverse.

## Stato in OBDio

Modalità «Land Rover Td5 (OBDLink CX/LX)», scelta nella pagina Connessione e
salvata in `configurazione/veicolo.json`. Codice in `src/td5/`; prove in
`src/td5/*` (unitarie) e `tests/td5_simulato.rs` (thread di lavoro reale).
**Mai provata su un adattatore o su un veicolo.**

### Che cosa c'è

- Messaggi con lunghezza e checksum; lettura tollerante della risposta (con o
  senza eco, con o senza checksum, con o senza lunghezza), perché il formato
  esatto della stampa di un OBDLink non è noto.
- Algoritmo seme/chiave riscritto da td5keygen (licenza in
  [LICENSE-THIRD-PARTY.md](../LICENSE-THIRD-PARTY.md)); vettori di prova
  calcolati a parte con una trascrizione in Python di `keygen.c` (34A5 → 54D3
  come nel README di td5keygen; 0000 → 0001, FFFF → 8081, 8091 → 9AAC).
- Elenco chiuso delle richieste (`RichiestaTd5`): `10 A0`, `27 01`,
  `27 02 <chiave>` (la chiave si ottiene solo dal seme), `3E 01`, `21 <id>`
  per gli identificativi 09, 0D, 10, 1A, 1B, 1C, 1E, 23, 37, 38, 40, `21 3B`
  (guasti), `1A 87` (telaio), `1A 9A` (tipo di centralina), `21 32` (variante
  della mappa). Cancellazione dei guasti, prove degli attuatori, scrittura e
  programmazione non esistono nel codice; un test verifica i servizi.
- Mantenimento: TesterPresent dopo 1 s di silenzio, dal thread di lavoro
  quando non c'è lettura in corso. Se una richiesta non ha risposta positiva
  e nemmeno TesterPresent ce l'ha, la sessione viene riaperta una volta
  (inizializzazione, sessione, chiave) e la richiesta ripetuta; se non
  riesce, il collegamento si chiude con un errore esplicito.
- I valori passano per la strada di sempre (campioni con stato, cruscotto,
  grafici, registrazione `.obdio.jsonl` e CSV, riproduzione, MQTT, MCP). Dove
  il significato coincide si usano le chiavi OBD-II (giri, velocità,
  temperatura del liquido e dell'aria aspirata, pressione del collettore e
  barometrica, tensione); i valori solo del Td5 hanno chiavi `td5_…`
  (temperatura del gasolio, quattro tracce del pedale e alimentazione dei
  sensori, posizione EGR e wastegate, correzione dei cinque iniettori,
  interruttori). Al collegamento ogni identificativo viene letto una volta:
  un parametro che la centralina non fornisce non compare, e uno che non
  risponde ha lo stato «mancante», mai zero.
- Diagnosi: guasti dalla mappa di 35 byte con i nomi di `FaultCodes.java`
  (Td5OpenDiag, Apache 2.0), traduzione italiana solo dove il significato è
  chiaro; tipo di centralina, variante della mappa, tensione della batteria.
  Il numero di telaio resta solo in memoria come per l'OBD-II. «Cancella
  codici guasto» non c'è, per scelta.
- Adattatore senza chip STN (`STI` e `STDI` rispondono «?»): errore che
  spiega che un ELM327 non può leggere il Td5 e suggerisce OBDLink CX/LX.
- Stessa sequenza via BLE (OBDLink CX, profilo FFF0) e via porta seriale
  (OBDLink LX): il dialogo sta sopra il tratto `Transport`.
- «Avvia la demo Td5» con un simulatore (fascia MODALITÀ DEMO).

### Le tre strategie

Comuni: `ATZ`, `ATE0`, `STI`, `STDI`, `ATL0`, `ATS1`, `ATH1`. Poi, in ordine
finché una ottiene `50 A0` (fra un tentativo e l'altro 2,5 s di silenzio,
così la centralina chiude una sessione rimasta a metà):

| | Comandi |
|---|---|
| A | `STP 21`, `STPBR 10400`, `STPTO 500`, `STPTRQ 50`, `STIP1X 40`, `STIFI 25, 25, 8113F7810C`, poi richieste come `0210A0` |
| B | `STP 25`, gli stessi tempi, `STIFI …`, poi `STP 21`, `STPTO 500`, `STPTRQ 50` e richieste come `0210A0` |
| C | `STP 25`, gli stessi tempi, `ATSW 00` (se rifiutato `ATWM 023E01`), `STIFI …`, poi `STPX h:0210A0, d:, r:1`; la chiave come `STPX h:042702, d:KHKL, r:1` |

`STIFI` è riuscito se fra i byte stampati c'è C1 (o, per prudenza, se
risponde solo «OK»). La strategia che funziona viene ricordata per i
ripristini. Ogni comando e ogni risposta grezza vanno nel registro dei
collegamenti (`registro_collegamento.log`) e nel registro dell'app, per
tutta la sessione, esclusi la lettura a ripetizione e i mantenimenti
riusciti; la risposta a `1A 87` (telaio) è sempre omessa.

### Che cosa non è verificato (da provare con l'adattatore)

- Se `STIFI` funziona con `STP 21` (il manuale lo dà solo per ISO 14230 con
  inizializzazione rapida) e che cosa stampa: con l'esempio del manuale
  (`C1 E9 8F BD`) non si capisce se lunghezza e checksum compaiano.
- Se con `STP 21` una richiesta esadecimale va sulla linea così com'è con il
  checksum aggiunto, e come l'adattatore capisce la fine della risposta
  (pausa fra i byte, `STIP1X`).
- Se la sessione regge al cambio `STP 25` → `STP 21` (strategia B).
- Se con `STPX` in ISO 14230 l'adattatore riscrive i bit di lunghezza del
  primo byte, come fa l'ELM327 con l'intestazione (strategia C), e se
  `ATSW 00` spegne davvero il suo mantenimento.
- La risposta a `3E 01`: OBDio accetta qualunque risposta che inizi con 7E.
- Ordine dei bit nella mappa dei guasti (qui il bit 0 è il meno
  significativo) e significato di «(L)» e «(C)».
- Formato delle risposte a `1A 9A`, `21 32` e `1A 87` (mostrate come testo se
  stampabili, altrimenti in esadecimale).
- Le decodifiche scelte dove le fonti non sono d'accordo (sotto).

### Dove le fonti non sono d'accordo

- **1A**: proto.md legge la pressione del collettore ai byte 2–3 (6,98 kPa
  nell'esempio a motore spento: impossibile); Td5OpenDiag legge quattro
  temperature (byte 0–1, 4–5, 8–9, 12–13). OBDio usa liquido, aria aspirata e
  gasolio come Td5OpenDiag, che corrispondono ai valori dello strumento citati
  in proto.md per la stessa trama (23,2 °C e 64,0 °C). La temperatura dei
  byte 8–9 (aria esterna, solo su alcune versioni) non è mostrata.
- **1C**: proto.md legge aria aspirata (un byte / 10) e gasolio (− 20);
  Td5OpenDiag la pressione del collettore (byte 0–1, 102,17 kPa
  nell'esempio: plausibile) e la portata d'aria (byte 4–5). OBDio mostra solo
  la pressione del collettore; la portata resta fuori (proto.md la dice
  ignota).
- **1B**: proto.md quattro tracce e alimentazione (10 byte), Td5OpenDiag tre
  tracce e alimentazione (8 byte): OBDio sceglie in base alla lunghezza.
- **0D**: un byte in km/h (proto.md e Td5OpenDiag); pid.txt dice due byte in
  miglia orarie.
- **Mantenimento**: proto.md `01 3E 3F`, Td5OpenDiag `02 3E 01 41`: OBDio usa
  il secondo.
- **Checksum degli esempi**: in proto.md le trame d'esempio di 09, 1B e 1C
  hanno il checksum sbagliato (la somma dà 58, 6B e 11 invece di 57, 48 e
  F3); le prove usano quelle corrette e verificano che le originali vengano
  rifiutate.
- **pid.txt** riporta codici guasto con il servizio 18 e la cancellazione con
  il 14: OBDio segue Td5OpenDiag (`21 3B`), e nessuna cancellazione.

### Dove si aggancerebbe un cavo KKL

Il tratto `CanaleTd5` (`src/td5/stn.rs`) separa i messaggi del Td5 dal modo
in cui raggiungono il filo. Un cavo «KKL» (linea K su porta seriale) sarebbe
un'altra realizzazione dello stesso tratto: inizializzazione rapida con il
segnale di break, 10 400 baud, scarto dell'eco. Non c'è.

## Fonti

- SimonRafferty, «Land-Rover-Td5-Arduino-Diagnostics» (MIT):
  <https://github.com/SimonRafferty/Land-Rover-Td5-Arduino-Diagnostics>
- pajacobson, «td5keygen» (BSD-2-Clause):
  <https://github.com/pajacobson/td5keygen>
- TD5Monitor, pagina «Hardware»: <https://td5monitor.com/hardware>
- OBDLink MX Protocol Commands:
  <https://www.scantool.net/downloads/118/mx_protocol_commands.pdf>
- Td5OpenDiag, «Td5OpenDiag-android» (Apache-2.0):
  <https://github.com/Td5OpenDiag/Td5OpenDiag-android>
- hairyone, «TD5Tester» (Apache-2.0): <https://github.com/hairyone/TD5Tester>
- EA2EGA, «Ekaitza_Itzali» (catture del traffico, senza licenza: solo
  consultazione): <https://github.com/EA2EGA/Ekaitza_Itzali>
- Datasheet ELM327DSH: <https://www.scantool.net/scantool/downloads/103/elm327dsh.pdf>
