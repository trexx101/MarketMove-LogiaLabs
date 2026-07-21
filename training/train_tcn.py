#!/usr/bin/env python3
"""
# %% Colab setup
# !pip install torch scikit-learn pandas numpy tqdm scipy

D3: TCN training + walk-forward evaluation + deploy gate.
Changes from previous run:
- barrier_c = 0.5 (was 0.15 — barriers now wider, ~1% instead of 0.21%)
- Magnitude targets clipped to [-3, 3] (was unbounded — 50x outliers dominated loss)
- Uses fetch_features.py to load REAL funding_rate, basis_z, ob_imbalance
  from Binance Futures API + CSV taker volume (was all zeros).
- Signed-regression MSE (no focal loss, no class-imbalance degeneracy).

Train with: python training/train_tcn.py
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

# ── Config ────────────────────────────────────────────────────────────────────

class Config:
    seq_len = 72
    horizons = [12, 36, 72]  # 12h / 36h / 72h lookahead
    horizon_labels = ['H1', 'H2', 'H3']
    barrier_c = 0.5           # k = c * ATR / close (was 0.15)
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


# ── TCN Model ──────────────────────────────────────────────────────────────────

class CausalConv1d(nn.Module):
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__()
        self.padding = (kernel_size - 1) * dilation
        self.conv = nn.Conv1d(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)

    def forward(self, x):
        return self.conv(nn.functional.pad(x, (self.padding, 0)))


class TCN(nn.Module):
    def __init__(self, in_dim=6, hidden_dim=64, dropout=0.1, n_horizons=3):
        super().__init__()
        dilations = [1, 2, 4, 8, 16, 32]
        layers = []
        for i, d in enumerate(dilations):
            ich = in_dim if i == 0 else hidden_dim
            layers.extend([
                CausalConv1d(ich, hidden_dim, kernel_size=3, dilation=d),
                nn.GroupNorm(1, hidden_dim),
                nn.SiLU(),
                nn.Dropout(dropout),
            ])
        self.backbone = nn.Sequential(*layers)
        self.heads = nn.ModuleList([
            nn.Linear(hidden_dim, 1) for _ in range(n_horizons)
        ])

    def forward(self, x):
        x = x.permute(0, 2, 1)
        feat = self.backbone(x)[:, :, -1]
        return [head(feat).squeeze(-1) for head in self.heads]


# ── Dataset ────────────────────────────────────────────────────────────────────

class WindowDataset(Dataset):
    """Sliding-window: (seq_len, 6) → 3 signed-regression targets (clipped)."""
    def __init__(self, X, labels_dict, seq_len, horizon_labels, mag_clip=MAG_CLIP):
        self.X = X
        self.labels = labels_dict
        self.seq_len = seq_len
        self.horizon_labels = horizon_labels
        self.mag_clip = mag_clip
        self.n = len(X) - seq_len

    def __len__(self):
        return max(0, self.n)

    def __getitem__(self, idx):
        window = torch.FloatTensor(self.X[idx: idx + self.seq_len])
        sample = {'features': window}
        li = idx + self.seq_len
        for hkey in self.horizon_labels:
            dirs, mags = self.labels[hkey]
            if li < len(mags):
                target = float(np.clip(mags[li], -self.mag_clip, self.mag_clip))
                sample[f'target_{hkey}'] = torch.FloatTensor([target])
            else:
                sample[f'target_{hkey}'] = torch.FloatTensor([0.0])
        return sample


# ── Training + Walk-Forward ────────────────────────────────────────────────────

def compute_ic(preds, trues):
    if len(preds) < 2 or np.std(preds) < 1e-12 or np.std(trues) < 1e-12:
        return 0.0
    return float(pearsonr(preds, trues)[0])


def train_walk_forward(X, labels_dict):
    n = len(X)
    embargo = max(72, max(Config.horizons) + Config.seq_len)
    tscv = TimeSeriesSplit(n_splits=Config.n_folds, gap=embargo)
    fold_metrics = []

    for fold_i, (train_idx, val_idx) in enumerate(tscv.split(X)):
        print(f"\n===== FOLD {fold_i+1}/{Config.n_folds} (train={len(train_idx)}, val={len(val_idx)}) =====")

        train_ds = WindowDataset(X[train_idx], labels_dict, Config.seq_len, Config.horizon_labels)
        val_ds = WindowDataset(X[val_idx], labels_dict, Config.seq_len, Config.horizon_labels)
        if len(train_ds) == 0 or len(val_ds) == 0:
            print("  insufficient data, skipping")
            continue

        train_loader = DataLoader(train_ds, batch_size=Config.batch_size, shuffle=False, drop_last=True)
        val_loader = DataLoader(val_ds, batch_size=Config.batch_size, shuffle=False, drop_last=True)

        model = TCN(in_dim=6, hidden_dim=Config.hidden_dim, dropout=Config.dropout,
                    n_horizons=len(Config.horizons)).to(Config.device)
        opt = optim.AdamW(model.parameters(), lr=Config.lr, weight_decay=Config.weight_decay)
        loss_fn = nn.MSELoss()

        for epoch in range(Config.epochs):
            model.train()
            total = 0.0
            for batch in train_loader:
                feat = batch['features'].to(Config.device)
                preds = model(feat)
                loss = loss_fn(preds[0], batch[f'target_{Config.horizon_labels[0]}'].to(Config.device).squeeze(-1))
                for i in range(1, len(Config.horizon_labels)):
                    loss = loss + loss_fn(preds[i], batch[f'target_{Config.horizon_labels[i]}'].to(Config.device).squeeze(-1))
                opt.zero_grad()
                loss.backward()
                nn.utils.clip_grad_norm_(model.parameters(), 1.0)
                opt.step()
                total += loss.item()
            if (epoch + 1) % 10 == 0:
                print(f"  epoch {epoch+1} | loss {total/len(train_loader):.6f}")

        # ── OOS evaluation ────────────────────────────────────────────────────
        model.eval()
        all_scores = {h: [] for h in Config.horizon_labels}
        all_trues = {h: [] for h in Config.horizon_labels}
        with torch.no_grad():
            for batch in val_loader:
                feat = batch['features'].to(Config.device)
                preds = model(feat)
                for i, hkey in enumerate(Config.horizon_labels):
                    all_scores[hkey].extend(preds[i].cpu().numpy())
                    all_trues[hkey].extend(batch[f'target_{hkey}'].squeeze(-1).numpy())

        fold = {}
        for hkey in Config.horizon_labels:
            preds = np.array(all_scores[hkey])
            trues = np.array(all_trues[hkey])
            ic = compute_ic(preds, trues)
            equity = float(np.cumsum(np.sign(preds) * trues)[-1]) if len(preds) > 0 else 0.0
            fold[hkey] = {'IC': ic, 'equity': equity}
            print(f"  OOS {hkey}: IC={ic:+.4f}  equity={equity:+.4f}")

        fold_metrics.append(fold)

    return fold_metrics


# ── Deploy Gate ────────────────────────────────────────────────────────────────

def check_deploy_gate(fold_metrics):
    if not fold_metrics:
        return False
    for h in Config.horizon_labels:
        ics = [f[h]['IC'] for f in fold_metrics if h in f]
        equities = [f[h]['equity'] for f in fold_metrics if h in f]
        if not ics:
            continue
        mean_ic = np.nanmean(ics)
        mean_equity = np.nanmean(equities)
        print(f"  {h}: mean IC={mean_ic:+.4f}  mean equity={mean_equity:+.4f}")
        if mean_ic > Config.ic_gate and mean_equity > Config.equity_gate:
            return True
    return False


def _train_full(X, labels, norm_stats):
    """Train final model on full dataset, export artifacts."""
    full_ds = WindowDataset(X, labels, Config.seq_len, Config.horizon_labels)
    full_loader = DataLoader(full_ds, batch_size=Config.batch_size, shuffle=False, drop_last=True)
    model = TCN(in_dim=6, hidden_dim=Config.hidden_dim, dropout=Config.dropout,
                n_horizons=len(Config.horizons)).to(Config.device)
    opt = optim.AdamW(model.parameters(), lr=Config.lr, weight_decay=Config.weight_decay)
    loss_fn = nn.MSELoss()
    for epoch in range(Config.epochs):
        model.train()
        for batch in full_loader:
            feat = batch['features'].to(Config.device)
            preds = model(feat)
            loss = loss_fn(preds[0], batch[f'target_{Config.horizon_labels[0]}'].to(Config.device).squeeze(-1))
            for i in range(1, len(Config.horizon_labels)):
                loss = loss + loss_fn(preds[i], batch[f'target_{Config.horizon_labels[i]}'].to(Config.device).squeeze(-1))
            opt.zero_grad()
            loss.backward()
            nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step()

    ts = datetime.now().strftime("%Y%m%d_%H%M%S")
    os.makedirs("models", exist_ok=True)
    torch.save(model.state_dict(), f"models/market_markov_net_v2_{ts}.pt")
    with open(f"models/norm_stats_{ts}.json", 'w') as f:
        json.dump(norm_stats, f, indent=2)
    meta = {"train_date": ts, "features": 6, "horizons": Config.horizons,
            "barrier_c": Config.barrier_c, "mag_clip": MAG_CLIP}
    with open(f"models/model_meta_{ts}.json", 'w') as f:
        json.dump(meta, f, indent=2)
    print(f"Exported: models/market_markov_net_v2_{ts}.pt + norm_stats_{ts}.json + meta_{ts}.json")


# ── Main ───────────────────────────────────────────────────────────────────────

def main():
    from labels import volatility_scaled_labels, build_feature_matrix
    from fetch_features import fetch_and_merge_features

    data_dir = os.environ.get('DATA_DIR', '/content/drive/MyDrive/QuantData/BTCUSDT_1H')
    spot_columns = ['open_time', 'open', 'high', 'low', 'close', 'volume', 'close_time',
                    'quote_vol', 'trades', 'taker_buy_base', 'taker_buy_quote', 'ignore']

    # ── Load spot + fetch real funding/basis/ob data ───────────────────────────
    print("=== Fetching real Binance Futures data (funding, basis, ob_imbalance) ===")
    df = fetch_and_merge_features(data_dir, spot_columns, symbol="BTCUSDT")
    print(f"Loaded {len(df)} candles\n")

    # ── Build features ─────────────────────────────────────────────────────────
    X, norm_stats = build_feature_matrix(df, lookback=Config.seq_len)
    print(f"Features: {X.shape}")
    # Print feature stats so you can verify they're not all zeros
    for i, name in enumerate(['vol_regime', 'vol_break', 'funding_rate', 'basis_z', 'llm_bull_prob', 'ob_imbalance']):
        print(f"  {name}: mean={np.nanmean(X[:, i]):.6f} std={np.nanstd(X[:, i]):.6f}")

    # ── Build labels ───────────────────────────────────────────────────────────
    labels = volatility_scaled_labels(
        df, lookback=Config.seq_len, c=Config.barrier_c,
        horizons_bars=tuple(Config.horizons), mag_clip=MAG_CLIP,
    )
    print(f"\nLabels: {len(labels['H1'][0])} samples  Embargo: {labels['embargo']}")
    print(f"Penetration rates: {labels['penetration_rates']}")
    for hkey in Config.horizon_labels:
        dirs, mags = labels[hkey]
        n_up = int(np.sum(dirs > 0))
        n_dn = int(np.sum(dirs < 0))
        n_neutral = int(np.sum(dirs == 0))
        print(f"  {hkey}: up={n_up} down={n_dn} neutral={n_neutral} "
              f"mag_mean={np.mean(mags):.4f} mag_std={np.std(mags):.4f} mag_max={np.max(np.abs(mags)):.1f}")

    for hkey in Config.horizon_labels:
        pen_rate = labels['penetration_rates'][hkey]
        if pen_rate < 0.02:
            print(f"WARNING: {hkey} penetration rate {pen_rate:.2%} is very low.")
        if pen_rate > 0.95:
            print(f"WARNING: {hkey} penetration rate {pen_rate:.2%} is very high — widen barrier c.")

    # ── Walk-forward training ─────────────────────────────────────────────────
    fold_metrics = train_walk_forward(X, labels)

    # ── Deploy gate ───────────────────────────────────────────────────────────
    print("\n=== WALK-FORWARD OOS IC SUMMARY ===")
    passes = check_deploy_gate(fold_metrics)
    if passes:
        print("DEPLOY GATE PASSED — training final model on full dataset")
        _train_full(X, labels, norm_stats)
    else:
        print("NO EDGE — DO NOT DEPLOY")
        print("Mean OOS IC did not exceed 0.03 on any horizon with positive equity.")
        print("Reconsider alpha source or feature engineering.")
        exit(1)


if __name__ == "__main__":
    main()