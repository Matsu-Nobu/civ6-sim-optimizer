"""Writes a plan of the simulator as the plan file of the game mod (game_mods/Civ6EconAutoplay/Civ6EconPlan.lua), for
the mod to play it in the game, and the simulator's turns of that plan for tools/plan_diff.py.

  plan_lua.py <game data .json> <plan.json> <turns out .jsonl> [civ6opt options]

Game turn t plays the simulator's turn t. A save is written at the start of a turn, before the game has processed the
last turn's growth and production, so the simulator's state after its turn t is the game's save of turn t + 2 (11/01
runs from EconPlanStart_mp1_T002: equal at turns 2 and 4). In that turn the plan has the research, civic,
government and policies in effect in that turn, each city's item (with a district's plot) and worked plots, the
builders' orders, the settlers' moves, and the purchases, trade routes, governors, pantheon and dedication chosen in
that turn. A builder acts on the turn its charge is used in the simulator; a settler moves from the turn after the
one it is made in and founds its city on the turn it arrives.
"""
import json
import sys
from pathlib import Path

import civ6opt

PLAN_FILE = civ6opt.ROOT / "game_mods" / "Civ6EconAutoplay" / "Civ6EconPlan.lua"
OFFSET = 0


def lua(v):
    if v is None:
        return "nil"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return repr(v)
    if isinstance(v, str):
        return json.dumps(v)
    if isinstance(v, dict):
        return "{" + ", ".join(f"[{lua(k)}] = {lua(x)}" for k, x in v.items()) + "}"
    return "{" + ", ".join(lua(x) for x in v) + "}"


def lines(args):
    out = civ6opt.run(args, capture_output=True, text=True, check=True).stdout
    return [json.loads(x) for x in out.splitlines() if x.startswith("{")]


def builder_at(snap, bid):
    return next((b for b in snap.get("builder_at", []) if b[3] == bid), None) if snap else None


def charge_turn(snaps, bid, t, last):
    """The turn from t on in which the builder uses a charge: its charges drop below those it had at the start of turn
    t, or it is gone with its last charge. A builder made in turn t acts in that turn."""
    before = builder_at(snaps.get(t - 1), bid)
    if before is None:
        return t
    for u in range(t, last + 1):
        b = builder_at(snaps.get(u), bid)
        if b is None or b[2] < before[2]:
            return u
    return t


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    game, plan, turns_out = sys.argv[1], sys.argv[2], Path(sys.argv[3])
    opts = sys.argv[4:] or ["--turn", "65", "--ext", "0"]
    snaps = {s["turn"]: s for s in lines(["run", game, *opts, "--genome", plan]) if "cities" in s}
    decisions = [d for d in lines(["decisions", game, *opts, "--genome", plan]) if "key" in d]
    turns_out.write_text("".join(json.dumps(snaps[t]) + "\n" for t in sorted(snaps)))
    g = json.load(open(game))
    first, last = min(snaps), max(snaps)
    centres = {c["name"]: tuple(c["xy"]) for s in snaps.values() for c in s["cities"]}
    for c in g["rows"][-1].get("worldCities", []):
        centres.setdefault(c["name"], (c["x"], c["y"]))

    turns = {t: {"cities": [], "builders": [], "settlers": [], "buys": [], "governors": [], "routes": [],
                 "unsupported": []} for t in range(first, last + 1)}
    chosen = {"research": None, "civic": None}
    prod = {}
    seen_builders = set()
    by_turn = {}
    for d in decisions:
        by_turn.setdefault(d["turn"], []).append(d)
    for t in range(first, last + 1):
        out = turns[t]
        for d in by_turn.get(t, []):
            kind, ch = d["key"][0], d["choice"]
            if kind in chosen:
                chosen[kind] = ch
            elif kind == "prod":
                prod[d["key"][1]] = ch
            elif kind == "builder" and ch[0] != "wait":
                bid = d["key"][1]
                if bid not in seen_builders:
                    seen_builders.add(bid)
                    # where it stands at the start of the turn; a builder made this turn: the plot in its id (turn,
                    # x, y, number)
                    b = builder_at(snaps.get(t - 1), bid)
                    xy = (b[0], b[1]) if b else (bid // 10000 % 1000, bid // 10 % 1000)
                    out["builders"].append({"op": "new", "id": bid, "x": xy[0], "y": xy[1]})
                out["builders"].append({"op": ch[0], "id": bid, "x": ch[2][0], "y": ch[2][1],
                                        "what": ch[3] if isinstance(ch[3], str) else None,
                                        "at": charge_turn(snaps, bid, t, last)})
            elif kind == "settle":
                x, y = ch
                s = next((s for s in snaps[t]["settlers"] if (s[1], s[2]) == (x, y)), None)
                if t + 1 in turns:
                    turns[t + 1]["settlers"].append({"op": "go", "id": f"{d['key'][1]}{d['key'][2]}", "x": x, "y": y,
                                                     "arrive": s[3] if s else t + 1})
            elif kind == "buy" and ch[0] == "item":
                cx, cy = centres[ch[1]]
                out["buys"].append({"kind": "item", "x": cx, "y": cy, "item": ch[2], "yield": "YIELD_GOLD"})
            elif kind == "buy" and ch[0] == "tile":
                cx, cy = centres[ch[1]]
                out["buys"].append({"kind": "tile", "cx": cx, "cy": cy, "x": ch[3][0], "y": ch[3][1]})
            elif kind == "trade" and ch[0] == "route":
                (ox, oy), (dx, dy) = centres[ch[1]], centres[ch[2]]
                out["routes"].append({"ox": ox, "oy": oy, "dx": dx, "dy": dy})
            elif kind == "governor" and ch[0] == "appoint":
                x, y = centres[ch[2]]
                out["governors"].append({"op": "appoint", "governor": ch[1], "x": x, "y": y})
            elif kind == "governor" and ch[0] == "promote":
                out["governors"].append({"op": "promote", "governor": ch[1], "promotion": ch[2]})
            elif kind == "pantheon":
                out["pantheon"] = ch
            elif kind == "dedication":
                out["dedication"] = ch
            elif kind in ("buy", "governor", "gp") and ch in (["none"], "wait"):
                pass
            elif kind not in ("focus", "policy", "gov", "builder"):
                out["unsupported"].append({"kind": kind, "choice": json.dumps(ch)})
        s = snaps[t]
        out.update(research=chosen["research"], civic=chosen["civic"], government=s["government"],
                   policies=[p for p in s["policies"] if p and p != "NONE"])
        for c in s["cities"]:
            item = prod.get(c["name"])
            site = next((d.split("@")[1] for d in c["districts"] if item and d.split("@")[0].rstrip("*") == item), None)
            sx, sy = map(int, site.split(",")) if site else (None, None)
            x, y = c["xy"]
            out["cities"].append({"x": x, "y": y, "item": item, "sx": sx, "sy": sy,
                                  "worked": [[w[0], w[1], 1] for w in c["worked"] if [w[0], w[1]] != [x, y]]})
    for d in turns.values():
        for b in d["builders"]:
            if "at" in b:
                b["at"] += OFFSET
        for st in d["settlers"]:
            st["arrive"] += OFFSET
    game_turns = {t + OFFSET: d for t, d in turns.items()}
    text = (f"-- Written by tools/plan_lua.py from {Path(game).name} and {Path(plan).name}. Do not edit.\n"
            f"ECONPLAN = {{ START = {first + OFFSET}, LAST = {last + OFFSET}, OFFSET = {OFFSET}, TURNS = {lua(game_turns)} }}\n")
    PLAN_FILE.write_text(text)
    n_unsup = sum(len(d["unsupported"]) for d in turns.values())
    print(f"turns {first}..{last}; {len(seen_builders)} builders; "
          f"{sum(len(d['settlers']) for d in turns.values())} settlers; {n_unsup} decisions the mod does not play")


if __name__ == "__main__":
    main()
