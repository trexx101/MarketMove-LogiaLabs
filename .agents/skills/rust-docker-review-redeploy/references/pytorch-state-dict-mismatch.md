# PyTorch state_dict Key Mismatch: nn.Module Wrapper vs nn.Conv1d Subclass

**Problem:** The same PyTorch `CausalConv1d` architecture can produce two
different state_dict key shapes depending on how it's implemented:

| Implementation | state_dict keys | Example |
|---|---|---|
| `nn.Module` + `self.conv = nn.Conv1d(...)` | Nested under `conv.` | `blocks.0.conv1.conv.weight` |
| `class CausalConv1d(nn.Conv1d)` (subclass) | Top-level | `blocks.0.conv1.weight` |

Both forward functions can be identical (causal left-pad, no right pad),
but `load_state_dict(strict=True)` sees 28 missing + 28 unexpected keys.

## Diagnosis

```python
import torch
state = torch.load("model.pt", map_location="cpu", weights_only=True)
print("Keys with 'conv.conv':", [k for k in state if "conv.conv" in k][:2])
print("Keys with 'conv.weight' (no .conv.):", [k for k in state if "conv.weight" in k and "conv.conv" not in k][:2])
```

If the artifact has `conv.conv.weight` keys, the serving code must use the
wrapper pattern. If it has top-level `conv.weight`, it must use the subclass.

## Fix (wrapper pattern — matches Colab notebook)

```python
# WRONG: nn.Conv1d subclass → keys like blocks.0.conv1.weight
class CausalConv1d(nn.Conv1d):
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
        self._causal_padding = (kernel_size - 1) * dilation
    def forward(self, x):
        return super().forward(F.pad(x, (self._causal_padding, 0)))

# RIGHT: nn.Module wrapping nn.Conv1d as self.conv → keys like blocks.0.conv1.conv.weight
class CausalConv1d(nn.Module):
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__()
        self.conv = nn.Conv1d(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
        self._causal_padding = (kernel_size - 1) * dilation
    def forward(self, x):
        return self.conv(F.pad(x, (self._causal_padding, 0)))
```

## Verification

```python
import torch
import torch.nn as nn
import torch.nn.functional as F

class CausalConv1d(nn.Module):
    def __init__(self, in_ch, out_ch, kernel_size, dilation):
        super().__init__()
        self.conv = nn.Conv1d(in_ch, out_ch, kernel_size, dilation=dilation, padding=0)
        self._causal_padding = (kernel_size - 1) * dilation
    def forward(self, x):
        return self.conv(F.pad(x, (self._causal_padding, 0)))

conv = CausalConv1d(in_ch=8, out_ch=16, kernel_size=3, dilation=1)

# 1. Check key shape
keys = [n for n, _ in conv.named_parameters()]
print("Keys:", keys)
# Expected: ['conv.weight', 'conv.bias']

# 2. Check causal padding (no future leakage)
x = torch.randn(2, 8, 126)
out = conv(x)
assert list(out.shape) == [2, 16, 126], "sequence length changed"
assert torch.isfinite(out).all(), "NaN/Inf output"

# 3. Causal property: output[:123] unaffected by input[-3:] zeroed
x_t = x.clone(); x_t[:,:,-3:] = 0.0
causal_diff = (out - conv(x_t)[:,:,:126])[:,:,:123].abs().max().item()
tail_diff   = (out - conv(x_t)[:,:,:126])[:,:,123:].abs().max().item()
assert causal_diff < 1e-6, f"early positions affected: {causal_diff}"
assert tail_diff   > 1e-4, f"tail unchanged (broken conv?): {tail_diff}"

# 4. Full model strict load
model = QqqTCN(in_dim=8, hidden_dim=64, dropout=0.0)
missing, unexpected = model.load_state_dict(state, strict=True)
assert len(missing)==0 and len(unexpected)==0, f"mismatch: {missing}, {unexpected}"
```

## Known Occurrence

MarketMoves QQQ Equities TCN (2026-07-27):
- Colab notebook uses `nn.Module` wrapper: `self.conv = nn.Conv1d(...)`
- Local inference was written as `nn.Conv1d` subclass (wrong)
- Fix: changed to `nn.Module` + `self.conv = nn.Conv1d(...)` wrapper
- Artifact: `models/qqq_tcn_v1.pt` (rebuilt with causal padding, Jul 27)
