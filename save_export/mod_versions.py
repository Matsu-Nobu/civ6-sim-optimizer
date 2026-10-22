"""Lists the mods (with their versions in the titles) a game was played with, from the first of its first five
autosaves that lists them: the game's rules database must be made with the same mods to recompute the game data.

  mod_versions.py <save dir> > mods.txt
"""
import re
import sys
from pathlib import Path

# the runs of at least 6 printable characters, as `strings -n 6` finds them
RUNS = re.compile(rb"[\x20-\x7e\t]{6,}")
LISTED = re.compile(r'^\{"([^"]+ [0-9][0-9.]*)":\[\]\}')
TITLE = re.compile(r'"en_US", "text":"([^"]+ [0-9][0-9.]*)"')


def mods(save):
    found = set()
    for run in RUNS.findall(save.read_bytes()):
        text = run.decode("ascii")
        found.update(m.group(1) for m in [LISTED.match(text)] if m)
        found.update(m.group(1) for m in TITLE.finditer(text))
    return sorted(found)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    for save in sorted(Path(sys.argv[1]).glob("*.Civ6Save"))[:5]:
        found = mods(save)
        if found:
            print("\n".join(found))
            return
    sys.exit(f"no save in {sys.argv[1]} lists its mods")


if __name__ == "__main__":
    main()
