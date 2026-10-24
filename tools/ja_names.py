"""Writes data/rules/names_ja.tsv: each type of the rules database (TYPE<TAB>name), with the name the game's Japanese
version shows, and the city names (CITY_NAME_X<TAB>name, X as the saves name the city). Look names up with grep, e.g. `grep -P '^DISTRICT_ENCAMPMENT\t' data/rules/names_ja.tsv` or
`grep -P '\t.*兵営' data/rules/names_ja.tsv`.

  ja_names.py <rules.sqlite>

The names come from the Japanese text of the game, its DLCs (except the scenarios) and the workshop mods; the later
files (DLCs, then the mods) override the earlier ones, as in the game. Types without a name are left out.
"""
import sqlite3
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "data/rules/names_ja.tsv"
GAME = Path("/mnt/c/Program Files (x86)/Steam/steamapps/common/Sid Meier's Civilization VI")
WORKSHOP = Path("/mnt/c/Program Files (x86)/Steam/steamapps/workshop/content/289070")


def japanese_rows(path):
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError:
        return
    for el in root.iter():
        if el.tag in ("Row", "Replace") and (el.get("Language") or "").lower() == "ja_jp":
            text = el.get("Text")
            if text is None and el.find("Text") is not None:
                text = el.find("Text").text
            if el.get("Tag") and text:
                yield el.get("Tag"), text


def main(rules):
    text = {}
    # the scenarios' text is loaded only in those scenarios
    files = sorted(p for p in GAME.rglob("*.xml") if "/Text/" in str(p) and "Scenario" not in str(p))
    for f in files + sorted(WORKSHOP.glob("*/lang/*.xml")):
        text.update(japanese_rows(f))
    types = [t for (t,) in sqlite3.connect(f"file:{rules}?mode=ro", uri=True).execute("select Type from Types order by Type")]
    with open(OUT, "w") as out:
        for t in types:
            name = text.get(f"LOC_{t}_NAME")
            if name:
                out.write(f"{t}\t{name}\n")
        for tag, name in sorted(text.items()):
            if tag.startswith("LOC_CITY_NAME_"):
                out.write(f"{tag[4:]}\t{name}\n")


if __name__ == "__main__":
    main(*sys.argv[1:])
