# OBDio — specifica di design

Riferimento per chi implementa l'interfaccia. L'app è uno strumento da
officina: si usa in piedi accanto all'auto, con un portatile o (in futuro) un
telefono, spesso con le mani occupate e la luce sbagliata. Tre conseguenze:

1. **Leggibile a colpo d'occhio**: un valore si deve leggere da un metro.
2. **Bersagli grandi**: nulla di importante sotto i 44 px di lato su schermi
   stretti.
3. **Mai un dato inventato**: un valore assente si mostra come assente, con il
   motivo. Non esistono zeri di ripiego, nemmeno sugli strumenti analogici.

Postura del prodotto: *sovrana* durante una sessione di diagnosi (attenzione
piena per minuti o ore) → interfaccia densa ma calma, colore riservato ai dati
e agli stati.

## 1. Token visivi

I valori derivano dalla scala Tailwind (zinc per i neutri), tradotti in egui.
Vanno definiti una sola volta in `src/ui/tema.rs` e usati ovunque: nessun
colore o misura scritti a mano nelle pagine.

### Colori (tema scuro, unico)

| Token | Tailwind | Hex | Uso |
|---|---|---|---|
| `sfondo` | zinc-950 | `#09090B` | Fondo dell'app |
| `superficie` | zinc-900 | `#18181B` | Pannelli, schede, strumenti |
| `superficie_alta` | zinc-800 | `#27272A` | Elementi sopra una superficie, hover |
| `bordo` | zinc-800 | `#27272A` | Separazione sottile fra superfici |
| `bordo_forte` | zinc-700 | `#3F3F46` | Bordi di controlli, tacche secondarie |
| `testo` | zinc-100 | `#F4F4F5` | Testo e valori principali |
| `testo_2` | zinc-400 | `#A1A1AA` | Etichette, unità, testo di supporto |
| `testo_3` | zinc-500 | `#71717A` | Note, tacche, stati inattivi |
| `accento` | sky-400 | `#38BDF8` | **Unico** colore d'accento: selezione, valore corrente, azione primaria |
| `ok` | emerald-400 | `#34D399` | Stato positivo |
| `attenzione` | amber-400 | `#FBBF24` | Soglia di attenzione, dato mancante |
| `pericolo` | rose-500 | `#F43F5E` | Soglia critica, errori, azioni distruttive |
| `demo` | orange-500 | `#F97316` | Solo per la fascia e le etichette della modalità demo/riproduzione |

Regola 60-30-10: 60 % neutri, 30 % superfici, 10 % accento e stati. Il colore
porta sempre un significato; non si usa per decorare.

Serie nei grafici (in quest'ordine): sky-400 `#38BDF8`, amber-400 `#FBBF24`,
emerald-400 `#34D399`, pink-400 `#F472B6`, violet-400 `#A78BFA`,
teal-300 `#5EEAD4`, orange-400 `#FB923C`, lime-400 `#A3E635`.

### Misure

| Token | Valore | Uso |
|---|---|---|
| Spaziatura | 4, 8, 12, 16, 24 px | Solo questi passi |
| Raggio | 8 px schede e strumenti, 6 px pulsanti e campi | |
| Bordo | 1 px | 2 px solo per lo strumento selezionato in modifica |
| Bersaglio minimo | 44 px su layout stretto, 32 px su largo | |
| Tipografia | 12 nota · 14,5 corpo · 19 titolo di sezione · 28–64 valore degli strumenti | Massimo tre misure per vista, esclusi gli strumenti |

I numeri degli strumenti usano cifre a larghezza fissa (font monospace di egui)
perché il valore non «balli» mentre cambia.

## 2. Struttura e navigazione

Sei destinazioni, sempre le stesse:

| Destinazione | Contenuto |
|---|---|
| **Connessione** | BLE, seriale, demo, riproduzione di una sessione |
| **Diagnosi** | DTC con descrizione, MIL, prontezza, fermo immagine, Mode 06, cancellazione codici |
| **Cruscotto** | Pagine di strumenti personalizzabili |
| **Sensori** | Elenco di tutti i parametri supportati, valori dal vivo, un grafico per sensore |
| **Sessioni** | Registrazioni salvate, importazione, esportazione |
| **Impostazioni** | MQTT, unità, cartelle, informazioni |

Layout adattivo in base alla larghezza disponibile:

- **Largo (≥ 900 px)**: barra laterale sinistra con icona e nome; contenuto a
  destra; ispettori e pannelli secondari affiancati.
- **Stretto (< 900 px)**: barra di navigazione in basso con sole icone e nome
  breve; contenuto a colonna singola; ispettori come foglio dal basso.

Una **barra di stato** in alto è sempre visibile: stato del collegamento,
registrazione in corso con durata, pubblicazione MQTT, e la fascia arancione
quando i dati non vengono da un veicolo (demo o riproduzione).

## 3. Strumenti del cruscotto

Ogni strumento mostra **un** parametro. Tipi:

| Tipo | Quando usarlo |
|---|---|
| **Quadrante** | Grandezze con un campo noto e zone significative: giri, velocità, temperatura |
| **Arco** | Come il quadrante ma compatto (180°), per riquadri bassi |
| **Barra** | Percentuali e correzioni; orizzontale o verticale; con zero centrale per i valori con segno |
| **Numerico** | Valore grande con unità, minimo e massimo della sessione |
| **Grafico** | Andamento degli ultimi N secondi dello stesso parametro |
| **Contachilometri** | Distanza a cifre «a rulli»: totale se la centralina lo espone, altrimenti parziale calcolato |
| **Spia** | Stato acceso/spento: MIL, soglia superata, registrazione |

### Anatomia del quadrante

- Sviluppo di **270°**, da 135° a 405° (apertura in basso).
- Arco di fondo in `bordo_forte`; arco del valore in `accento` dallo zero al
  valore corrente.
- Zone: arco esterno sottile in `attenzione` e `pericolo` sugli intervalli
  configurati (per esempio la zona rossa dei giri).
- Tacche principali con numero (5–10 secondo l'ampiezza), tacche secondarie
  senza numero. I numeri usano un passo «tondo» (1, 2, 2,5, 5 × 10ⁿ).
- Lancetta sottile con perno; il valore numerico grande sta nella metà
  inferiore, con l'unità sotto in `testo_2`.
- Titolo in alto in `testo_2`.
- **Marcatore di picco** opzionale: piccolo triangolo sul massimo raggiunto.
- Quando il valore supera una soglia, il numero e l'arco del valore prendono
  il colore della zona. Nient'altro lampeggia.
- La lancetta si muove con uno smorzamento breve (≈150 ms) verso il valore
  nuovo; non anticipa e non oscilla.

### Stati di ogni strumento

| Stato | Aspetto |
|---|---|
| In attesa | Scala disegnata, nessuna lancetta, «—» al posto del valore |
| Dato valido | Come sopra |
| Dato vecchio (> 3 s) | Valore e lancetta in `testo_3`, nota «fermo da N s» |
| Risposta mancante / errore | Nessuna lancetta, «—», stato scritto in `attenzione` o `pericolo` |
| Non supportato | Strumento attenuato con «Non supportato da questa centralina» |
| Selezionato (modifica) | Bordo 2 px `accento`, maniglia di ridimensionamento |

## 4. Personalizzazione del cruscotto

- Il cruscotto ha **pagine** (schede in alto); ogni pagina è una **griglia**:
  12 colonne su layout largo, 4 su stretto, celle quadrate. Ogni strumento
  occupa un rettangolo di celle.
- **Uso** e **Modifica** sono due modi distinti e il modo corrente è sempre
  evidente: in modifica compaiono la griglia punteggiata, una barra
  «Modifica del cruscotto» e i pulsanti *Aggiungi strumento*, *Salva*,
  *Annulla*.
- In modifica: trascinare sposta lo strumento (aggancio alla griglia),
  la maniglia nell'angolo lo ridimensiona, un tocco lo seleziona e apre
  l'**ispettore** (a destra su largo, dal basso su stretto).
- Ogni gesto ha un'alternativa visibile: nell'ispettore posizione e misura si
  possono cambiare con pulsanti e campi numerici.
- Ispettore: parametro, tipo di strumento, titolo, unità mostrata, minimo e
  massimo della scala, soglia di attenzione e soglia critica (con verso:
  sopra o sotto), decimali, marcatore di picco, durata per il tipo Grafico.
  I valori predefiniti derivano dal parametro scelto (per i giri: 0–8.000,
  rosso oltre 6.500), così uno strumento appena aggiunto è già sensato.
- Lo strumento sovrapposto a un altro non si può rilasciare: il rettangolo di
  destinazione diventa `pericolo` e il rilascio riporta alla posizione
  d'origine.
- *Annulla* riporta la pagina all'ultimo salvataggio: per questo aggiungere,
  spostare ed eliminare non chiedono conferma.
- I cruscotti si **esportano e importano** come file JSON.
- Modelli pronti: «Guida» (giri, velocità, temperatura, carico,
  contachilometri), «Motore» (pressione collettore, anticipo, temperature,
  tensione), «Diagnosi P0420» (sonde, correzioni, giri).
- Il cruscotto interroga da solo i parametri usati dagli strumenti della
  pagina visibile.

## 5. Azioni distruttive: cancellazione dei codici

È l'unica azione dell'app che modifica lo stato della centralina e **non si
può annullare**: qui la conferma è giustificata.

- Pulsante *Cancella codici guasto…* in stile `pericolo`, lontano da
  *Leggi diagnosi*.
- Finestra di conferma che dice cosa verrà cancellato (codici memorizzati e
  in sospeso, fermo immagine, risultati dei monitor, stato di prontezza) e
  cosa no (codici permanenti), con le condizioni richieste (quadro acceso,
  motore spento) e l'avvertenza sulla revisione.
- Il pulsante di conferma si abilita solo dopo la spunta «Ho letto e voglio
  cancellare i codici». Il pulsante predefinito è *Annulla*.
- Prima di cancellare l'app salva automaticamente una copia della diagnosi
  corrente: è il modo per non perdere il fermo immagine.
- Dopo la cancellazione la diagnosi viene riletta e l'esito è mostrato in
  chiaro: riuscita, rifiutata dalla centralina (con il motivo), o errore.

## 6. Riproduzione

Una sessione registrata si può «rivivere»: i campioni scorrono nel tempo e
alimentano cruscotto, sensori e grafici come se arrivassero dall'auto.

- Barra di trasporto sempre visibile durante la riproduzione: riproduci/pausa,
  posizione trascinabile con i marcatori della sessione, velocità
  (0,5×, 1×, 2×, 5×, 10×), ripetizione.
- La fascia arancione dice «RIPRODUZIONE — dati registrati il …» finché dura.
- Durante la riproduzione non si può registrare né cancellare codici.

## 7. Principi di scrittura

Testi in italiano, brevi, al presente. I pulsanti dicono l'azione
(«Leggi diagnosi», non «OK»). Gli errori dicono cosa è successo e cosa fare.
Nessun gergo ELM327 nei testi per l'utente: «La centralina non risponde»,
non «NO DATA».
