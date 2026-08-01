<script>
  export let value = `// Default Threshold Strategy (Rhai)
// Entry: when signal > entry_threshold
// Exit: when signal < exit_threshold

let entry_threshold = 0.004;
let exit_threshold = -0.002;
let sma_window = 150;

fn should_enter(ctx) {
    ctx.signal > entry_threshold
}

fn should_exit(ctx) {
    ctx.signal < exit_threshold
}
`;

  let lineCount = 0;

  $: {
    lineCount = value.split('\n').length;
  }

  function handleInput(e) {
    value = e.target.value;
  }

  function handleKeydown(e) {
    if (e.key === 'Tab') {
      e.preventDefault();
      const start = e.target.selectionStart;
      const end = e.target.selectionEnd;
      value = value.substring(0, start) + '    ' + value.substring(end);
      // Move cursor after the tab
      requestAnimationFrame(() => {
        e.target.selectionStart = e.target.selectionEnd = start + 4;
      });
    }
  }
</script>

<div class="rhai-editor">
  <div class="editor-header">
    <span class="editor-label">Rhai Script</span>
    <span class="editor-hint">Tab inserts 4 spaces</span>
  </div>
  <div class="editor-body">
    <div class="line-numbers">
      {#each Array(lineCount) as _, i}
        <span class="line-num">{i + 1}</span>
      {/each}
    </div>
    <textarea
      class="editor-textarea"
      bind:value
      on:input={handleInput}
      on:keydown={handleKeydown}
      spellcheck="false"
      autocomplete="off"
      autocapitalize="off"
    ></textarea>
  </div>
</div>

<style>
  .rhai-editor {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .editor-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.5rem 0.75rem;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
  }

  .editor-label {
    font-size: 0.68rem;
    color: var(--text-secondary);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .editor-hint {
    font-size: 0.7rem;
    color: var(--text-muted);
  }

  .editor-body {
    display: flex;
    min-height: 220px;
    max-height: 400px;
  }

  .line-numbers {
    flex-shrink: 0;
    padding: 0.75rem 0;
    background: var(--bg-inset);
    border-right: 1px solid var(--border);
    text-align: right;
    user-select: none;
    overflow: hidden;
  }

  .line-num {
    display: block;
    padding: 0 0.75rem;
    font-family: var(--font-mono);
    font-size: 0.8rem;
    line-height: 1.6;
    color: var(--text-muted);
  }

  .editor-textarea {
    flex: 1;
    padding: 0.75rem;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 0.8rem;
    line-height: 1.6;
    resize: vertical;
    outline: none;
    tab-size: 4;
    white-space: pre;
    overflow: auto;
  }

  .editor-textarea::placeholder {
    color: var(--text-muted);
  }
</style>