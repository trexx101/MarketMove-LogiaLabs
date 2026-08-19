<script>
  import { onMount } from 'svelte';

  const API_BASE = '/api';

  let briefing = null;
  let loading = true;
  let error = null;
  let chatQuestion = '';
  let chatResponse = '';
  let chatLoading = false;
  let chatError = null;

  onMount(async () => {
    await fetchBriefing();
  });

  async function fetchBriefing() {
    loading = true;
    error = null;
    try {
      const res = await fetch(`${API_BASE}/advisor/briefing`);
      const data = await res.json();
      if (data.enabled && data.briefing) {
        briefing = data.briefing;
      } else if (!data.enabled) {
        error = data.reason || 'Advisor is disabled';
      } else {
        error = 'Briefing not yet generated for today';
      }
    } catch (e) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  async function sendQuestion() {
    if (!chatQuestion.trim() || chatLoading) return;
    chatLoading = true;
    chatError = null;
    chatResponse = '';
    try {
      const res = await fetch(`${API_BASE}/advisor/ask`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ question: chatQuestion }),
      });
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop();
        for (const line of lines) {
          if (!line.startsWith('data: ')) continue;
          try {
            const data = JSON.parse(line.slice(6));
            if (data.token) chatResponse += data.token;
            if (data.error) chatError = data.error;
          } catch (e) { /* skip malformed */ }
        }
      }
    } catch (e) {
      chatError = e.message;
    } finally {
      chatLoading = false;
    }
  }

  function fmtDate(d) {
    if (!d) return '';
    return d;
  }
</script>

<div class="advisor">
  <div class="header">
    <h1>AI Advisor</h1>
    <span class="subtitle">Daily briefing — powered by market data, features, and sentiment</span>
  </div>

  {#if loading}
    <div class="state">Loading briefing...</div>
  {:else if error}
    <div class="state error">{error}</div>
    <button class="retry-btn" on:click={fetchBriefing}>Retry</button>
  {:else if briefing}
    <div class="briefing-card">
      <div class="briefing-meta">
        <span class="date">{fmtDate(briefing.for_date)}</span>
        <span class="model">{briefing.model_used}</span>
        <span class="status status-{briefing.parse_status}">{briefing.parse_status}</span>
      </div>

      {#if briefing.warnings.length > 0}
        <div class="warnings">
          {#each briefing.warnings as w}
            <span class="warn-chip">{w}</span>
          {/each}
        </div>
      {/if}

      <div class="digest">
        {@html briefing.digest.replace(/\n/g, '<br>')}
      </div>

      {#if briefing.suggested_action}
        <div class="action">
          <span class="label">Suggested action:</span>
          <span class="value">{briefing.suggested_action}</span>
          {#if briefing.suggested_confidence}
            <span class="confidence">({(briefing.suggested_confidence * 100).toFixed(0)}%)</span>
          {/if}
        </div>
      {/if}
    </div>
  {:else}
    <div class="state">No briefing available</div>
  {/if}

  <div class="chat-section">
    <h2>Ask a follow-up</h2>
    <div class="chat-input-row">
      <input
        type="text"
        bind:value={chatQuestion}
        placeholder="Why did the model exit the long position?"
        on:keydown={(e) => e.key === 'Enter' && sendQuestion()}
        disabled={chatLoading}
      />
      <button on:click={sendQuestion} disabled={chatLoading || !chatQuestion.trim()}>
        {chatLoading ? '...' : 'Send'}
      </button>
    </div>
    {#if chatError}
      <div class="chat-error">{chatError}</div>
    {/if}
    {#if chatResponse}
      <div class="chat-response">{chatResponse}</div>
    {/if}
  </div>
</div>

<style>
  .advisor {
    padding: 1.5rem;
    max-width: 900px;
  }

  .header {
    margin-bottom: 1.5rem;
  }

  .header h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .subtitle {
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  .state {
    padding: 2rem;
    text-align: center;
    color: var(--text-secondary);
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .state.error {
    color: var(--red);
    border-color: var(--red);
  }

  .retry-btn {
    margin-top: 0.75rem;
    padding: 0.5rem 1rem;
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: var(--radius-xs);
    cursor: pointer;
    font-family: inherit;
    font-size: 0.85rem;
  }

  .briefing-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1.25rem;
    margin-bottom: 1.5rem;
  }

  .briefing-meta {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    margin-bottom: 1rem;
    font-size: 0.75rem;
    color: var(--text-secondary);
  }

  .date {
    font-weight: 600;
    color: var(--text-primary);
  }

  .status {
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-xs);
    font-weight: 600;
    text-transform: uppercase;
    font-size: 0.65rem;
  }

  .status-ok {
    background: var(--green-subtle);
    color: var(--green);
  }

  .status-failed {
    background: var(--red-subtle);
    color: var(--red);
  }

  .status-partial {
    background: var(--yellow-subtle);
    color: var(--yellow);
  }

  .warnings {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-bottom: 1rem;
  }

  .warn-chip {
    padding: 0.2rem 0.6rem;
    background: var(--yellow-subtle);
    color: var(--yellow);
    border-radius: var(--radius-xs);
    font-size: 0.72rem;
    font-weight: 500;
  }

  .digest {
    font-size: 0.88rem;
    line-height: 1.6;
    color: var(--text-primary);
    white-space: pre-line;
  }

  .action {
    margin-top: 1rem;
    padding: 0.75rem 1rem;
    background: var(--accent-subtle);
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.85rem;
  }

  .action .label {
    color: var(--text-secondary);
  }

  .action .value {
    color: var(--accent);
    font-weight: 600;
  }

  .action .confidence {
    color: var(--text-secondary);
    font-size: 0.8rem;
  }

  .chat-section {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1.25rem;
  }

  .chat-section h2 {
    font-size: 0.95rem;
    font-weight: 600;
    margin-bottom: 0.75rem;
    color: var(--text-primary);
  }

  .chat-input-row {
    display: flex;
    gap: 0.5rem;
  }

  .chat-input-row input {
    flex: 1;
    padding: 0.6rem 0.8rem;
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    color: var(--text-primary);
    font-family: inherit;
    font-size: 0.85rem;
  }

  .chat-input-row input:disabled {
    opacity: 0.5;
  }

  .chat-input-row button {
    padding: 0.6rem 1.2rem;
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: var(--radius-xs);
    cursor: pointer;
    font-family: inherit;
    font-size: 0.85rem;
    font-weight: 500;
  }

  .chat-input-row button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .chat-error {
    margin-top: 0.5rem;
    color: var(--red);
    font-size: 0.8rem;
  }

  .chat-response {
    margin-top: 0.75rem;
    padding: 0.75rem;
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    font-size: 0.85rem;
    line-height: 1.5;
    color: var(--text-primary);
    white-space: pre-line;
  }
</style>