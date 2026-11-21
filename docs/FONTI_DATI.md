# Fonti dei dati

## Descrizioni dei codici guasto (DTC)

L'app incorpora l'elenco dei codici guasto OBD-II generici in `data/dtc_generici.tsv`
(una riga per codice: `CODICE<TAB>titolo inglese`, UTF-8, fine riga LF). Il file è
incluso nell'eseguibile con `include_str!` in `src/obd/dtc.rs`.

- **Origine**: progetto OBDex, <https://github.com/foerbsnavi/OBDex> (elenco «generic»).
- **Licenza dei dati**: CC0-1.0 (pubblico dominio), nessun obbligo di attribuzione.
- **Contenuto**: 9.533 codici (P, B, C, U). Si usa solo il titolo inglese; descrizioni
  estese, cause e riparazioni dell'archivio originale non sono incluse.
- **Lingua**: i titoli sono in inglese. Fa eccezione il sottoinsieme italiano curato a mano
  (circa 55 codici in `src/obd/dtc.rs`), che ha la precedenza; i testi inglesi sono
  segnalati come «(descrizione in inglese)».
- **Codici del costruttore** (ad esempio `P1xxx`): il loro significato varia secondo la marca
  e di norma non è nell'elenco; l'app lo dice esplicitamente.
- **Non trovato**: nessun dataset italiano o specifico Peugeot/Citroën con licenza aperta.

## Rigenerare il file

1. Scaricare `obdex_generic.min.json` dal repository OBDex e salvarlo in
   `ricerca/dtc/obdex_generic.min.json` (la cartella `ricerca/` è ignorata da git: il file
   sorgente pesa circa 31 MB e non va committato).
2. Dalla radice del progetto: `python strumenti/genera_dtc.py`
   (accetta facoltativamente sorgente e destinazione come argomenti).
3. Lo script usa solo la libreria standard, ordina per codice, salta i record senza titolo
   inglese e termina con errore se trova codici duplicati.
