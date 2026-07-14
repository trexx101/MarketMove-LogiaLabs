"""MarketMarkovNet architecture.

Matches the Colab training code exactly so ``model.pt`` loads without
state_dict key errors.

- ``CausalConv1d``: causal 1-D convolution (left-only padding so predictions
  at step t only see steps ≤ t).
- ``MarketMarkovNet``: 6-layer causal CNN backbone (flat ``nn.Sequential``
  of ``CausalConv1d → GroupNorm(1, hidden_dim) → SiLU``, dilations
  1, 2, 4, 8, 16, 32), three parallel draft heads (1H / 4H / 24H), and
  two low-rank Markov heads that refine 4H and 24H predictions
  sequentially using the preceding horizon's output.

Input convention
----------------
``forward`` receives a float32 tensor of shape ``(batch, seq_len, n_features)``
(the Colab convention).  The model transposes to channels-first internally
before the convolutional backbone.

Output convention
-----------------
Returns ``(pred_1h, pred_4h, pred_24h)``, each a scalar tensor of shape
``(batch, 1)``, already divided by 100 to reverse the ×100 training-target
scaling so callers receive values in raw log-return units.
"""

from __future__ import annotations

import torch
import torch.nn as nn
import torch.nn.functional as F


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
        self.padding = (kernel_size - 1) * dilation
        self.conv = nn.Conv1d(
            in_channels,
            out_channels,
            kernel_size,
            dilation=dilation,
            padding=0,
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        # x: (batch, channels, seq_len)
        x_padded = F.pad(x, (self.padding, 0))
        return self.conv(x_padded)


class MarketMarkovNet(nn.Module):
    """BTC swing-trading prediction network (Colab reference architecture).

    Parameters
    ----------
    input_features:
        Number of input feature columns in the feature window.
        Defaults to 3 (log_return_z, atr_72_z, vwap_dev_z).
    hidden_dim:
        Channel width after the first backbone layer.
    rank:
        Inner rank for the low-rank Markov heads.
    """

    def __init__(
        self,
        input_features: int = 3,
        hidden_dim: int = 64,
        rank: int = 8,
    ) -> None:
        super().__init__()

        # 6-Layer Causal CNN with GroupNorm to preserve independent sequence momentum
        self.backbone = nn.Sequential(
            CausalConv1d(input_features, hidden_dim, kernel_size=3, dilation=1),
            nn.GroupNorm(1, hidden_dim),
            nn.SiLU(),

            CausalConv1d(hidden_dim, hidden_dim, kernel_size=3, dilation=2),
            nn.GroupNorm(1, hidden_dim),
            nn.SiLU(),

            CausalConv1d(hidden_dim, hidden_dim, kernel_size=3, dilation=4),
            nn.GroupNorm(1, hidden_dim),
            nn.SiLU(),

            CausalConv1d(hidden_dim, hidden_dim, kernel_size=3, dilation=8),
            nn.GroupNorm(1, hidden_dim),
            nn.SiLU(),

            CausalConv1d(hidden_dim, hidden_dim, kernel_size=3, dilation=16),
            nn.GroupNorm(1, hidden_dim),
            nn.SiLU(),

            CausalConv1d(hidden_dim, hidden_dim, kernel_size=3, dilation=32),
            nn.GroupNorm(1, hidden_dim),
            nn.SiLU(),
        )

        # Parallel Draft Heads (1H, 4H, 24H)
        self.draft_h1 = nn.Linear(hidden_dim, 1)
        self.draft_h4 = nn.Linear(hidden_dim, 1)
        self.draft_h24 = nn.Linear(hidden_dim, 1)

        # Low-Rank Markov Heads
        self.markov_wA_1to4 = nn.Linear(1, rank, bias=False)
        self.markov_wB_1to4 = nn.Linear(rank, 1, bias=False)
        self.markov_wA_4to24 = nn.Linear(1, rank, bias=False)
        self.markov_wB_4to24 = nn.Linear(rank, 1, bias=False)

    def forward(
        self, x: torch.Tensor
    ) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
        """Run a forward pass.

        Parameters
        ----------
        x:
            Float32 tensor of shape ``(batch, seq_len, n_features)``.

        Returns
        -------
        pred_1h, pred_4h, pred_24h:
            Each ``(batch, 1)`` float32, in raw log-return units (÷100).
        """
        # Transpose to channels-first for Conv1d: (batch, n_features, seq_len)
        x = x.transpose(1, 2)

        # Backbone: (batch, hidden_dim, seq_len)
        features = self.backbone(x)

        # Take the last timestep as the summary state: (batch, hidden_dim)
        final_state = features[:, :, -1]

        # Draft predictions (in ×100 training space)
        y_raw_h1 = self.draft_h1(final_state)
        y_raw_h4 = self.draft_h4(final_state)
        y_raw_h24 = self.draft_h24(final_state)

        # Low-rank Markov refinement
        y_star_h1 = y_raw_h1
        y_star_h4 = y_raw_h4 + self.markov_wB_1to4(self.markov_wA_1to4(y_star_h1))
        y_star_h24 = y_raw_h24 + self.markov_wB_4to24(self.markov_wA_4to24(y_star_h4))

        # Reverse ×100 target scaling used during training
        scale = torch.tensor(100.0, dtype=x.dtype, device=x.device)
        pred_1h = y_star_h1 / scale
        pred_4h = y_star_h4 / scale
        pred_24h = y_star_h24 / scale

        return pred_1h, pred_4h, pred_24h


def load_model(model_path: str, **kwargs: object) -> MarketMarkovNet:
    """Load a trained ``MarketMarkovNet`` from a state-dict checkpoint.

    Parameters
    ----------
    model_path:
        Path to ``model.pt`` saved with ``torch.save(model.state_dict(), ...)``.
    **kwargs:
        Forwarded to ``MarketMarkovNet.__init__`` (e.g. ``input_features``,
        ``hidden_dim``, ``rank``) to override defaults when the checkpoint was
        trained with non-default hyperparameters.

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
