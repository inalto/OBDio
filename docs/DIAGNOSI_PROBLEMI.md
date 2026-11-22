# Diagnosi dei problemi

Che cosa fare quando OBDio si chiude da solo o non riesce a collegarsi
all'adattatore, e quali file inviare a chi ti assiste.

## I due registri

OBDio scrive due file di testo nella cartella `registri`, accanto a
`OBDio.exe` (se quella cartella non è scrivibile, in
`Documenti\OBDio\registri`; in ultima istanza nella cartella temporanea,
`%TEMP%\OBDio\registri`). Il percorso esatto è nelle **Impostazioni →
Registri**, con i pulsanti **Apri la cartella** e **Copia l'ultimo errore**.

| File | Che cosa contiene |
|---|---|
| `registro_errori.log` | Una voce per ogni errore interno (panic): data e ora, versione di OBDio, versione di Windows, thread, punto del codice, messaggio, indirizzo di caricamento dell'eseguibile e backtrace. La voce viene scritta e sincronizzata sul disco **prima** che il processo si chiuda, anche quando l'errore nasce in un componente di sistema (Bluetooth/WinRT) e Windows chiude l'app all'istante. Oltre 1 MB passa a `registro_errori.1.log`. |
| `registro_collegamento.log` | Ogni passo di ogni tentativo di collegamento, con l'orario al millisecondo: scansione avviata e fermata, dispositivo scelto (nome, indirizzo, RSSI, servizi annunciati), stato della radio, connessione radio (tentativi, durata, esito), ogni servizio e caratteristica GATT con le proprietà, profilo riconosciuto o no, MTU, attivazione delle notifiche, ogni scambio ELM327 della sequenza iniziale, protocollo trovato, disconnessioni con il motivo, e i messaggi interni della libreria Bluetooth (`[btleplug …]`). Ogni riga va sul disco appena scritta: se l'app si chiude, l'ultima riga dice a che punto era. Oltre 256 kB passa a `registro_collegamento.1.log`. |

Nessuno dei due contiene il VIN: la risposta alla richiesta del VIN è omessa
come nel registro degli scambi dell'app.

Gli stessi passi BLE compaiono anche dal vivo nel **Registro degli scambi con
l'adattatore**, in fondo alla pagina **Connessione** (righe che iniziano con
`BLE`), con **Copia registro**.

### Avviso al riavvio

Se l'ultima volta OBDio si è chiuso per un errore registrato, al riavvio
compare in alto l'avviso «L'ultima volta OBDio si è chiuso per un errore» con
**Apri il registro**, **Apri la cartella** e **Chiudi**. L'avviso non blocca
nulla e non ricompare dopo **Chiudi** o dopo una chiusura regolare dell'app.

Quando l'errore nasce in un thread di sistema (per esempio nel Bluetooth di
Windows), prima della chiusura compare anche una finestra «OBDio — errore
interno» con il percorso del registro. Un errore interno durante un
collegamento o una lettura non chiude più l'app: compare «Errore interno
durante …: il collegamento è stato chiuso» e si può riprovare subito.

## Come inviare i registri

1. **Impostazioni → Registri → Apri la cartella**.
2. Comprimi la cartella `registri` (clic destro → *Comprimi in file ZIP*) e
   inviala, oppure allega i due file `.log`.
3. Aggiungi due righe: che cosa stavi facendo e verso che ora.

Per un messaggio veloce basta **Copia l'ultimo errore** e incollarlo.

## Diagnosi Bluetooth da terminale

`OBDio.exe` ha una modalità a riga di comando che fa scansione, connessione e
elenco dei servizi GATT **con lo stesso codice dell'app**, senza aprire la
finestra. Non invia alcun comando ELM327 e non scrive nulla sull'adattatore
(nemmeno l'attivazione delle notifiche).

1. Adattatore nella presa OBD, quadro su ON, app del telefono chiuse.
2. Apri il **Prompt dei comandi** nella cartella di `OBDio.exe` (in Esplora
   risorse scrivi `cmd` nella barra dell'indirizzo e premi Invio).
3. Esegui:

   ```text
   start /wait OBDio.exe --diagnosi-ble carista
   ```

   Il nome (o l'indirizzo, per esempio `00:11:22:33:44:55`) è facoltativo:
   senza, viene scelto il primo dispositivo che sembra un adattatore OBD.
   In PowerShell: `Start-Process -Wait .\OBDio.exe '--diagnosi-ble','carista'`.

Dopo circa 15–40 secondi il resoconto è salvato in
`diagnosi_ble_<data>_<ora>.txt` accanto a `OBDio.exe` (o nella cartella dei
registri). L'eseguibile non ha una console sua: le righe compaiono nel
terminale da cui è stato avviato, ma il file c'è comunque. Invia il file
insieme alla cartella `registri`.

Dai sorgenti, lo stesso strumento è `cargo run --example diagnosi_ble -- carista`.

## Problemi di collegamento frequenti

| Che cosa vedi | Che cosa vuol dire | Che cosa provare |
|---|---|---|
| L'adattatore non compare nella scansione | Spento, fuori portata, o già collegato a un altro dispositivo | Quadro su ON; chiudi l'app del telefono (o spegni il Bluetooth del telefono); avvicina il PC. |
| «Adattatore non raggiungibile» — *connessione radio: nessuna risposta entro 15 s* | L'adattatore non ha accettato la connessione Bluetooth LE. OBDio ritenta una volta da solo | Come sopra. Se l'adattatore è già associato in Windows ma «non connesso», prova a rimuoverlo e riassociarlo, oppure usa la porta seriale (COM). |
| «L'adattatore rifiuta l'accesso Bluetooth» (accesso negato, errore di protocollo GATT) | Il dispositivo risponde ma non concede i suoi servizi: molti adattatori Classic + LE, come il Carista, li aprono solo dopo l'associazione | Associa l'adattatore in *Impostazioni → Bluetooth e dispositivi → Aggiungi dispositivo* e riprova; oppure, dopo l'associazione, usa il collegamento seriale con la porta COM in uscita (README, «Porta seriale»). |
| «Profilo BLE sconosciuto» | Collegato, ma i servizi non sono fra i profili documentati: l'app non scrive nulla | **Copia elenco UUID** (o la diagnosi da terminale) e invia il testo; intanto prova la porta seriale. |
| «Scoperta dei servizi GATT» o «attivazione delle notifiche» senza risposta | La connessione radio c'è ma il dispositivo non completa il passo indicato | Spesso è l'associazione mancante (vedi sopra) o un'app del telefono che tiene l'adattatore. |
| «L'adattatore non risponde ai comandi» | Il canale è aperto ma non risponde come un ELM327 | Scollega e ricollega l'adattatore dalla presa; controlla che non sia in uso da un'altra app. |
| «La centralina non risponde» | L'adattatore funziona ma l'auto no | Quadro su ON o motore acceso; alcune auto rispondono solo a motore acceso. |
| «Errore interno durante …» | Un errore nel programma: il collegamento è stato chiuso, l'app resta aperta | Riprova; se si ripete invia i registri. |
| L'app si chiude da sola | Errore registrato in `registro_errori.log` (al riavvio compare l'avviso) | Invia la cartella `registri`. |

## Per chi analizza i registri

- Le build di rilascio hanno le tabelle delle righe nel file
  `obdio_desktop.pdb` (circa 48 MB), che il pacchetto non contiene:
  l'eseguibile non cresce. Copiando il `.pdb` **della stessa build** accanto a
  `OBDio.exe`, il backtrace riporta funzioni, file e righe. Senza, i frame
  sono indirizzi: con la riga «Indirizzo di caricamento dell'eseguibile» e il
  `.pdb` conservato di quella versione si risolvono a posteriori (indirizzo −
  base = RVA). Conviene quindi conservare il `.pdb` di ogni rilascio.
- Un panic in un callback di sistema lascia **due** voci consecutive: quella
  vera e, subito dopo, «panic in a function that cannot unwind», scritta dal
  runtime di Rust un istante prima dell'abort.
- La variabile d'ambiente `OBDIO_REGISTRI=<cartella>` sposta i registri
  (prove automatiche); `OBDIO_SENZA_AVVISO=1` toglie la finestra di sistema.
