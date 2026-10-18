"""Collect English game text (base game, DLCs, BBG workshop mod) into data/rules/text_en.sqlite (Tag -> Text), so
effect semantics can be read from the in-game descriptions. Later sources (BBG) override earlier ones."""
import re
import sqlite3
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

GAME = Path("/mnt/c/Program Files (x86)/Steam/steamapps/common/Sid Meier's Civilization VI")
WORKSHOP = Path("/mnt/c/Program Files (x86)/Steam/steamapps/workshop/content/289070")


def rows(path):
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError:
        return
    for el in root.iter():
        if el.tag in ("Row", "Replace"):
            tag = el.get("Tag")
            lang = el.get("Language", "en_US")
            text = el.get("Text")
            if text is None:
                t = el.find("Text")
                text = t.text if t is not None else None
            if tag and text and lang == "en_US":
                yield tag, text


def main(out="data/rules/text_en.sqlite"):
    files = sorted(p for p in GAME.rglob("*.xml") if "/Text/" in str(p) and "Translations" not in p.name
                   and re.search(r"/en_US/|Text\.xml$|_Text", str(p)))
    files += sorted(WORKSHOP.glob("*/lang/english.xml"))
    db = sqlite3.connect(out)
    db.execute("drop table if exists Text")
    db.execute("create table Text (Tag text primary key, Text text, Source text)")
    n = 0
    for f in files:
        for tag, text in rows(f):
            db.execute("insert or replace into Text values (?,?,?)", (tag, text, str(f)))
            n += 1
    db.commit()
    print(f"{len(files)} files, {n} rows", file=sys.stderr)


if __name__ == "__main__":
    main(*sys.argv[1:])
