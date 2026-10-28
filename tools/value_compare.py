"""Compares models of a state's value (the final score of the play from that state) on the rows `civ6opt valuerows`
writes: a linear model per turn (ridge regression on standardized features, as optimizer/src/value.rs) and one
LightGBM model over all turns (the turn as a feature).

  value_compare.py <rows.tsv> [--test-every 5]

Every fifth play is held out. For each turn bucket it prints R2 (the share of the scores' variance the model explains)
and the pairwise accuracy: among held-out states of the same turn whose scores differ by more than 1%, the share of
pairs the model orders right (what a tree search needs: which of two states is better).
"""
import argparse
import sys

import numpy as np


def load(path):
    a = np.loadtxt(path, delimiter="\t")
    return a[:, 0].astype(int), a[:, 1].astype(int), a[:, 2:-1], a[:, -1]


def ridge_per_turn(turns, X, y, tr, alpha=0.01):
    models = {}
    for t in np.unique(turns):
        m = tr & (turns == t)
        if m.sum() < 5:
            continue
        mu, sd = X[m].mean(0), X[m].std(0) + 1e-9
        Z = (X[m] - mu) / sd
        Z = np.hstack([np.ones((len(Z), 1)), Z])
        w = np.linalg.solve(Z.T @ Z + alpha * len(Z) * np.eye(Z.shape[1]), Z.T @ y[m])
        models[t] = (mu, sd, w)

    def predict(t, x):
        mu, sd, w = models[t]
        return np.hstack([np.ones((len(x), 1)), (x - mu) / sd]) @ w
    return models, predict


def pair_accuracy(pred, y, rng, n=4000):
    if len(y) < 2:
        return float("nan")
    i, j = rng.integers(0, len(y), n), rng.integers(0, len(y), n)
    keep = np.abs(y[i] - y[j]) > 0.01 * np.abs(y).mean()
    if not keep.any():
        return float("nan")
    return float(np.mean(np.sign(pred[i] - pred[j])[keep] == np.sign(y[i] - y[j])[keep]))


def r2(pred, y):
    return float(1 - ((y - pred) ** 2).sum() / max(((y - y.mean()) ** 2).sum(), 1e-9))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("rows")
    ap.add_argument("--test-every", type=int, default=5)
    a = ap.parse_args()
    plays, turns, X, y = load(a.rows)
    te = plays % a.test_every == a.test_every - 1
    tr = ~te
    print(f"{len(np.unique(plays))} plays, {len(y)} rows, {X.shape[1]} features; held out {len(np.unique(plays[te]))} plays")
    _, lin = ridge_per_turn(turns, X, y, tr)
    try:
        import lightgbm as lgb
    except ImportError:
        sys.exit("lightgbm is needed (pip install -r requirements.txt)")
    F = np.hstack([turns[:, None], X])
    params = {"objective": "regression", "learning_rate": 0.05, "num_leaves": 63, "min_data_in_leaf": 20, "verbose": -1}
    gbm = lgb.train(params, lgb.Dataset(F[tr], y[tr]), num_boost_round=600)
    pg = gbm.predict(F)
    rng = np.random.default_rng(0)
    print("turns    rows  R2 linear  R2 LightGBM  pairs linear  pairs LightGBM")
    for lo in range(int(turns.min()), int(turns.max()) + 1, 10):
        m = te & (turns >= lo) & (turns < lo + 10)
        if m.sum() < 10:
            continue
        pl = np.concatenate([lin(t, X[m & (turns == t)]) for t in np.unique(turns[m])])
        ys = np.concatenate([y[m & (turns == t)] for t in np.unique(turns[m])])
        pgs = np.concatenate([pg[m & (turns == t)] for t in np.unique(turns[m])])
        # pairs only within a turn
        acc_l, acc_g = [], []
        for t in np.unique(turns[m]):
            k = m & (turns == t)
            acc_l.append(pair_accuracy(lin(t, X[k]), y[k], rng))
            acc_g.append(pair_accuracy(pg[k], y[k], rng))
        print(f"T{lo:2d}-{lo + 9:2d} {m.sum():6d}  {r2(pl, ys):9.3f}  {r2(pgs, ys):11.3f}  {np.nanmean(acc_l):12.3f}  {np.nanmean(acc_g):14.3f}")


if __name__ == "__main__":
    main()
