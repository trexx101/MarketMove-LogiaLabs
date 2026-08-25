# DESIGN.md — MarketMoves UI Design System

## Philosophy

The MarketMoves dashboard is a **Monitor** surface: density and glanceability
over decoration. Every pixel serves the operator's decision loop. The visual
language is derived from Kraken's dark trading UI, adapted via the
`popular-web-designs` template.

## Palette

All design tokens are defined as CSS custom properties in `App.svelte`
`:global(:root)`. Components reference these exclusively via `var(--token)`.

### Surfaces

| Token | Hex | Role |
|---|---|---|
| `--bg-base` | `#0c0d12` | Page background |
| `--bg-surface` | `#15161e` | Card / panel background |
| `--bg-surface-hover` | `#1c1d27` | Hover state for interactive surfaces |
| `--bg-inset` | `#0a0b0f` | Inset / input background |

### Borders

| Token | Hex | Role |
|---|---|---|
| `--border` | `#252631` | Default border |
| `--border-light` | `#2e2f3d` | Subtle border (scrollbars, dividers) |

### Text

| Token | Hex | Role |
|---|---|---|
| `--text-primary` | `#ececf1` | Headings, key data |
| `--text-secondary` | `#8b8d9a` | Secondary labels, metadata |
| `--text-muted` | `#5c5e6e` | Disabled/placeholder text |

### Accent (Purple)

| Token | Hex / RGBA | Role |
|---|---|---|
| `--accent` | `#7132f5` | Primary action, active state |
| `--accent-dark` | `#5741d8` | Pressed / darker variant |
| `--accent-subtle` | `rgba(113, 50, 245, 0.12)` | Subtle accent background |
| `--accent-glow` | `rgba(113, 50, 245, 0.25)` | Glow / focus ring |

### Semantic Colors

| Token | Hex / RGBA | Role |
|---|---|---|
| `--green` | `#149e61` | Profit, buy, long, success |
| `--green-subtle` | `rgba(20, 158, 97, 0.14)` | Subtle green background |
| `--red` | `#e5484d` | Loss, sell, short, error |
| `--red-subtle` | `rgba(229, 72, 77, 0.14)` | Subtle red background |
| `--yellow` | `#d29922` | Warning, neutral/uncertain |
| `--yellow-subtle` | `rgba(210, 153, 34, 0.14)` | Subtle yellow background |

## Typography

| Token | Value |
|---|---|
| `--font` | `'Inter', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif` |
| `--font-mono` | `ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', monospace` |

Body: `14px`, `line-height: 1.4`, `-webkit-font-smoothing: antialiased`.

## Spacing & Radius

| Token | Value | Role |
|---|---|---|
| `--radius` | `12px` | Cards, panels |
| `--radius-sm` | `8px` | Buttons, inputs |
| `--radius-xs` | `6px` | Small elements (badges, tags) |

## Shadows

| Token | Value |
|---|---|
| `--shadow` | `0 4px 24px rgba(0, 0, 0, 0.25)` |
| `--shadow-sm` | `0 1px 4px rgba(0, 0, 0, 0.2)` |

## Layout

The shell uses a flexbox column layout with a top app bar and a scrollable
content area. The grid is a responsive CSS Grid (`grid-template-columns:
repeat(auto-fill, minmax(380px, 1fr))`) with `gap: 1rem` for cards.

## Component Patterns

### Cards
- Background: `var(--bg-surface)`
- Border: `1px solid var(--border)`
- Border-radius: `var(--radius)`
- Box-shadow: `var(--shadow-sm)`
- Hover: `background: var(--bg-surface-hover)`

### Buttons
- Background: `var(--bg-surface)`
- Border: `1px solid var(--border)`
- Border-radius: `var(--radius-sm)`
- Primary variant: `color: #fff; background: var(--accent); border-color: var(--accent)`
- Hover: `background: var(--bg-surface-hover)`
- Disabled: `opacity: 0.5; cursor: not-allowed`

### Status Colors (P&L, Position)
- **Positive / Long / Profit:** `var(--green)`
- **Negative / Short / Loss:** `var(--red)`
- **Neutral / Flat:** `var(--text-secondary)`
- **Warning:** `var(--yellow)`

### Candlestick Chart
- Bull candle: `var(--green)`
- Bear candle: `var(--red)`
- Volume: `var(--text-muted)` at 40% opacity
- Grid lines: `var(--border)` at 50% opacity
- Crosshair: `var(--text-secondary)` at 60% opacity

### Tables
- Header: `var(--text-muted)`, uppercase, `font-size: 11px`
- Rows: alternating `var(--bg-base)` / `var(--bg-surface)`
- Hover: `var(--bg-surface-hover)`

## Scrollbar

```css
::-webkit-scrollbar { width: 6px; height: 6px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: var(--border-light); border-radius: 3px; }
::-webkit-scrollbar-thumb:hover { background: var(--text-muted); }
```

## Color Usage Rules

1. Never use raw hex values in component `<style>` blocks. Always reference
   `var(--token)`.
2. Green/red are **only** for P&L direction and trade side. Do not use them
   for UI chrome, labels, or decorative elements.
3. The accent purple is reserved for the primary action in each context
   (mode toggle, save button, active nav item). One purple element per view.
4. Text hierarchy: `--text-primary` for data values and headings,
   `--text-secondary` for labels and metadata, `--text-muted` only for
   placeholders and truly disabled text.

## Verification

After any UI change, run the grep-based color audit to catch hardcoded hex
values:

```bash
grep -rn '#[0-9a-fA-F]\{3,6\}' frontend/src/ \
  --include='*.svelte' --include='*.css' \
  | grep -v 'var(--' \
  | grep -v 'App.svelte'
```

Any match outside `App.svelte`'s `:global(:root)` block is a violation.