#!/usr/bin/env python3
"""
# %% Colab setup
# !pip install torch scikit-learn pandas numpy tqdm scipy

D3: TCN training + walk-forward evaluation + deploy gate.
Fixes from first Colab run (all predictions were neutral, IC=nan):
- Widened horizons to 12/36/72 bars (12h/36h/72h) — more penetration events.
- Tightened barrier k to c=0.15 * ATR (was 0.50).
- Replaced separate classification+magnitude heads + focal loss with a SINGLE
  signed-regression target (direction * magnitude) trained with plain MSE.
  This eliminates the class-imbalance degeneracy (>99% neutral class).
- compute_ic returns 0.0 (not nan) on zero-variance predictions.
- Prints penetration rates + label stats before training so you can verify
  labels aren't degenerate.

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

# ── Config ────────────────────────────────────────────────────────────────────

class Config:
    seq_len = 72            # lookback bars (matches engine feature_window_size)
    horizons = [12, 36, 72] # 12h / 36h / 72h lookahead for penetration
    horizon_labels = ['H1', 'H2', 'H3']
    barrier_c = 0.15       # k = c * ATR (narrower = more penetration events)
    batch_size = 64
    epochs = 50
    lr = 1e-3
    weight_decay = 1e-4
    dropout = 0.1
    hidden_dim = 64
    n_folds = 5
    device = 'cuda' if torch.cuda.is_available() else 'cpu'
    ic_gate = 0.03          # min mean OOS IC to deploy
    equity_gate = 0.0       # min OOS equity to deploy


# ── TCN Model ──────────────────────────────────────────────────────────────────

class CausalConv1d(nn.Module):
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__()
        self.padding = (kernel_size - 1) * dilation
        self.conv = nn.Conv1d(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)

    def forward(self, x):
        return self.conv(nn.functional.pad(x, (self.padding, 0)))


class TCN(nn.Module):
    """TCN with a single signed-regression head per horizon."""
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
        # x: (batch, seq_len, in_dim) → (batch, in_dim, seq_len)
        x = x.permute(0, 2, 1)
        feat = self.backbone(x)[:, :, -1]  # last timestep
        return [head(feat).squeeze(-1) for head in self.heads]


# ── Dataset ────────────────────────────────────────────────────────────────────

class WindowDataset(Dataset):
    """Sliding-window: (seq_len, 6) → 3 signed-regression targets."""
    def __init__(self, X, labels_dict, seq_len, horizon_labels):
        self.X = X
        self.labels = labels_dict
        self.seq_len = seq_len
        self.horizon_labels = horizon_labels
        self.n = len(X) - seq_len

    def __len__(self):
        return max(0, self.n)

    def __getitem__(self, idx):
        window = torch.FloatTensor(self.X[idx: idx + self.seq_len])
        sample = {'features': window}
        li = idx + self.seq_len  # label aligned with last bar of window
        for hkey in self.horizon_labels:
            dirs, mags = self.labels[hkey]
            # Target = signed magnitude (direction * magnitude), already signed.
            if li < len(mags):
                sample[f'target_{hkey}'] = torch.FloatTensor([mags[li]])
            else:
                sample[f'target_{hkey}'] = torch.FloatTensor([0.0])
        return sample


# ── Training + Walk-Forward ────────────────────────────────────────────────────

def compute_ic(preds, trues):
    if len(preds) < 2 or np.std(preds) < 1e-12 or np.std(trues) < 1e-12:
        return 0.0  # not nan — 0 means no edge, gate eval stays sane
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
                loss = 0.0
                for i, hkey in enumerate(Config.horizon_labels):
                    target = batch[f'target_{hkey}'].to(Config.device).squeeze(-1)
                    loss += loss_fn(preds[i], target)
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


# ── Main ───────────────────────────────────────────────────────────────────────

def main():
    from labels import volatility_scaled_labels, build_feature_matrix
    import pandas as pd
    import glob

    data_dir = os.environ.get('DATA_DIR', '/content/drive/MyDrive/QuantData/BTCUSDT_1H')
    all_files = sorted(glob.glob(os.path.join(data_dir, "*.csv")))
    columns = ['open_time', 'open', 'high', 'low', 'close', 'volume', 'close_time',
               'quote_vol', 'trades', 'taker_buy_base', 'taker_buy_quote', 'ignore']
    df_list = [pd.read_csv(f, names=columns) for f in all_files]
    df = pd.concat(df_list, ignore_index=True)
    df['open_time'] = df['open_time'].astype('int64')
    df = df.sort_values('open_time').reset_index(drop=True)

    if 'funding_rate' not in df.columns:
        df['funding_rate'] = 0.0
    if 'basis_z' not in df.columns:
        df['basis_z'] = 0.0
    if 'ob_imbalance' not in df.columns:
        df['ob_imbalance'] = 0.0

    print(f"Loaded {len(df)} candles from {len(all_files)} files")

    # ── Build features ─────────────────────────────────────────────────────────
    X, norm_stats = build_feature_matrix(df, lookback=Config.seq_len)
    print(f"Features: {X.shape}")

    # ── Build labels ───────────────────────────────────────────────────────────
    labels = volatility_scaled_labels(
        df, lookback=Config.seq_len, c=Config.barrier_c,
        horizons_bars=tuple(Config.horizons),
    )
    print(f"Labels: {len(labels['H1'][0])} samples  Embargo: {labels['embargo']}")
    print(f"Penetration rates: {labels['penetration_rates']}")
    # Sanity: print label stats per horizon
    for hkey in Config.horizon_labels:
        dirs, mags = labels[hkey]
        n_up = int(np.sum(dirs > 0))
        n_dn = int(np.sum(dirs < 0))
        n_neutral = int(np.sum(dirs == 0))
        print(f"  {hkey}: up={n_up} down={n_dn} neutral={n_neutral} "
              f"mag_mean={np.mean(mags):.4f} mag_std={np.std(mags):.4f}")

    # Check for degenerate labels
    for hkey in Config.horizon_labels:
        pen_rate = labels['penetration_rates'][hkey]
        if pen_rate < 0.02:
            print(f"WARNING: {hkey} penetration rate {pen_rate:.2%} is very low — "
                  "consider widening horizons or tightening barrier c.")

    # ── Walk-forward training ─────────────────────────────────────────────────
    fold_metrics = train_walk_forward(X, labels)

    # ── Deploy gate ───────────────────────────────────────────────────────────
    print("\n=== WALK-FORWARD OOS IC SUMMARY ===")
    passes = check_deploy_gate(fold_metrics)
    if passes:
        print("DEPLOY GATE PASSED — training final model on full dataset")
        # Train on full data
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
                loss = sum(loss_fn(preds[i], batch[f'target_{h}'].to(Config.device).squeeze(-1))
                           for i, h in enumerate(Config.horizon_labels))
                opt.zero_grad()
                loss.backward()
                nn.utils.clip_grad_norm_(model.parameters(), 1.0)
                opt.step()
        # Export
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        os.makedirs("models", exist_ok=True)
        torch.save(model.state_dict(), f"models/market_markov_net_v2_{ts}.pt")
        with open(f"models/norm_stats_{ts}.json", 'w') as f:
            json.dump(norm_stats, f, indent=2)
        print(f"Exported: models/market_markov_net_v2_{ts}.pt + models/norm_stats_{ts}.json")
    else:
        print("NO EDGE — DO NOT DEPLOY")
        print("Mean OOS IC did not exceed 0.03 on any horizon with positive equity.")
        print("Reconsider alpha source (funding/basis/order-flow may not carry edge at this horizon).")
        exit(1)


if __name__ == "__main__":
    main()