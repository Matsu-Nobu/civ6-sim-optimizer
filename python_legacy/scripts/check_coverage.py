"""Coverage check for the agreed scope (turns 1-100, economy, all civs). Exits non-zero while anything in scope
is unimplemented. Usage: python3 python_legacy/scripts/check_coverage.py <rules.sqlite> [--list todo|out|implemented]"""
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.coverage import effect_table, source_tables, yield_tables  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402


def main():
    rules = Rules(sys.argv[1])
    want = sys.argv[sys.argv.index("--list") + 1] if "--list" in sys.argv else None
    eff = effect_table(rules)
    src = source_tables(rules)
    tab = yield_tables(rules)
    status = lambda s: s.split(":")[0]
    for name, rows in (("effect types", [(e, s) for e, _, s in eff]), ("source tables", src), ("yield/cost tables", tab)):
        c = Counter(status(s) for _, s in rows)
        print(f"{name:18s} implemented {c['implemented']:4d}  out of scope {c['out']:4d}  todo {c['todo']:4d}")
    if want:
        for e, n, s in eff:
            if status(s) == want:
                print(f"  effect {e} ({n} modifiers) {s}")
        for t, s in src + tab:
            if status(s) == want:
                print(f"  table {t} {s}")
    todo = sum(1 for _, _, s in eff if s == "todo") + sum(1 for _, s in src + tab if s == "todo")
    sys.exit(1 if todo else 0)


if __name__ == "__main__":
    main()
