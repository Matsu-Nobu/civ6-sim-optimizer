"""Our engine's applied modifiers vs the game's own active list (probe GameEffects dump, every 10 turns).
Usage: modifier_diff.py <rules> <export.jsonl> <mods.jsonl> [--all]   (default: in-scope effects only)"""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.coverage import classify, implemented_effects  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402

r = Rules(sys.argv[1])
ex = {json.loads(l)["turn"]: json.loads(l) for l in open(sys.argv[2])}
pid = next(iter(ex.values()))["player"]["playerId"]
impl = implemented_effects()
effect_of = dict(r.q("select m.ModifierId, d.EffectType from Modifiers m join DynamicModifiers d on d.ModifierType=m.ModifierType"))


def relevant(mid):
    e = effect_of.get(mid)
    if e is None:
        return False
    if "--all" in sys.argv or e == "EFFECT_ATTACH_MODIFIER":
        return e != "EFFECT_ATTACH_MODIFIER" or "--all" in sys.argv
    return not classify(e, impl).startswith("out")


missing, extra = Counter(), Counter()
turns = 0
for line in open(sys.argv[3]):
    d = json.loads(line)
    e = ex.get(d["turn"])
    if not e:
        continue
    turns += 1
    game = Counter()
    for mid, owner_pid, owner, n, names in d["modifiers"]:
        if owner_pid == pid:
            game[mid] += n
    eng = engine_for(r, player_of(e), e["cities"])
    ours = Counter(eng.applied)
    for mid in set(game) | set(ours):
        if not relevant(mid):
            continue
        g, o = game.get(mid, 0), ours.get(mid, 0)
        if g and not o:
            missing[mid] += 1
        elif o and not g:
            extra[mid] += 1
print(f"{turns} dumps compared (player {pid})")
print("game applies, we don't:")
for mid, n in missing.most_common(40):
    print(f"  {n:3d} {mid}  [{effect_of.get(mid, '?')[7:]}]")
print("we apply, game doesn't:")
for mid, n in extra.most_common(40):
    print(f"  {n:3d} {mid}  [{effect_of.get(mid, '?')[7:]}]")
