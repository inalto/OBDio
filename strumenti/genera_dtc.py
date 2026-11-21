#!/usr/bin/env python3
"""Genera data/dtc_generici.tsv dall'archivio OBDex (CC0-1.0).

Uso (dalla radice del progetto):
    python strumenti/genera_dtc.py [sorgente.json] [destinazione.tsv]

Predefiniti: ricerca/dtc/obdex_generic.min.json -> data/dtc_generici.tsv
"""
import json
import re
import sys
from pathlib import Path

RADICE = Path(__file__).resolve().parent.parent
SORGENTE = RADICE / "ricerca" / "dtc" / "obdex_generic.min.json"
DESTINAZIONE = RADICE / "data" / "dtc_generici.tsv"
FORMA_CODICE = re.compile(r"^[PCBU][0-3][0-9A-F]{3}$")


def main() -> None:
    sorgente = Path(sys.argv[1]) if len(sys.argv) > 1 else SORGENTE
    destinazione = Path(sys.argv[2]) if len(sys.argv) > 2 else DESTINAZIONE
    with open(sorgente, encoding="utf-8") as f:
        record = json.load(f)
    if isinstance(record, dict):
        record = list(record.values())

    righe = {}
    for r in record:
        codice = str(r.get("code", "")).strip().upper()
        titolo = ((r.get("title") or {}).get("en") or "")
        titolo = " ".join(titolo.replace("\t", " ").split())
        if not titolo:
            continue
        if not FORMA_CODICE.match(codice):
            sys.exit(f"errore: codice non valido: {codice!r}")
        if codice in righe:
            sys.exit(f"errore: codice duplicato: {codice}")
        righe[codice] = titolo

    destinazione.parent.mkdir(parents=True, exist_ok=True)
    with open(destinazione, "w", encoding="utf-8", newline="\n") as f:
        for codice in sorted(righe):
            f.write(f"{codice}\t{righe[codice]}\n")
    print(f"{len(righe)} righe scritte in {destinazione}")


if __name__ == "__main__":
    main()
