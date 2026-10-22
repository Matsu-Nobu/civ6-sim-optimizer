"""Method comparison: each search method from each start with each seed, the same budget (MINUTES per run, THREADS
threads). Runs one after another; a run whose result file exists is skipped, so the script can be stopped and started
again. Results: data/search/cmp/<method>_<start>_s<seed>.{json,log}

  compare_methods.py      environment: P (game data, data/games/rome_20261007b.json), MINUTES (30), THREADS (10)
"""
import os
import time

import civ6opt

P = os.environ.get("P", str(civ6opt.ROOT / "data" / "games" / "rome_20261007b.json"))
MINUTES = os.environ.get("MINUTES", "30")
THREADS = os.environ.get("THREADS", "10")
OUT = civ6opt.ROOT / "data" / "search" / "cmp"


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for seed in (1, 2, 3):
        for start in ("record_wide", "base"):
            for method in ("lns", "ils", "ma", "nrpa"):
                f = OUT / f"{method}_{start}_s{seed}"
                if f.with_suffix(".json").exists():
                    continue
                common = ["--turn", "60", "--ext", "10", "--seed", seed, "--threads", THREADS]
                args = {
                    "lns": ["lns", P, *common, "--init", start, "--window", "5", "--budget", "70", "--passes", "4"],
                    "ils": ["ils", P, *common, "--init", start, "--budget", "70", "--minutes", MINUTES],
                    "ma": ["ma", P, *common, "--init", start, "--budget", "70", "--minutes", MINUTES],
                    "nrpa": ["search", P, *common, "--init", start, "--runs", THREADS, "--budget", "7200", "--rounds", "3"],
                }[method]
                print(f"{time.strftime('%H:%M')} {method} {start} seed {seed}", flush=True)
                tmp = f.with_suffix(".json.tmp")
                # a run that dies (rare heap aborts, cause not found) is run once more
                for attempt in (1, 2):
                    with open(f.with_suffix(".log"), "w") as log:
                        if civ6opt.run([*args, "--out", tmp], stdout=log, stderr=log).returncode == 0:
                            tmp.replace(f.with_suffix(".json"))
                            break
                    print(f"{time.strftime('%H:%M')} {method} {start} seed {seed} died (attempt {attempt})", flush=True)


if __name__ == "__main__":
    main()
