#!/usr/bin/env python3
"""
# %% Colab setup
# !pip install torch scikit-learn pandas numpy tqdm scipy

D3: TCN training + walk-forward evaluation + deploy gate fixed.
Revamp: adaptive vol regime, barrier_c=2.0, Huber loss, LR scheduler, early stopping.
"""
import torch
import torch.nn as nn
import torch.optim as optim
import numpy as np
import json
import os
from datetime import datetime
from sklearn.model_selection import TimeSeriesSplit
from torch.utils.data import Dataset, DataLoader
from scipy.stats import pearsonr

MAG_CLIP = 3.0

class Config:
    seq_len = 72
    horizons = [12, 36, 72]
    horizon_labels = ['H1', 'H2', 'H3']
    barrier_c = 2.0
    batch_size = 64
    epochs = 50
    lr = 1e-3
    weight_decay = 1e-4
    dropout = 0.1
    hidden_dim = 64
    n_folds = 5
    device = 'cuda' if torch.cuda.is_available() else 'cpu'
    ic_gate = 0.03
    equity_gate = 0.0


class CausalConv1d(nn.Module):
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__()
        self.padding = (kernel_size - 1) * dilation
        self.conv = nn.Conv1d(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
    def forward(self, x):
        return self.conv(nn.functional.pad(x, (self.padding, 0)))


class ResidualBlock(nn.Module):
    def __init__(self, in_ch, out_ch, kernel_size, dilation, dropout):
        super().__init__()
        self.conv1 = CausalConv1d(in_ch, out_ch, kernel_size, dilation)
        self.conv2 = CausalConv1d(out_ch, out_ch, kernel_size, dilation)
        self.norm1 = nn.GroupNorm(1, out_ch)
        self.norm2 = nn.GroupNorm(1, out_ch)
        self.dropout = nn.Dropout(dropout)
        self.activation = nn.SiLU()
        self.residual = nn.Conv1d(in_ch, out_ch, 1) if in_ch != out_ch else nn.Identity()
    def forward(self, x):
        residual = self.residual(x)
        out = self.conv1(x); out = self.norm1(out); out = self.activation(out); out = self.dropout(out)
        out = self.conv2(out); out = self.norm2(out)
        return self.activation(out + residual)


class TCN(nn.Module):
    def __init__(self, in_dim=6, hidden_dim=64, dropout=0.1, n_horizons=3):
        super().__init__()
        self.input_proj = nn.Linear(in_dim, hidden_dim)
        layers = [ResidualBlock(hidden_dim, hidden_dim, 3, d, dropout) for d in [1, 2, 4, 8, 16, 32, 64]]
        self.backbone = nn.Sequential(*layers)
        self.heads = nn.ModuleList([
            nn.Sequential(nn.Linear(hidden_dim, hidden_dim // 2), nn.SiLU(), nn.Dropout(dropout),
                          nn.Linear(hidden_dim // 2, 1)) for _ in range(n_horizons)
        ])
        self.loss_weights = nn.Parameter(torch.ones(n_horizons))
    def forward(self, x):
        x = self.input_proj(x).permute(0, 2, 1)
        feat = self.backbone(x)[:, :, -1]
        return [head(feat).squeeze(-1) for head in self.heads]


class WindowDataset(Dataset):
    def __init__(self, X, labels_dict, seq_len, horizon_labels, mag_clip=MAG_CLIP):
        self.X = X; self.labels = labels_dict; self.seq_len = seq_len
        self.horizon_labels = horizon_labels; self.mag_clip = mag_clip; self.n = len(X) - seq_len
    def __len__(self): return max(0, self.n)
    def __getitem__(self, idx):
        window = torch.FloatTensor(self.X[idx: idx + self.seq_len])
        sample = {'features': window}; li = idx + self.seq_len
        for hkey in self.horizon_labels:
            _, mags = self.labels[hkey]
            t = float(np.clip(mags[li], -self.mag_clip, self.mag_clip)) if li < len(mags) else 0.0
            sample[f'target_{hkey}'] = torch.FloatTensor([t])
        return sample


def compute_ic(preds, trues):
    if len(preds) < 2 or np.std(preds) < 1e-12 or np.std(trues) < 1e-12: return 0.0
    return float(pearsonr(preds, trues)[0])


def evaluate_model(model, loader, horizon_labels, device):
    model.eval(); losses = []; fn = nn.SmoothL1Loss()
    with torch.no_grad():
        for batch in loader:
            feat = batch['features'].to(device); preds = model(feat)
            for i, h in enumerate(horizon_labels):
                losses.append(fn(preds[i], batch[f'target_{h}'].to(device).squeeze(-1)).item())
    return float(np.mean(losses)) if losses else float('inf')


def train_walk_forward(X, labels_dict):
    n = len(X); embargo = max(72, max(Config.horizons) + Config.seq_len)
    tscv = TimeSeriesSplit(n_splits=Config.n_folds, gap=embargo)
    fold_metrics = []
    for fold_i, (ti, vi) in enumerate(tscv.split(X)):
        print(f"\n===== FOLD {fold_i+1}/{Config.n_folds} (train={len(ti)}, val={len(vi)}) =====")
        tr_ds = WindowDataset(X[ti], labels_dict, Config.seq_len, Config.horizon_labels)
        va_ds = WindowDataset(X[vi], labels_dict, Config.seq_len, Config.horizon_labels)
        if not (len(tr_ds) and len(va_ds)): print("  skip"); continue
        tr_ld = DataLoader(tr_ds, Config.batch_size, shuffle=True, drop_last=True)
        va_ld = DataLoader(va_ds, Config.batch_size)
        model = TCN(in_dim=X.shape[1], hidden_dim=Config.hidden_dim, dropout=Config.dropout,
                    n_horizons=len(Config.horizons)).to(Config.device)
        opt = optim.AdamW(model.parameters(), lr=Config.lr, weight_decay=Config.weight_decay)
        sch = optim.lr_scheduler.OneCycleLR(opt, Config.lr, epochs=Config.epochs,
                                             steps_per_epoch=len(tr_ld), pct_start=0.3)
        fn = nn.SmoothL1Loss()
        best_val, wait = float('inf'), 0
        for ep in range(Config.epochs):
            model.train(); total = 0.0
            for b in tr_ld:
                feat = b['features'].to(Config.device); preds = model(feat)
                w = torch.softmax(model.loss_weights, dim=0)
                loss = sum(w[i] * fn(preds[i], b[f'target_{h}'].to(Config.device).squeeze(-1))
                           for i, h in enumerate(Config.horizon_labels))
                opt.zero_grad(); loss.backward(); nn.utils.clip_grad_norm_(model.parameters(), 1.0); opt.step(); sch.step()
                total += loss.item()
            if (ep + 1) % 10 == 0: print(f"  epoch {ep+1} | loss {total/len(tr_ld):.4f}")
            if ep % 5 == 4:
                v = evaluate_model(model, va_ld, Config.horizon_labels, Config.device)
                if v < best_val: best_val, wait = v, 0
                else: wait += 1
                if wait >= 10: print(f"  early stop at {ep+1}"); break
        model.eval()
        sc, tr = {h: [] for h in Config.horizon_labels}, {h: [] for h in Config.horizon_labels}
        with torch.no_grad():
            for b in va_ld:
                feat = b['features'].to(Config.device); preds = model(feat)
                for i, h in enumerate(Config.horizon_labels):
                    sc[h].extend(preds[i].cpu().numpy()); tr[h].extend(b[f'target_{h}'].squeeze(-1).numpy())
        fold = {}
        for h in Config.horizon_labels:
            p, t = np.array(sc[h]), np.array(tr[h]); ic = compute_ic(p, t)
            eq = float(np.cumsum(np.sign(p) * t)[-1]) if len(p) > 0 else 0.0
            fold[h] = {'IC': ic, 'equity': eq}; print(f"  OOS {h}: IC={ic:+.4f}  equity={eq:+.4f}")
        fold_metrics.append(fold)
    return fold_metrics


def check_deploy_gate(fold_metrics):
    if not fold_metrics: return False
    for h in Config.horizon_labels:
        ics = [f[h]['IC'] for f in fold_metrics if h in f]; eqs = [f[h]['equity'] for f in fold_metrics if h in f]
        if not ics: continue
        mic, meq = np.nanmean(ics), np.nanmean(eqs)
        print(f"  {h}: mean IC={mic:+.4f}  mean equity={meq:+.4f}")
        if mic > Config.ic_gate and meq > Config.equity_gate: return True
    return False


def _train_full(X, labels, norm_stats, feature_names):
    ds = WindowDataset(X, labels, Config.seq_len, Config.horizon_labels)
    ld = DataLoader(ds, Config.batch_size, shuffle=True, drop_last=True)
    model = TCN(in_dim=X.shape[1], hidden_dim=Config.hidden_dim, dropout=Config.dropout,
                n_horizons=len(Config.horizons)).to(Config.device)
    opt = optim.AdamW(model.parameters(), lr=Config.lr, weight_decay=Config.weight_decay)
    sch = optim.lr_scheduler.OneCycleLR(opt, Config.lr, epochs=Config.epochs,
                                         steps_per_epoch=len(ld), pct_start=0.3)
    fn = nn.SmoothL1Loss()
    for _ in range(Config.epochs):
        model.train()
        for b in ld:
            feat = b['features'].to(Config.device); preds = model(feat)
            w = torch.softmax(model.loss_weights, dim=0)
            loss = sum(w[i] * fn(preds[i], b[f'target_{h}'].to(Config.device).squeeze(-1))
                       for i, h in enumerate(Config.horizon_labels))
            opt.zero_grad(); loss.backward(); nn.utils.clip_grad_norm_(model.parameters(), 1.0); opt.step(); sch.step()
    ts = datetime.now().strftime("%Y%m%d_%H%M%S")
    os.makedirs("models", exist_ok=True)
    torch.save(model.state_dict(), f"models/market_markov_net_v2_{ts}.pt")
    with open(f"models/norm_stats_{ts}.json", 'w') as f: json.dump(norm_stats, f, indent=2)
    meta = {"train_date": ts, "features": X.shape[1], "feature_names": feature_names,
            "horizons": Config.horizons, "barrier_c": Config.barrier_c, "mag_clip": MAG_CLIP}
    with open(f"models/model_meta_{ts}.json", 'w') as f: json.dump(meta, f, indent=2)
    print(f"Exported: models/market_markov_net_v2_{ts}.pt + norm_stats_{ts}.json + meta_{ts}.json")


def main():
    from labels import volatility_scaled_labels, build_feature_matrix, calibrate_barrier_c
    from fetch_features import fetch_and_merge_features

    data_dir = os.environ.get('DATA_DIR', '/content/drive/MyDrive/QuantData/BTCUSDT_1H')
    spot_cols = ['open_time', 'open', 'high', 'low', 'close', 'volume', 'close_time',
                 'quote_vol', 'trades', 'taker_buy_base', 'taker_buy_quote', 'ignore']
    print("=== Fetching data ===")
    df = fetch_and_merge_features(data_dir, spot_cols, symbol="BTCUSDT")
    print(f"Loaded {len(df)} candles\n")

    print("Calibrating barrier c...")
    best_c = calibrate_barrier_c(df, target_penetration=0.5, horizons_bars=(Config.horizons[0],))
    Config.barrier_c = best_c
    print(f"barrier_c = {best_c:.2f}")

    X, norm_stats, feature_names = build_feature_matrix(df, lookback=Config.seq_len)
    print(f"Features: {X.shape}")
    for i, name in enumerate(feature_names):
        print(f"  {name}: mean={np.nanmean(X[:, i]):.6f} std={np.nanstd(X[:, i]):.6f}")

    labels = volatility_scaled_labels(df, lookback=Config.seq_len, c=Config.barrier_c,
                                       horizons_bars=tuple(Config.horizons), mag_clip=MAG_CLIP)
    print(f"Labels: {len(labels['H1'][0])} samples  Embargo: {labels['embargo']}")
    print(f"Penetration rates: {labels['penetration_rates']}")
    for hkey in Config.horizon_labels:
        d, m = labels[hkey]
        print(f"  {hkey}: up={int(np.sum(d>0))} dn={int(np.sum(d<0))} neutral={int(np.sum(d==0))} "
              f"mag_m={np.mean(m):.4f} mag_s={np.std(m):.4f} mag_max={np.max(np.abs(m)):.1f}")
    for hkey in Config.horizon_labels:
        pr = labels['penetration_rates'][hkey]
        if pr < 0.02: print(f"WARNING: {hkey} penetration {pr:.2%} very low")
        if pr > 0.95: print(f"WARNING: {hkey} penetration {pr:.2%} very high")

    fold_metrics = train_walk_forward(X, labels)
    print("\n=== WALK-FORWARD OOS IC SUMMARY ===")
    passes = check_deploy_gate(fold_metrics)
    if passes:
        print("DEPLOY GATE PASSED — training final model")
        _train_full(X, labels, norm_stats, feature_names)
    else:
        print("NO EDGE — DO NOT DEPLOY"); exit(1)


if __name__ == "__main__":
    main()