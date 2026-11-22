# Pubblicazione MQTT

OBDio può pubblicare in tempo reale su un broker MQTT i dati che legge
dall'adattatore (o dal simulatore). Si configura dalla scheda
**Impostazioni**; il file è `configurazione\mqtt.json` accanto all'eseguibile.
Protocollo: MQTT 3.1.1 su TCP, **senza cifratura**.

## Argomenti

Con prefisso `obdio` e veicolo `auto` (entrambi modificabili; il nome del
veicolo usa solo `a-z`, `0-9`, `_`, `-`: il resto diventa `_`):

| Argomento | Trattenuto | Contenuto |
|---|---|---|
| `obdio/auto/stato` | sì | JSON di stato, vedi sotto |
| `obdio/auto/sensori/<chiave>` | no | Il solo numero, in testo, con il punto decimale |
| `obdio/auto/istantanea` | no | JSON con l'ultimo stato di ogni parametro |
| `obdio/auto/diagnosi` | sì | JSON della diagnosi, a ogni lettura |
| `obdio/auto/evento` | no | Marcatori e cancellazione dei codici guasto |

Le chiavi dei sensori sono quelle del catalogo (`giri`, `velocita`,
`temp_refrigerante`, `o2s1_lambda`, …); la pagina **Sensori** ne mostra i nomi.

### Stato

```json
{"stato":"online","origine":"demo_sintetico","app":"OBDio","versione":"0.2.0"}
```

- `origine` è `veicolo` per un adattatore reale, `demo_sintetico` per il
  simulatore (i dati della demo non descrivono un veicolo) e `riproduzione`
  mentre si rivive una sessione registrata: sono dati del passato, mai
  pubblicati come letture attuali. Lo stesso valore compare nell'istantanea e
  negli eventi. Durante la riproduzione la diagnosi registrata non viene
  pubblicata su `…/diagnosi`.
- Il messaggio è `online` solo mentre c'è un collegamento attivo con
  l'adattatore (o con il simulatore). Senza collegamento, o a MQTT spento, è
  `{"stato":"offline"}`.
- **Ultima volontà**: il broker pubblica `{"stato":"offline"}` (trattenuto) se
  OBDio sparisce senza salutare. Alla chiusura pulita lo stesso messaggio è
  pubblicato esplicitamente prima della disconnessione.

### Sensori

`obdio/auto/sensori/giri` → `2128`. Solo letture valide: un valore assente
(risposta mancante, parametro non supportato, errore) **non viene pubblicato**
e non è mai sostituito da uno zero. Al massimo un messaggio ogni *intervallo per
sensore* (predefinito 500 ms) per ciascuna chiave.

### Istantanea

Al massimo ogni *intervallo dell'istantanea* (predefinito 1000 ms):

```json
{
  "ts": "2026-11-21T10:15:30.123+01:00",
  "origine": "demo_sintetico",
  "valori": {
    "giri": {"valore": 2128.0, "unita": "giri/min", "stato": "ok", "ecu": "7E8"},
    "o2s2_tensione": {"valore": null, "unita": "V", "stato": "risposta_mancante", "ecu": ""}
  }
}
```

Qui un valore assente è `null` e `stato` ne dice il motivo: `ok`,
`non_supportato`, `risposta_mancante`, `errore_comunicazione`. `ecu` è vuoto se
nessuna centralina ha risposto.

### Diagnosi

È lo stesso referto che l'app salva nelle sessioni (campi `dtc`, `prontezza`,
`fermo`, `mode06`, …), ripubblicato e trattenuto a ogni lettura e dopo la
cancellazione dei codici. **Il VIN non è incluso**: non fa parte del referto
serializzato e un test lo verifica.

### Evento

```json
{"ts":"2026-11-21T10:16:02.001+01:00","tipo":"marcatore","etichetta":"Minimo","origine":"veicolo"}
{"ts":"2026-11-21T10:20:00.500+01:00","tipo":"codici_cancellati","esito":"accettata","dettaglio":"7E8: cancellazione eseguita","origine":"veicolo"}
```

`esito` è `accettata`, `rifiutata`, `nessuna_risposta` o `errore`.

## Home Assistant

Con l'opzione **Annuncia i sensori a Home Assistant** OBDio pubblica, per ogni
sensore la prima volta che ne vede un valore valido (e a ogni riconnessione), una
configurazione trattenuta su
`homeassistant/sensor/obdio_<veicolo>_<chiave>/config`:

```json
{
  "name": "Giri motore",
  "unique_id": "obdio_auto_giri",
  "has_entity_name": true,
  "state_topic": "obdio/auto/sensori/giri",
  "unit_of_measurement": "giri/min",
  "state_class": "measurement",
  "availability_topic": "obdio/auto/stato",
  "availability_template": "{{ value_json.stato }}",
  "payload_available": "online",
  "payload_not_available": "offline",
  "device": {"identifiers": ["obdio_auto"], "name": "OBDio auto",
             "manufacturer": "OBDio", "sw_version": "0.2.0"}
}
```

`device_class` è aggiunto solo dove non c'è ambiguità: `temperature` (°C),
`voltage` (V), `pressure` (kPa), `speed` (km/h), `distance` (km). La
disponibilità legge il campo `stato` del JSON di stato, quindi i sensori
risultano non disponibili senza collegamento e quando OBDio si chiude.

I sensori compaiono in Home Assistant dopo che sono stati letti almeno una
volta: scegli i parametri nella pagina **Sensori** e avvia la lettura. Il
prefisso di discovery (`homeassistant`) va cambiato solo se lo si è cambiato in
Home Assistant. Le configurazioni di discovery sono trattenute: per toglierle
bisogna cancellarle dal broker.

## Frequenza, coda e messaggi scartati

- Nessun messaggio viene mai scritto dal thread di lavoro direttamente sulla
  rete: finisce in una coda di 256 posti svuotata da un thread dedicato. Se la
  coda è piena, o il broker non è raggiungibile, i messaggi dei sensori,
  dell'istantanea e degli eventi sono **scartati e contati**: la lettura dei
  dati non rallenta e non si ferma.
- Stato, diagnosi e configurazioni di discovery sono ricordati in memoria e
  **rinviati a ogni riconnessione**, così il broker ha sempre l'ultimo valore.
- La scheda Impostazioni mostra lo stato (disattivato, connessione in corso,
  connesso, errore), i messaggi accodati e scartati e l'ultimo errore; la barra
  superiore mostra un indicatore compatto.
- Se la connessione cade, OBDio riprova da solo con pause crescenti da 1 a 30
  secondi.
- Alla chiusura dell'app, o spegnendo MQTT, l'attesa è limitata a un paio di
  secondi anche con un broker che non risponde.

## Sicurezza

- **Nessuna cifratura.** TLS non è supportato in questa versione: nome utente,
  password e dati viaggiano in chiaro sulla rete. Usa un broker nella tua rete
  locale e una password che non usi altrove.
- **La password non viene scritta su disco**, salvo la spunta *Ricorda la
  password (salvata in chiaro)*: in quel caso finisce, senza protezione, in
  `mqtt.json`, leggibile da chiunque abbia accesso al PC. Senza la spunta va
  reinserita a ogni avvio.
- **Il VIN non viene mai pubblicato**, né negli argomenti né nei payload.

## Come è stato verificato

Test automatici con un piccolo broker MQTT scritto per i test (nello stesso
processo, su 127.0.0.1) e il thread di lavoro vero con il simulatore: ultima
volontà e credenziali, stato trattenuto, sensori numerici, istantanea con
`null`, diagnosi senza VIN, discovery, QoS 1, caduta e ritorno del broker con il
polling che continua, chiusura con un broker muto. **Non provato** con un broker
reale (per esempio Mosquitto) né con Home Assistant: la compatibilità della
discovery segue la documentazione di Home Assistant ma non è stata verificata lì.
