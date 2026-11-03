"""Learns a state's value (the final score of the play from that state) with a neural net that reads each city, on the
rows `civ6opt valuerows --city-out` writes, and compares it with LightGBM on the state's features alone.

  value_net.py --train <job dir>... --test <job dir>... [--epochs 30] [--lgb]

A job dir holds one valuerows run: the state rows (rows_<n>.tsv: play, turn, the state's features, the score) and the
city rows (cities.tsv: play, turn, the city's place, its features). The net passes every city through one small net,
sums, averages and takes the maximum of the results over the state's cities, and passes them with the state's
features and the turn through another; it learns the score minus the part already made (the feature in column
--made, 22: the objective so far), standardized within each turn so the early turns weigh as much as the late ones.
It is meant to run on the GPU (Windows, PyTorch with ROCm). The accuracy is value_compare.py's: among held-out states
of the same turn whose scores differ by more than 1%, the share of pairs the model orders right; "same plan" keeps
pairs within one test dir.
"""
import argparse
import glob
import os
import sys
import time

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from value_compare import pair_accuracy  # noqa: E402


def load_dir(d):
    """The state rows and city rows of one job dir, cached as .npy next to them."""
    cache = os.path.join(d, "rows.npz")
    if os.path.exists(cache):
        z = np.load(cache)
        return z["s"], z["c"]
    import pandas as pd
    rows = [p for p in glob.glob(os.path.join(d, "rows_*.tsv")) if not p.endswith(".log")]
    assert len(rows) == 1, f"{d}: one rows_<n>.tsv expected, found {rows}"
    s = pd.read_csv(rows[0], sep="\t", header=None, dtype=np.float64).to_numpy()
    c = pd.read_csv(os.path.join(d, "cities.tsv"), sep="\t", header=None, dtype=np.float32).to_numpy()
    np.savez(cache, s=s, c=c)
    return s, c


def gather(dirs):
    """States (file, play, turn, features, score) and their cities, the cities sorted by state: city rows
    off[i]:off[i+1] belong to state i."""
    S, C, file_of = [], [], []
    for k, d in enumerate(dirs):
        s, c = load_dir(d)
        # one key per (play, turn) within the dir; the city rows found by it
        key_s = s[:, 0].astype(np.int64) * 1000 + s[:, 1].astype(np.int64)
        key_c = c[:, 0].astype(np.int64) * 1000 + c[:, 1].astype(np.int64)
        order = np.argsort(key_s, kind="stable")
        s = s[order]
        key_s = key_s[order]
        corder = np.lexsort((c[:, 2], key_c))
        c, key_c = c[corder], key_c[corder]
        lo, hi = np.searchsorted(key_c, key_s, "left"), np.searchsorted(key_c, key_s, "right")
        assert (hi > lo).all(), f"{d}: a state without cities"
        S.append(s)
        C.append((c[:, 3:], lo, hi))
        file_of.append(np.full(len(s), k))
    s = np.vstack(S)
    counts = np.concatenate([hi - lo for _, lo, hi in C])
    cities = np.vstack([c[np.concatenate([np.arange(a, b) for a, b in zip(lo, hi)])] for c, lo, hi in C])
    off = np.concatenate([[0], np.cumsum(counts)])
    return np.concatenate(file_of), s, cities, off


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--train", nargs="+", required=True)
    ap.add_argument("--test", nargs="+", required=True)
    ap.add_argument("--made", type=int, default=22)
    ap.add_argument("--epochs", type=int, default=30)
    ap.add_argument("--batch", type=int, default=4096)
    ap.add_argument("--width", type=int, default=128)
    ap.add_argument("--lgb", action="store_true", help="also LightGBM on the state's features (all CPU threads)")
    a = ap.parse_args()
    import torch
    dev = "cuda" if torch.cuda.is_available() else "cpu"
    t0 = time.time()
    f_tr, s_tr, c_tr, off_tr = gather(a.train)
    f_te, s_te, c_te, off_te = gather(a.test)
    print(f"{len(s_tr)} training states, {len(s_te)} held out, {len(c_tr) + len(c_te)} city rows; loaded in {time.time() - t0:.0f}s on {dev}",
          flush=True)

    def split(s):
        return s[:, 1].astype(int), s[:, 2:-1], s[:, -1]
    t_tr, X_tr, y_tr = split(s_tr)
    t_te, X_te, y_te = split(s_te)
    r_tr, r_te = y_tr - X_tr[:, a.made], y_te - X_te[:, a.made]
    # the target standardized within each turn (training rows' statistics)
    mu, sd = {}, {}
    for t in np.unique(t_tr):
        v = r_tr[t_tr == t]
        mu[t], sd[t] = v.mean(), v.std() + 1e-6
    def tm(t):
        return np.array([mu.get(x, 0.0) for x in t]), np.array([sd.get(x, 1.0) for x in t])
    m_tr, d_tr = tm(t_tr)
    m_te, d_te = tm(t_te)
    z_tr = (r_tr - m_tr) / d_tr
    # inputs standardized (training statistics); log1p would hide scale, so plain z-scores, the turn scaled
    gx_mu, gx_sd = X_tr.mean(0), X_tr.std(0) + 1e-6
    cx_mu, cx_sd = c_tr.mean(0), c_tr.std(0) + 1e-6

    def to_dev(t, X, c, off):
        g = np.hstack([((X - gx_mu) / gx_sd), (t[:, None] / 65.0)]).astype(np.float32)
        return (torch.tensor(g, device=dev), torch.tensor(((c - cx_mu) / cx_sd).astype(np.float32), device=dev),
                torch.tensor(off, device=dev))
    G_tr, Cc_tr, O_tr = to_dev(t_tr, X_tr, c_tr, off_tr)
    G_te, Cc_te, O_te = to_dev(t_te, X_te, c_te, off_te)
    Z_tr = torch.tensor(z_tr.astype(np.float32), device=dev)
    w = a.width
    nc, ng = Cc_tr.shape[1], G_tr.shape[1]

    class Net(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.city = torch.nn.Sequential(torch.nn.Linear(nc, w), torch.nn.ReLU(), torch.nn.Linear(w, w), torch.nn.ReLU())
            self.head = torch.nn.Sequential(torch.nn.Linear(3 * w + ng, 2 * w), torch.nn.ReLU(), torch.nn.Linear(2 * w, 2 * w),
                                            torch.nn.ReLU(), torch.nn.Linear(2 * w, 1))

        def forward(self, g, c, seg, n):
            h = self.city(c)
            total = torch.zeros(n, w, device=h.device).index_add_(0, seg, h)
            count = torch.zeros(n, device=h.device).index_add_(0, seg, torch.ones(len(seg), device=h.device))
            top = torch.full((n, w), -1e9, device=h.device).scatter_reduce(0, seg[:, None].expand(-1, w), h, "amax")
            return self.head(torch.cat([total, total / count[:, None], top, g], 1)).squeeze(1)

    def batch(G, Cc, O, idx):
        lo, hi = O[idx], O[idx + 1]
        n = hi - lo
        seg = torch.repeat_interleave(torch.arange(len(idx), device=dev), n)
        start = torch.repeat_interleave(lo - torch.cumsum(n, 0) + n, n)
        rows = start + torch.arange(int(n.sum()), device=dev)
        return G[idx], Cc[rows], seg, len(idx)

    net = Net().to(dev)
    opt = torch.optim.AdamW(net.parameters(), lr=1e-3, weight_decay=1e-4)
    sched = torch.optim.lr_scheduler.CosineAnnealingLR(opt, a.epochs)
    n = len(G_tr)
    for ep in range(a.epochs):
        net.train()
        perm = torch.randperm(n, device=dev)
        tot = 0.0
        for i in range(0, n, a.batch):
            idx = perm[i:i + a.batch]
            g, c, seg, k = batch(G_tr, Cc_tr, O_tr, idx)
            loss = ((net(g, c, seg, k) - Z_tr[idx]) ** 2).mean()
            opt.zero_grad()
            loss.backward()
            opt.step()
            tot += loss.item() * len(idx)
        sched.step()
        print(f"epoch {ep + 1}: training loss {tot / n:.4f} ({time.time() - t0:.0f}s)", flush=True)
    net.eval()
    with torch.no_grad():
        out = []
        for i in range(0, len(G_te), 65536):
            idx = torch.arange(i, min(i + 65536, len(G_te)), device=dev)
            out.append(net(*batch(G_te, Cc_te, O_te, idx)).cpu().numpy())
    pred = {"net": np.concatenate(out) * d_te + m_te + X_te[:, a.made]}
    if a.lgb:
        import lightgbm as lgb
        F_tr, F_te = np.hstack([t_tr[:, None], X_tr]), np.hstack([t_te[:, None], X_te])
        params = {"objective": "regression", "learning_rate": 0.05, "num_leaves": 63, "min_data_in_leaf": 20, "verbose": -1}
        gbm = lgb.train(params, lgb.Dataset(F_tr, r_tr), num_boost_round=600)
        pred["LightGBM"] = gbm.predict(F_te) + X_te[:, a.made]
    rng = np.random.default_rng(0)
    print("model     turns    pairs  same plan")
    for name, p in pred.items():
        acc, same = {}, {}
        for t in np.unique(t_te):
            k = t_te == t
            acc[t] = pair_accuracy(p[k], y_te[k], X_te[k], rng)
            same[t] = pair_accuracy(p[k], y_te[k], X_te[k], rng, group=f_te[k])
        for lo in range(2, 66, 10):
            ts = [t for t in acc if lo <= t < lo + 10]
            print(f"{name:9s} T{lo:2d}-{lo + 9:2d}  {np.nanmean([acc[t] for t in ts]):.3f}  {np.nanmean([same[t] for t in ts]):.3f}")
        print(f"{name:9s} all       {np.nanmean(list(acc.values())):.3f}  {np.nanmean(list(same.values())):.3f}")
        for k in range(len(a.test)):
            per = [pair_accuracy(p[(t_te == t) & (f_te == k)], y_te[(t_te == t) & (f_te == k)], X_te[(t_te == t) & (f_te == k)], rng)
                   for t in np.unique(t_te[f_te == k])]
            label = os.path.basename(os.path.normpath(a.test[k]))
            print(f"{name:9s} {label}: same plan {np.nanmean(per):.3f}")


if __name__ == "__main__":
    main()
