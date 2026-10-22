"""Trains the option model of optimizer/src/learn.rs on the rows `civ6opt collect` writes, reports how well it picks
options on plans it did not see, and writes the trees for the Rust side.

usage: python tools/learn_train.py rows.tsv [rows2.tsv ...] --out model.json [--test search_rows.tsv] [--threads 4]

--test: rows of a search that tried every option of its decisions (civ6opt irollout --rows without --model), not
trained on. For the decisions where some option beats the plan's choice, it reports how often the best one is the
model's first pick and among its first three, by kind of decision: the search tries the model's first options.
"""
import argparse
import json

import lightgbm as lgb
import numpy as np

MISSING = -999.0
# the score changes are clipped: a missed requirement costs tens of thousands, which would drown the differences
# between options that keep it
CLIP = 3000.0


def load(paths):
    cols, rows = None, []
    for k, path in enumerate(paths):
        with open(path) as f:
            head = f.readline().rstrip("\n").split("\t")
            assert cols is None or head == cols, f"{path}: other columns"
            cols = head
            for line in f:
                r = line.rstrip("\n").split("\t")
                r[0] = f"{k}:{r[0]}"
                rows.append(r)
    return cols, rows


def test_picks(m, path, cols, matrix):
    tcols, rows = load([path])
    assert tcols == cols, f"{path}: other columns"
    pred = m.predict(matrix(rows))
    by_decision = {}
    for r, p in zip(rows, pred):
        by_decision.setdefault(r[1], []).append((r, p))
    stats = {}
    for options in by_decision.values():
        alts = [(float(r[-1]), p) for r, p in options if r[-2] == "0"]
        if len(alts) < 2 or max(a[0] for a in alts) <= 0:
            continue
        best = max(range(len(alts)), key=lambda i: alts[i][0])
        order = sorted(range(len(alts)), key=lambda i: -alts[i][1])
        s = stats.setdefault(options[0][0][2], [0, 0, 0, 0.0])
        s[0] += 1
        s[1] += order[0] == best
        s[2] += best in order[:3]
        s[3] += min(3, len(alts)) / len(alts)
    print(f"{path}: decisions with a better option | best is the model's first | among its first three | three at random")
    total = [sum(s[i] for s in stats.values()) for i in range(4)]
    for k, s in sorted(stats.items()) + [("all", total)]:
        print(f"  {k:9} {s[0]:5} | {s[1] / s[0]:.2f} | {s[2] / s[0]:.2f} | {s[3] / s[0]:.2f}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("rows", nargs="+")
    ap.add_argument("--out", required=True)
    ap.add_argument("--rounds", type=int, default=400)
    ap.add_argument("--test")
    ap.add_argument("--threads", type=int, default=4)
    args = ap.parse_args()
    cols, rows = load(args.rows)
    numeric = cols[4:-2]
    kinds = sorted({r[2] for r in rows})
    names = sorted({r[3] for r in rows})
    ki, ni = {k: i for i, k in enumerate(kinds)}, {k: i for i, k in enumerate(names)}
    def matrix(rows):
        # a name the training did not see gets a code of no category, as on the Rust side
        x = np.array([[ki.get(r[2], len(ki)), ni.get(r[3], len(ni))] + [float(v) for v in r[4:-2]] for r in rows])
        x[np.isnan(x)] = MISSING
        return x

    x = matrix(rows)
    y = np.clip(np.array([float(r[-1]) for r in rows]), -CLIP, CLIP)
    chosen = np.array([r[-2] == "1" for r in rows])
    group = np.array([f"{r[0]}:{r[1]}" for r in rows])
    plan = np.array([r[0] for r in rows])
    params = dict(objective="huber", alpha=200.0, learning_rate=0.05, num_leaves=63, min_data_in_leaf=20,
                  feature_fraction=0.9, bagging_fraction=0.8, bagging_freq=1, cat_smooth=10, num_threads=args.threads, verbose=-1)
    feats = ["kind", "name"] + numeric

    def fit(mask):
        d = lgb.Dataset(x[mask], y[mask], feature_name=feats, categorical_feature=[0, 1], free_raw_data=False)
        return lgb.train(params, d, num_boost_round=args.rounds)

    # each plan held out in turn: the model trained on the others picks among the sampled options of its decisions
    print(f"{len(rows)} rows, {len(set(group))} decisions, {len(set(plan))} plans")
    print("held out | decisions | kept is best | model's pick is best | pairs ordered right: model, base policy | mean gain: model | best | base policy")
    for hp in sorted(set(plan)) if len(set(plan)) > 1 else []:
        m = fit(plan != hp)
        test = plan == hp
        pred = m.predict(x[test])
        g, yt, ch, base = group[test], y[test], chosen[test], x[test][:, feats.index("option_is_base")]
        n = kept_best = model_best = pairs = pairs_model = pairs_base = 0
        gain_model = gain_best = gain_base = 0.0
        for d in np.unique(g):
            i = np.where(g == d)[0]
            n += 1
            best = yt[i].max()
            kept_best += yt[i][ch[i]][0] >= best
            j = i[np.argmax(pred[i])]
            model_best += yt[j] >= best
            gain_model += yt[j]
            gain_best += best
            # pairs of options whose scores differ by more than the noise of the huber loss's alpha
            for a in i:
                for c in i:
                    if yt[a] > yt[c] + 50:
                        pairs += 1
                        pairs_model += pred[a] > pred[c]
                        pairs_base += base[a] > base[c]
            b = i[base[i] == 1]
            gain_base += yt[b[0]] if len(b) else 0.0
        print(f"{hp:8} | {n:9} | {kept_best / n:12.2f} | {model_best / n:20.2f} | {pairs_model / max(pairs, 1):18.2f}, {pairs_base / max(pairs, 1):.2f} | {gain_model / n:7.1f} | {gain_best / n:5.1f} | {gain_base / n:6.1f}")
    m = fit(np.ones(len(rows), bool))
    if args.test:
        test_picks(m, args.test, cols, matrix)
    imp = sorted(zip(m.feature_importance("gain"), feats), reverse=True)[:12]
    print("gain by feature:", ", ".join(f"{f} {v:.0f}" for v, f in imp))
    trees = []
    for t in m.dump_model()["tree_info"]:
        nodes, leaves = [], []

        def walk(n):
            if "leaf_value" in n:
                leaves.append(n["leaf_value"])
                return -len(leaves)
            k = len(nodes)
            nodes.append(None)
            if n["decision_type"] == "==":
                split = [int(c) for c in str(n["threshold"]).split("||")]
            else:
                assert n["decision_type"] == "<=", n["decision_type"]
                split = n["threshold"]
            nodes[k] = [n["split_feature"], split, walk(n["left_child"]), walk(n["right_child"])]
            return k

        root = t["tree_structure"]
        if "leaf_value" in root:
            trees.append({"nodes": [], "leaves": [root["leaf_value"]]})
            continue
        walk(root)
        # leaf indices: the Rust side reads a child c < 0 as leaves[!c] = leaves[-c - 1]
        trees.append({"nodes": nodes, "leaves": leaves})
    with open(args.out, "w") as f:
        # rows with LightGBM's predictions, which the Rust side reproduces when it loads the trees
        step = max(1, len(x) // 50)
        check = [[list(map(float, r)), float(p)] for r, p in zip(x[::step], m.predict(x[::step]))]
        json.dump({"numeric": numeric, "kinds": kinds, "names": names, "trees": trees, "check": check}, f)
    print("saved", args.out)


if __name__ == "__main__":
    main()
