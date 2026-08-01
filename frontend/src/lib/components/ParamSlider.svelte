<script>
  export let label = '';
  export let min = 0;
  export let max = 1;
  export let step = 0.001;
  export let value = 0;
  export let unit = '';
  export let dp = 3;

  function handleInput(e) {
    value = parseFloat(e.target.value);
  }

  $: pct = ((value - min) / (max - min)) * 100;
</script>

<div class="param-slider">
  <div class="slider-header">
    <span class="slider-label">{label}</span>
    <span class="slider-value">{value.toFixed(dp)}{unit}</span>
  </div>
  <div class="slider-row">
    <span class="slider-min">{min.toFixed(dp)}</span>
    <input
      type="range"
      {min}
      {max}
      {step}
      bind:value
      on:input={handleInput}
      style="--pct: {pct}%"
    />
    <span class="slider-max">{max.toFixed(dp)}</span>
  </div>
</div>

<style>
  .param-slider {
    margin-bottom: 0.75rem;
  }

  .slider-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.35rem;
  }

  .slider-label {
    font-size: 0.8rem;
    color: var(--text-secondary);
    font-weight: 500;
  }

  .slider-value {
    font-size: 0.82rem;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
    font-family: var(--font-mono);
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    padding: 0.15rem 0.5rem;
    min-width: 70px;
    text-align: right;
  }

  .slider-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .slider-min,
  .slider-max {
    font-size: 0.7rem;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    font-family: var(--font-mono);
    min-width: 40px;
  }

  .slider-max {
    text-align: right;
  }

  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    flex: 1;
    height: 6px;
    background: var(--bg-inset);
    border-radius: 3px;
    outline: none;
    cursor: pointer;
  }

  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 16px;
    height: 16px;
    background: var(--accent);
    border-radius: 50%;
    cursor: pointer;
    border: 2px solid var(--bg-base);
    box-shadow: 0 0 4px var(--accent-glow);
  }

  input[type="range"]::-moz-range-thumb {
    width: 16px;
    height: 16px;
    background: var(--accent);
    border-radius: 50%;
    cursor: pointer;
    border: 2px solid var(--bg-base);
    box-shadow: 0 0 4px var(--accent-glow);
  }

  input[type="range"]::-webkit-slider-runnable-track {
    height: 6px;
    border-radius: 3px;
    background: linear-gradient(
      to right,
      var(--accent) 0%,
      var(--accent) var(--pct),
      var(--bg-inset) var(--pct),
      var(--bg-inset) 100%
    );
  }
</style>