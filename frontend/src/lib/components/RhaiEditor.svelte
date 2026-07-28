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
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 8px;
    overflow: hidden;
  }

  .editor-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.5rem 0.75rem;
    background: #161b22;
    border-bottom: 1px solid #30363d;
  }

  .editor-label {
    font-size: 0.8rem;
    color: #8b949e;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .editor-hint {
    font-size: 0.7rem;
    color: #484f58;
  }

  .editor-body {
    display: flex;
    min-height: 220px;
    max-height: 400px;
  }

  .line-numbers {
    flex-shrink: 0;
    padding: 0.75rem 0;
    background: #0d1117;
    border-right: 1px solid #21262d;
    text-align: right;
    user-select: none;
    overflow: hidden;
  }

  .line-num {
    display: block;
    padding: 0 0.75rem;
    font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
    font-size: 0.8rem;
    line-height: 1.6;
    color: #484f58;
  }

  .editor-textarea {
    flex: 1;
    padding: 0.75rem;
    background: transparent;
    border: none;
    color: #c9d1d9;
    font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
    font-size: 0.8rem;
    line-height: 1.6;
    resize: vertical;
    outline: none;
    tab-size: 4;
    white-space: pre;
    overflow: auto;
  }

  .editor-textarea::placeholder {
    color: #484f58;
  }

  .editor-textarea::-webkit-scrollbar {
    width: 8px;
    height: 8px;
  }

  .editor-textarea::-webkit-scrollbar-track {
    background: #0d1117;
  }

  .editor-textarea::-webkit-scrollbar-thumb {
    background: #30363d;
    border-radius: 4px;
  }
</style>