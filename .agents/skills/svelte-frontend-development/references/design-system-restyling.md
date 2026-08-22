# Design System Restyling for Svelte Dashboards

Reference for applying a `popular-web-designs` template to an existing
Svelte 4 dashboard. Covers light-to-dark adaptation, CSS variable
migration, and the verification pattern.

## When to Use

- User says the dashboard UI is "mediocre" or needs a visual upgrade.
- Restyling an existing Svelte dashboard with a known brand's design
  language (Kraken, Linear, Sentry, Vercel, etc.).
- Pairing `claude-design` (process/taste) with `popular-web-designs`
  (visual vocabulary) for a Svelte frontend.

## Workflow

1. Load the design template: `skill_view(name="popular-web-designs",
   file_path="templates/<site>.md")`
2. Load `claude-design` for the design process — commit to a surface
   archetype before touching colors. A trading dashboard is a **Monitor**
   surface: density and glanceability over decoration.
3. Define `:global(:root)` CSS custom properties in `App.svelte`
4. Migrate components one at a time, building after each
5. Run a grep-based verification script to find missed hardcoded colors

## CSS Variable Architecture

Define all design tokens as CSS custom properties in `:global(:root)`
inside `App.svelte`. Every component references these via `var(--token)`.

```css
:global(:root) {
  /* Surfaces */
  --bg-base: #0c0d12;
  --bg-surface: #15161e;
  --bg-surface-hover: #1c1d27;
  --bg-inset: #0a0b0f;

  /* Borders */
  --border: #252631;
  --border-light: #2e2f3d;

  /* Text */
  --text-primary: #ececf1;
  --text-secondary: #8b8d9a;
  --text-muted: #5c5e6e;

  /* Accent (from design system — preserve exactly) */
  --accent: #7132f5;
  --accent-dark: #5741d8;
  --accent-subtle: rgba(113, 50, 245, 0.12);

  /* Semantic */
  --green: #149e61;
  --green-subtle: rgba(20, 158, 97, 0.14);
  --red: #e5484d;
  --red-subtle: rgba(229, 72, 77, 0.14);
  --yellow: #d29922;

  /* Radii */
  --radius: 12px;
  --radius-sm: 8px;
  --radius-xs: 6px;

  /* Shadows */
  --shadow: 0 4px 24px rgba(0, 0, 0, 0.25);

  /* Fonts */
  --font: 'Inter', system-ui, -apple-system, sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, Menlo, monospace;
}
```

## Light-to-Dark Token Mapping

Many `popular-web-designs` templates are light-theme. For a dark
dashboard, invert the surface hierarchy while preserving accent and
semantic colors exactly.

| Design system (light)      | Dark adaptation             |
|----------------------------|----------------------------|
| White `#ffffff` surface    | `#15161e` surface           |
| Light gray border          | `#252631` border            |
| Near-black text `#101114`  | `#ececf1` text              |
| Secondary text gray        | `#8b8d9a`                   |
| Accent (e.g. purple)       | Keep exact hex              |
| Green/red semantic         | Keep exact hex + 12-14% opacity for subtle bg |
| 12px radius                | Keep as-is                  |
| Subtle shadow              | Darken to `rgba(0,0,0,0.25)` |

**The accent color is the brand identity.** Preserve it exactly across
light and dark. The surface hierarchy is what you invert.

## Canvas Charts: Hardcoded Hex Required

`ctx.fillStyle` and `ctx.strokeStyle` do NOT resolve CSS variables.
Canvas drawing code must use hardcoded hex values from the design system:

```javascript
// These must match the CSS variables but as raw hex
ctx.strokeStyle = '#7132f5';  // --accent
ctx.fillStyle = '#149e61';    // --green
ctx.fillStyle = '#e5484d';    // --red
ctx.strokeStyle = '#1c1d27';  // grid lines (--bg-surface-hover)
ctx.fillStyle = '#5c5e6e';    // axis labels (--text-muted)
```

Keep these in sync with the CSS variables manually. A comment next to
each hex noting which variable it corresponds to helps.

## Component Migration Order

1. **App.svelte** — global `:root` variables, body font, sidebar, shell
2. **Dashboard.svelte** — grid layout, page header
3. **StatusPanel.svelte** — most data-dense panel, sets the card pattern
4. **CandlestickChart.svelte** — canvas colors + card wrapper
5. **PnLEquityCurve.svelte** — canvas colors + card wrapper
6. **TradeHistory.svelte** — table styling
7. **ModelHealth.svelte** — status indicators
8. **FeatureInspector.svelte** — feature bars
9. **StrategyConfigPanel.svelte** — form controls
10. **Ledger.svelte** — full-page view with table + chart
11. **StrategyLab.svelte** — style block only (largest file, patch
    `<style>` section in-place rather than rewriting)

## Verification Script

After migration, run a script that greps for old hardcoded colors to
find components that were missed:

```bash
#!/bin/bash
FRONTEND="frontend/src"
OLD_COLORS=$(grep -rn '#161b22\|#0d1117\|#30363d\|#58a6ff\|#8b949e\|#c9d1d9\|#3fb950\|#f85149' \
  "$FRONTEND" --include="*.svelte" | grep -v node_modules | wc -l)
if [ "$OLD_COLORS" -gt 0 ]; then
  echo "WARN: $OLD_COLORS lines still use old hardcoded colors"
  grep -rn '#161b22\|#0d1117\|#30363d\|#58a6ff\|#8b949e\|#c9d1d9\|#3fb950\|#f85149' \
    "$FRONTEND" --include="*.svelte" | head -10
else
  echo "OK: no old hardcoded colors"
fi
```

**Do NOT filter out canvas hex matches.** Canvas drawing code
(`ctx.fillStyle`, `ctx.strokeStyle`) uses hardcoded hex that must be
updated to the new design system's palette — the grep correctly flags
these as things to fix. A match inside a `<script>` block (canvas draw
function) is just as actionable as one in a `<style>` block.

## Font Loading

Load the design system's font via `<svelte:head>` in `App.svelte`:

```svelte
<svelte:head>
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
  <link
    href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700&display=swap"
    rel="stylesheet"
  />
</svelte:head>
```

Most `popular-web-designs` templates use proprietary fonts. Each
template's Hermes Implementation Notes block specifies the Google Fonts
substitute. Common mapping: Inter for most sans-serif UIs.

## Deploy Verification

After rebuilding the Docker container and restarting, verify the live
container is actually serving the new bundle:

```bash
# Container is healthy
docker ps --filter name=mmn-engine --format '{{.Names}} {{.Status}}'
# → mmn-engine Up X seconds (healthy)

# Container is serving the new JS bundle (hash matches dist/)
CONTAINER_IP=$(docker inspect mmn-engine --format \
  '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}')
curl -s "http://$CONTAINER_IP:8080/" | \
  grep -o 'index-[a-zA-Z0-9]*\.js' | head -1
# → index-qe4Kb4Gp.js (should match the file in frontend/dist/assets/)
```

The bundle hash in the served HTML must match the hash in
`frontend/dist/assets/`. If it doesn't, the Docker image wasn't
rebuilt after the frontend changed — rebuild with:
`docker build -f engine/Dockerfile -t <image> .`

## Pitfalls

1. **Forgetting sub-components.** Large views like StrategyLab import
   sub-components (ParamSlider, RhaiEditor, MetricsTable, ABComparison,
   EquityCurveChart) that have their own `<style>` blocks with old
   hardcoded colors. The main view's style patch won't catch them. Run
   the grep verification to find them.

2. **`font: '10px var(--font-mono)'` in canvas.** CSS variables don't
   work in canvas `ctx.font` strings either. Use the raw font family:
   `ctx.font = '10px monospace'` or `ctx.font = '10px ui-monospace, monospace'`.

3. **Global scrollbar styling.** Add `:global(::-webkit-scrollbar)` rules
   in `App.svelte` to match the dark theme — default browser scrollbars
   clash with a dark dashboard.

4. **Canvas `ctx.font` with CSS variables.** Writing
   `ctx.font = '10px var(--font-mono)'` silently fails — the browser
   cannot parse the CSS variable in the canvas font string, and the
   font falls back to default. Use the raw family name:
   `ctx.font = '10px monospace'` or
   `ctx.font = '12px Inter, sans-serif'`.

5. **StrategyLab sub-components missed by style-only patch.** When
   patching StrategyLab.svelte's `<style>` block, the sub-components it
   imports (ParamSlider, RhaiEditor, MetricsTable, ABComparison,
   EquityCurveChart) each have their own `<style>` with old colors.
   The main view's style patch won't catch them. Run the grep
   verification across all `.svelte` files to find them.
