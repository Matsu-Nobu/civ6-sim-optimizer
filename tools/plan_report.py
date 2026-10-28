"""Writes a plan's game as a turn-by-turn list in Markdown, with the game's Japanese names: what each city produces,
purchases, builders' actions, settlers' sites, research, civics, policies, governors, and the turn's yields.

  plan_report.py <game data .json> <plan .json> <out.md> [civ6opt options, e.g. --turn 65 --team --ally-supply ...]

Builders waiting, nothing bought, a balanced focus and empty policy slots are left out.
"""
import json
import sys
from pathlib import Path

import civ6opt

NAMES = {}
for line in (civ6opt.ROOT / "data" / "rules" / "names_ja.tsv").read_text().splitlines():
    k, v = line.split("\t")
    NAMES[k] = v

KINDS = {"prod": "生産", "buy": "購入", "builder": "労働者", "settle": "開拓者", "research": "研究", "civic": "社会制度",
         "policy": "政策", "gov": "政体", "governor": "総督", "focus": "市民の重点", "trade": "交易路", "gp": "偉人",
         "pantheon": "パンテオン", "dedication": "献身"}
FOCUS = {"balanced": "均等", "food": "食料", "production": "生産", "gold": "ゴールド", "science": "科学", "culture": "文化",
         "faith": "信仰"}
ACTS = {"improve": "改善", "harvest": "伐採", "plant": "植林", "rush": "建設を早める"}


def ja(x):
    if not isinstance(x, str):
        return x
    return NAMES.get(x) or NAMES.get(f"CITY_NAME_{x}") or x


def choice_text(kind, c):
    if kind == "focus":
        return FOCUS.get(c, c)
    if isinstance(c, str):
        return "なし" if c == "NONE" else ja(c)
    if kind == "settle" or (isinstance(c, list) and len(c) == 2 and all(isinstance(v, int) for v in c)):
        return f"({c[0]},{c[1]})"
    head = c[0]
    if head == "item":
        return f"{ja(c[1])}に{ja(c[2])}（{c[3]} ゴールド）"
    if head == "tile":
        return f"{ja(c[1])}のタイル ({c[3][0]},{c[3][1]})（{c[2]} ゴールド）"
    if head in ACTS:
        what = "資源" if c[3] == "resource" else "地形" if c[3] == "feature" else ja(c[3])
        return f"{ja(c[1])}の ({c[2][0]},{c[2][1]}) で{ACTS[head]}：{what}"
    if head == "appoint":
        return f"{ja(c[1])}を{ja(c[2])}に任命"
    if head == "promote":
        return f"{ja(c[1])}に{ja(c[2])}"
    if head == "route":
        return f"{ja(c[1])}から{ja(c[2])}へ"
    return json.dumps(c, ensure_ascii=False)


def decisions_by_turn(text):
    """The `civ6opt decisions` output as {turn: [line, ...]} in Japanese."""
    by_turn = {}
    for line in text.splitlines():
        try:
            d = json.loads(line)
        except json.JSONDecodeError:
            continue
        # the defaults a plan keeps (a builder waiting, nothing bought, a balanced citizen focus) are left out
        if d["choice"] in (["wait"], ["none"], "balanced") or (d["key"][0] == "policy" and d["choice"] == "NONE"):
            continue
        # a decision with one option is still what the game did (the first scout, a forced civic), except the routine
        # ones
        if d.get("options", 2) < 2 and d["key"][0] not in ("prod", "research", "civic", "settle", "gov"):
            continue
        k = d["key"]
        kind = k[0]
        who = ja(k[1]) if kind in ("prod", "focus") else ""
        by_turn.setdefault(d["turn"], []).append(f"{KINDS.get(kind, kind)}{('（' + who + '）') if who else ''}：{choice_text(kind, d['choice'])}")
    return by_turn


def snapshots(text):
    """The `civ6opt run` output as ({turn: state}, final line)."""
    snaps, final = {}, None
    for line in text.splitlines():
        try:
            d = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "turn" in d:
            snaps[d["turn"]] = d
        elif "score" in d:
            final = d
    return snaps, final


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    game, plan, out, opts = sys.argv[1], sys.argv[2], Path(sys.argv[3]), sys.argv[4:]
    decisions = civ6opt.run(["decisions", game, *opts, "--genome", plan], capture_output=True, text=True, check=True).stdout
    states = civ6opt.run(["run", game, *opts, "--genome", plan], capture_output=True, text=True, check=True).stdout
    by_turn = decisions_by_turn(decisions)
    snaps, final = snapshots(states)
    lines = [f"# {Path(plan).name}", "", f"点数 {final['score']:.0f}、食料と生産の累計 {final['total']:.0f}。", ""]
    for t in sorted(by_turn):
        s = snaps.get(t)
        head = f"## T{t}"
        if s:
            o = s["output"]
            head += f"（都市 {len(s['cities'])}、人口 {sum(c['pop'] for c in s['cities'])}、食料 {o[0]:.0f}、生産 {o[1]:.0f}、科学 {o[3]:.0f}、文化 {o[4]:.0f}、ゴールド {s['gold']:.0f}）"
        lines += [head, ""] + [f"- {x}" for x in by_turn.get(t, [])] + [""]
    out.write_text("\n".join(lines))
    print("wrote", out)


if __name__ == "__main__":
    main()
