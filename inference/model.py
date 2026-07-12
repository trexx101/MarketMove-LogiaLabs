"""MarketMarkovNet architecture.

Matches Colab cell ``g6OcfSsDQAVq`` as described in ``Training_model_Design.md``:

- ``CausalConv1d``: causal 1-D convolution (right-side padding truncated so
  predictions at step t only see steps ≤ t).
- ``MarketMarkovNet``: 6-layer causal CNN backbone with GroupNorm + SiLU,
  three parallel draft heads (1H / 4H / 24H), and two low-rank Markov heads
  that refine 4H and 24H predictions sequentially using the preceding horizon.

Input convention
----------------
``forward`` receives a float32 tensor of shape ``(batch, n_features, seq_len)``
(channels-first, as expected by ``nn.Conv1d``).

Output convention
-----------------
Returns ``(pred_1h, pred_4h, pred_24h)``, each a scalar tensor of shape
``(batch, 1)``, already divided by 100 to reverse the ×100 training-target
scaling so callers receive values in raw log-return units.
"""

from __future__ import annotations

import torch
import torch.nn as nn


class CausalConv1d(nn.Module):
    """1-D convolution with causal (left-only) receptive field.

    Achieved by padding ``(kernel_size - 1) * dilation`` zeros to the *left*
    of the sequence and then trimming the right-side overhang from the output
    so the sequence length is preserved.
    """

    def __init__(
        self,
        in_channels: int,
        out_channels: int,
        kernel_size: int,
        dilation: int = 1,
    ) -> None:
        super().__init__()
        self._pad = (kernel_size - 1) * dilation
        self.conv = nn.Conv1d(
            in_channels,
            out_channels,
            kernel_size=kernel_size,
            dilation=dilation,
            padding=0,  # manual causal padding below
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        # x: (batch, channels, seq_len)
        if self._pad > 0:
            x = nn.functional.pad(x, (self._pad, 0))
        return self.conv(x)


class _BackboneBlock(nn.Module):
    """Single backbone block: CausalConv1d → GroupNorm → SiLU."""

    def __init__(
        self,
        in_channels: int,
        out_channels: int,
        kernel_size: int = 3,
        dilation: int = 1,
        num_groups: int = 8,
    ) -> None:
        super().__init__()
        self.conv = CausalConv1d(in_channels, out_channels, kernel_size, dilation)
        # GroupNorm operates over channels; num_groups must divide out_channels
        effective_groups = min(num_groups, out_channels)
        while out_channels % effective_groups != 0:
            effective_groups -= 1
        self.norm = nn.GroupNorm(effective_groups, out_channels)
        self.act = nn.SiLU()

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.act(self.norm(self.conv(x)))


class _LowRankMarkovHead(nn.Module):
    """Low-rank correction head for sequential Markov refinement.

    Models the influence of the previous horizon's prediction on the current
    one via a rank-bottleneck linear transformation:
        correction = V( U( concat(backbone_state, prev_draft) ) )
    where U: in_dim → rank and V: rank → 1.
    """

    def __init__(self, in_dim: int, rank: int = 4) -> None:
        super().__init__()
        self.down = nn.Linear(in_dim, rank, bias=False)
        self.up = nn.Linear(rank, 1, bias=True)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.up(self.down(x))


class MarketMarkovNet(nn.Module):
    """BTC swing-trading prediction network.

    Parameters
    ----------
    n_features:
        Number of input feature channels (columns in the feature window).
        Defaults to 3 (log_return_z, atr_72_z, vwap_dev_z).
    hidden_dim:
        Channel width after the first backbone layer.  Must be divisible by
        ``backbone_groups``.
    backbone_layers:
        Number of causal conv blocks (6 to match the Colab).
    kernel_size:
        Conv kernel width for all backbone blocks.
    markov_rank:
        Inner rank for the low-rank Markov heads.
    backbone_groups:
        ``num_groups`` passed to every ``GroupNorm`` in the backbone.
    """

    def __init__(
        self,
        n_features: int = 3,
        hidden_dim: int = 64,
        backbone_layers: int = 6,
        kernel_size: int = 3,
        markov_rank: int = 4,
        backbone_groups: int = 8,
    ) -> None:
        super().__init__()

        # ── Backbone ─────────────────────────────────────────────────────────
        layers: list[nn.Module] = []
        in_ch = n_features
        for i in range(backbone_layers):
            out_ch = hidden_dim if i == 0 else hidden_dim
            dilation = 2**i  # exponentially increasing receptive field
            layers.append(
                _BackboneBlock(
                    in_ch,
                    out_ch,
                    kernel_size=kernel_size,
                    dilation=dilation,
                    num_groups=backbone_groups,
                )
            )
            in_ch = out_ch
        self.backbone = nn.Sequential(*layers)

        # ── Parallel draft heads ──────────────────────────────────────────────
        self.draft_1h = nn.Linear(hidden_dim, 1)
        self.draft_4h = nn.Linear(hidden_dim, 1)
        self.draft_24h = nn.Linear(hidden_dim, 1)

        # ── Low-rank Markov heads ─────────────────────────────────────────────
        # 4H is refined using backbone state + draft_1h (1 extra feature)
        self.markov_4h = _LowRankMarkovHead(hidden_dim + 1, rank=markov_rank)
        # 24H is refined using backbone state + refined_4h (1 extra feature)
        self.markov_24h = _LowRankMarkovHead(hidden_dim + 1, rank=markov_rank)

    def forward(
        self, x: torch.Tensor
    ) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
        """Run a forward pass.

        Parameters
        ----------
        x:
            Float32 tensor of shape ``(batch, n_features, seq_len)``.

        Returns
        -------
        pred_1h, pred_4h, pred_24h:
            Each ``(batch, 1)`` float32, in raw log-return units (÷100).
        """
        # Backbone: (batch, hidden_dim, seq_len)
        h = self.backbone(x)

        # Take the last timestep as the summary state: (batch, hidden_dim)
        state = h[:, :, -1]

        # Draft predictions (in ×100 training space)
        d1h = self.draft_1h(state)    # (batch, 1)
        d4h = self.draft_4h(state)    # (batch, 1)
        d24h = self.draft_24h(state)  # (batch, 1)

        # Low-rank Markov refinement ─ each horizon conditions on the previous
        ctx_4h = torch.cat([state, d1h], dim=-1)        # (batch, hidden+1)
        r4h = self.markov_4h(ctx_4h)                    # (batch, 1)
        pred_4h_raw = d4h + r4h

        ctx_24h = torch.cat([state, pred_4h_raw], dim=-1)
        r24h = self.markov_24h(ctx_24h)
        pred_24h_raw = d24h + r24h

        # Reverse ×100 target scaling used during training
        scale = torch.tensor(100.0, dtype=x.dtype, device=x.device)
        pred_1h = d1h / scale
        pred_4h = pred_4h_raw / scale
        pred_24h = pred_24h_raw / scale

        return pred_1h, pred_4h, pred_24h


def load_model(model_path: str, **kwargs: object) -> MarketMarkovNet:
    """Load a trained ``MarketMarkovNet`` from a state-dict checkpoint.

    Parameters
    ----------
    model_path:
        Path to ``model.pt`` saved with ``torch.save(model.state_dict(), ...)``.
    **kwargs:
        Forwarded to ``MarketMarkovNet.__init__`` (e.g. ``n_features``,
        ``hidden_dim``) to override defaults when the checkpoint was trained
        with non-default hyperparameters.

    Returns
    -------
    MarketMarkovNet
        Model in CPU eval mode with gradients disabled.
    """
    model = MarketMarkovNet(**kwargs)  # type: ignore[arg-type]
    state = torch.load(model_path, map_location="cpu", weights_only=True)
    model.load_state_dict(state)
    model.eval()
    return model
