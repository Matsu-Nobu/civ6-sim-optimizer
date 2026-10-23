"""Makes the game data files from the compressed records and maps in data/exports and, for each game, the rules
database data/rules/<game>.sqlite: the game's Cache/DebugGameplay.sqlite, written by a game set up like the recorded
one (the mods in data/exports/game_<date>_mods.txt, the same victory types). Also needs data/rules/text_en.sqlite
(civ6opt text) and the built civ6opt (cargo build --release in optimizer/).

  build_games.py [out dir, default data/games] [game data name...]

With names, only those games are made.
"""
import sys
from compression import zstd
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import civ6opt  # noqa: E402

DATA = civ6opt.ROOT / "data"

# record date, game data name, player, last turn of the plans, builder candidates per city, other options
GAMES = [
    ("20261005", "rome_20261005", 0, 60, 8, []),
    ("20261007b", "rome_20261007b", 0, 85, 8, []),
    ("20261013", "france_20261013", 8, 60, 8, []),
    ("20261014", "france_20261014", 5, 68, 8, []),
    # 10/17: the slinger (T15) and the trader (T16) were bought with gold an ally gave the same turn, so the record's
    # gold does not show the purchases; they are given as gifts.
    ("20261017", "persia_20261017", 2, 45, 4, ["--unit-gift", "14:UNIT_SLINGER,15:UNIT_TRADER"]),
    ("20261018", "france_20261018", 1, 45, 8, []),
    ("20261019", "gran_colombia_20261019", 0, 55, 8, []),
    ("20261021", "rome_20261021", 2, 65, 8, []),
    ("20261024", "france_20261024", 12, 45, 8, []),
]


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else DATA / "games"
    only = set(sys.argv[2:])
    out.mkdir(parents=True, exist_ok=True)
    for f in (DATA / "exports").glob("*.zst"):
        plain = f.with_suffix("")
        if not plain.exists():
            plain.write_bytes(zstd.decompress(f.read_bytes()))
    for date, name, player, horizon, builders, extra in GAMES:
        if only and name not in only:
            continue
        civ6opt.run(["build", DATA / "exports" / f"game_{date}", DATA / "rules" / f"{name}.sqlite", out / f"{name}.json",
                     "--player", player, "--horizon", horizon, "--builder-candidates", builders,
                     "--text", DATA / "rules" / "text_en.sqlite", *extra], check=True)


if __name__ == "__main__":
    main()
