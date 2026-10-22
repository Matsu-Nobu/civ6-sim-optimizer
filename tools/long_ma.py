"""A long memetic search that survives crashes: runs civ6opt ma until it stops on its own (stalled, or MINUTES used
up); a run that dies is started again from the best plan saved so far (the file is rewritten every generation).

  long_ma.py <out.json> [init]   environment: MINUTES (total, 360), STALL (60), THREADS (10), SEED (1), POP (10),
                                 P (game data, data/games/rome_20261007b.json)
"""
import os
import sys
import time
from pathlib import Path

import civ6opt


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    out = Path(sys.argv[1])
    first = sys.argv[2] if len(sys.argv) == 3 else "record_wide"
    env = os.environ
    p = env.get("P", str(civ6opt.ROOT / "data" / "games" / "rome_20261007b.json"))
    minutes, seed = int(env.get("MINUTES", "360")), int(env.get("SEED", "1"))
    t0, attempt = time.time(), 0
    while True:
        left = minutes - int((time.time() - t0) / 60)
        if left <= 1:
            print("time used up")
            break
        init = str(out) if out.exists() else first
        attempt += 1
        print(f"{time.strftime('%H:%M')} attempt {attempt} from {init}, {left} minutes left", flush=True)
        log = out.with_name(f"{out.stem}_{attempt}.log")
        with open(log, "w") as f:
            rc = civ6opt.run(["ma", p, "--turn", "60", "--ext", "10", "--seed", seed + attempt - 1, "--threads", env.get("THREADS", "10"),
                              "--pop", env.get("POP", "10"), "--init", init, "--budget", "70", "--minutes", left,
                              "--stall", env.get("STALL", "60"), "--out", out], stdout=f, stderr=f).returncode
        print(log.read_text().splitlines()[-1])
        if rc == 0:
            break
        print(f"attempt {attempt} died (exit {rc}); restarting from the saved best")


if __name__ == "__main__":
    main()
