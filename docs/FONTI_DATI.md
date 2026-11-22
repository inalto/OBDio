# Fonti dei dati

## Descrizioni dei codici guasto (DTC)

L'app incorpora l'elenco dei codici guasto OBD-II generici in `data/dtc_generici.tsv`
(una riga per codice: `CODICE<TAB>titolo inglese`, UTF-8, fine riga LF). Il file è
incluso nell'eseguibile con `include_str!` in `src/obd/dtc.rs`.

- **Origine**: progetto OBDex, <https://github.com/foerbsnavi/OBDex> (elenco «generic»).
- **Licenza dei dati**: CC0-1.0 (pubblico dominio), nessun obbligo di attribuzione.
- **Contenuto**: 9.533 codici (P, B, C, U). Si usa solo il titolo inglese; descrizioni
  estese, cause e riparazioni dell'archivio originale non sono incluse.
- **Lingua**: l'app mostra una descrizione italiana per ogni codice, scelta in
  quest'ordine:
  1. il sottoinsieme italiano curato a mano (circa 55 codici in `src/obd/dtc.rs`);
  2. la traduzione automatica in `data/dtc_generici_it.tsv` (vedi sotto);
  3. il titolo inglese, se la traduzione mancasse.
- **Codici del costruttore** (ad esempio `P1xxx`): il loro significato varia secondo la marca
  e di norma non è nell'elenco; l'app lo dice esplicitamente.
- **Non trovato**: nessun dataset italiano o specifico Peugeot/Citroën con licenza aperta.

## Traduzione italiana

`data/dtc_generici_it.tsv` contiene la traduzione dei 9.533 titoli, con gli
stessi codici nello stesso ordine del file inglese (un test lo verifica).

- **Come è stata fatta**: da un modello linguistico (Claude Sonnet), titolo per
  titolo, con un glossario di officina fissato in anticipo (per esempio
  *O2 Sensor* → sonda lambda, *Solenoid* → elettrovalvola, *Bank* → bancata,
  *Control Module* → centralina). Non è una sostituzione parola per parola.
- **Cosa non è**: non è stata rivista da un tecnico né confrontata con la
  terminologia ufficiale di un costruttore. Alcuni termini senza un
  equivalente italiano consolidato sono rimasti in inglese (per esempio
  *control pilot* nei codici di ricarica).
- **Come lo dice l'app**: accanto a ogni descrizione tradotta in questo modo la
  pagina Diagnosi mostra il titolo inglese originale e la nota «tradotta
  automaticamente». In caso di dubbio vale l'originale.
- **Licenza**: la traduzione deriva da dati CC0 ed è distribuita alle stesse
  condizioni.
- **Correggere una voce**: modificare la riga nel file TSV; per una
  descrizione più curata, aggiungerla all'elenco in `src/obd/dtc.rs`, che ha la
  precedenza.

Se si rigenera il file inglese e cambiano i codici, la traduzione va
riallineata: il test `traduzione_allineata_all_originale` fallisce finché i due
file non coincidono codice per codice.

## Rigenerare il file

1. Scaricare `obdex_generic.min.json` dal repository OBDex e salvarlo in
   `ricerca/dtc/obdex_generic.min.json` (la cartella `ricerca/` è ignorata da git: il file
   sorgente pesa circa 31 MB e non va committato).
2. Dalla radice del progetto: `python strumenti/genera_dtc.py`
   (accetta facoltativamente sorgente e destinazione come argomenti).
3. Lo script usa solo la libreria standard, ordina per codice, salta i record senza titolo
   inglese e termina con errore se trova codici duplicati.
