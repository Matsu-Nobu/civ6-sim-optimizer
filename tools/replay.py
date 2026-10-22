"""How exactly the simulator reproduces the recorded games.

  replay.py check <turn> <game data name>... [--genome record]
      Replays the recorded play of each game (civ6opt run --genome record, data/games/<name>.json) to the turn given
      and compares it with the record turn by turn (tools/replay_check.py).
  replay.py summary [<game data name>...]
      For each game: the turns of the turn-by-turn check (civ6opt validate) without model errors, and the full replay
      at T30, T40 and T50 (food and production: replay / record).
"""
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import civ6opt

GAMES = civ6opt.ROOT / "data" / "games"
SUMMARY_GAMES = ["france_20261014", "france_20261018", "persia_20261017", "gran_colombia_20261019", "rome_20261007b"]


def check(turn, game, genome="record"):
    """The comparison of tools/replay_check.py for one game, as text; the replay's exogenous inputs first."""
    with tempfile.TemporaryDirectory() as tmp:
        out, err = Path(tmp) / "replay.out", Path(tmp) / "replay.err"
        with open(out, "w") as o, open(err, "w") as e:
            civ6opt.run(["run", GAMES / f"{game}.json", "--turn", turn + 1, "--ext", 0, "--allow-military", "--team", "--genome", genome],
                        stdout=o, stderr=e, check=True)
        exogenous = [line for line in err.read_text().splitlines() if line.startswith("exogenous")]
        compared = subprocess.run([sys.executable, Path(__file__).with_name("replay_check.py"), GAMES / f"{game}.json", out],
                                  capture_output=True, text=True, check=True).stdout
    return "\n".join(exogenous + [compared.rstrip("\n")])


def summary(games):
    for g in games:
        validate = civ6opt.run(["validate", GAMES / f"{g}.json", "--team", "--turn", 50], capture_output=True, text=True)
        # validate prints its total (errors <turns with model errors>/<turns>) last on stdout
        errors = re.search(r"errors [0-9/]*", validate.stdout.strip().splitlines()[-1])
        assert errors, f"{g}: civ6opt validate printed no total: {validate.stdout[-300:]!r}"
        turns = [" ".join(line.split()[:3]) for line in check(50, g).splitlines() if re.match(r"T(30|40|50) ", line)]
        print(f"{g}: {errors.group(0)} | {' | '.join(turns)}")


def main():
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    genome = "record"
    if "--genome" in args:
        i = args.index("--genome")
        genome = args[i + 1]
        del args[i:i + 2]
    if args[0] == "check":
        for g in args[2:]:
            print(f"=== {g}")
            print(check(int(args[1]), g, genome))
    elif args[0] == "summary":
        summary(args[1:] or SUMMARY_GAMES)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
