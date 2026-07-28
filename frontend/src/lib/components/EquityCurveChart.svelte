<script>
  import { onMount, onDestroy } from 'svelte';

  export let equityCurve = [];
  export let benchmarkCurve = null;
  export let label = 'Equity Curve';
  export let benchmarkLabel = 'Buy & Hold';

  let canvas;
  let container;
  let resizeObserver;

  onMount(() => {
    draw();
    resizeObserver = new ResizeObserver(() => draw());
    if (container) resizeObserver.observe(container);
  });

  onDestroy(() => {
    if (resizeObserver) resizeObserver.disconnect();
  });

  $: {
    // Redraw when data changes
    if (canvas) draw();
  }

  function draw() {
    if (!canvas || !container) return;
    const ctx = canvas.getContext('2d');
    const dpr = window.devicePixelRatio || 1;
    const w = container.clientWidth;
    const h = container.clientHeight || 250;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = w + 'px';
    canvas.style.height = h + 'px';
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, w, h);

    const padL = 55, padR = 15, padT = 15, padB = 35;
    const cw = w - padL - padR;
    const ch = h - padT - padB;

    if (equityCurve.length < 2) {
      ctx.fillStyle = '#8b949e';
      ctx.font = '12px sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText('No equity curve data', w / 2, h / 2);
      return;
    }

    // Collect all data points for range calculation
    const allValues = equityCurve.map(d => d[1]);
    if (benchmarkCurve && benchmarkCurve.length > 1) {
      allValues.push(...benchmarkCurve.map(d => d[1]));
    }

    let minV = Math.min(...allValues);
    let maxV = Math.max(...allValues);
    const range = maxV - minV || 1;

    const yScale = (v) => padT + ch - ((v - minV) / range) * ch;

    // Grid lines
    ctx.font = '10px monospace';
    ctx.fillStyle = '#484f58';
    ctx.textAlign = 'right';
    for (let i = 0; i <= 4; i++) {
      const y = padT + (ch / 4) * i;
      const val = maxV - (range / 4) * i;
      ctx.strokeStyle = '#21262d';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(w - padR, y);
      ctx.stroke();
      ctx.fillText(val.toFixed(0), padL - 6, y + 3);
    }

    function drawLine(data, strokeColor, fillColor, lineWidth = 2) {
      const n = data.length;
      const xStep = cw / (n - 1);

      // Fill area
      ctx.beginPath();
      for (let i = 0; i < n; i++) {
        const x = padL + i * xStep;
        const y = yScale(data[i][1]);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      const lastX = padL + (n - 1) * xStep;
      const bottomY = padT + ch;
      ctx.lineTo(lastX, bottomY);
      ctx.lineTo(padL, bottomY);
      ctx.closePath();
      ctx.fillStyle = fillColor;
      ctx.fill();

      // Line stroke
      ctx.beginPath();
      for (let i = 0; i < n; i++) {
        const x = padL + i * xStep;
        const y = yScale(data[i][1]);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.strokeStyle = strokeColor;
      ctx.lineWidth = lineWidth;
      ctx.stroke();
    }

    // Draw benchmark first (behind)
    if (benchmarkCurve && benchmarkCurve.length > 1) {
      drawLine(benchmarkCurve, '#8b949e', '#8b949e11', 1.5);
    }

    // Draw strategy equity curve
    const lastVal = equityCurve[equityCurve.length - 1][1];
    const firstVal = equityCurve[0][1];
    const color = lastVal >= firstVal ? '#3fb950' : '#f85149';
    const fillColor = lastVal >= firstVal ? '#3fb95015' : '#f8514915';
    drawLine(equityCurve, color, fillColor, 2.5);

    // Legend
    ctx.textAlign = 'left';
    const legendY = padT + 2;
    const legendX = padL + 10;

    // Strategy legend
    ctx.fillStyle = color;
    ctx.fillRect(legendX, legendY, 12, 12);
    ctx.fillStyle = '#c9d1d9';
    ctx.font = '11px sans-serif';
    ctx.fillText(label, legendX + 18, legendY + 10);

    // Benchmark legend
    if (benchmarkCurve && benchmarkCurve.length > 1) {
      ctx.fillStyle = '#8b949e';
      ctx.fillRect(legendX + 120, legendY, 12, 12);
      ctx.fillStyle = '#c9d1d9';
      ctx.fillText(benchmarkLabel, legendX + 138, legendY + 10);
    }
  }
</script>

<div class="chart-container" bind:this={container}>
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .chart-container {
    width: 100%;
    height: 250px;
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 8px;
    overflow: hidden;
  }
  canvas {
    display: block;
  }
</style>