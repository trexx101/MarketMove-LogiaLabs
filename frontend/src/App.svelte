<script>
  import Dashboard from './views/Dashboard.svelte';
  import Ledger from './views/Ledger.svelte';
  import StrategyLab from './views/StrategyLab.svelte';
  import Advisor from './views/Advisor.svelte';
  import { status } from './lib/stores.js';

  let currentView = 'dashboard';
  let sidebarOpen = false;

  $: mode = $status?.mode || 'PAPER';

  function nav(view) {
    currentView = view;
    sidebarOpen = false;
  }
</script>

<svelte:head>
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
  <link
    href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700&display=swap"
    rel="stylesheet"
  />
</svelte:head>

<div class="app-shell" class:sidebar-open={sidebarOpen}>
  <button class="menu-toggle" on:click={() => sidebarOpen = !sidebarOpen} aria-label="Toggle menu">
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
      <path d="M2 4h12M2 8h12M2 12h12" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
    </svg>
  </button>

  <nav class="sidebar" class:open={sidebarOpen}>
    <div class="logo">
      <span class="logo-mark">M</span>
      <span class="logo-text">MM - By LogiaLabs</span>
    </div>
    <ul>
      <li>
        <button class="nav-btn" class:active={currentView === 'dashboard'} on:click={() => nav('dashboard')}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <rect x="1.5" y="1.5" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1.3"/>
            <rect x="9.5" y="1.5" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1.3"/>
            <rect x="1.5" y="9.5" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1.3"/>
            <rect x="9.5" y="9.5" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1.3"/>
          </svg>
          Dashboard
        </button>
      </li>
      <li>
        <button class="nav-btn" class:active={currentView === 'strategy'} on:click={() => nav('strategy')}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path d="M2 14V3M2 14h12M5 11V7M8 11V5M11 11V8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
          </svg>
          Strategy Lab
        </button>
      </li>
      <li>
        <button class="nav-btn" class:active={currentView === 'ledger'} on:click={() => nav('ledger')}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <path d="M3 2h8l2 2v10H3V2z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/>
            <path d="M5 6h6M5 9h6M5 12h4" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
          </svg>
          Ledger
        </button>
      </li>
      <li>
        <button class="nav-btn" class:active={currentView === 'advisor'} on:click={() => nav('advisor')}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
            <circle cx="8" cy="6" r="3" stroke="currentColor" stroke-width="1.3"/>
            <path d="M2.5 14c0-3 2.5-5 5.5-5s5.5 2 5.5 5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
          </svg>
          Advisor
        </button>
      </li>
    </ul>
    <div class="sidebar-footer">
      <div class="mode-badge mode-{mode.toLowerCase()}">
        <span class="mode-dot"></span>
        {mode}
      </div>
    </div>
  </nav>

  {#if sidebarOpen}
    <div class="overlay" role="button" tabindex="0"
         on:click={() => sidebarOpen = false} on:keydown={(e) => e.key === 'Enter' && (sidebarOpen = false)}></div>
  {/if}

  <main class="content">
    {#if currentView === 'dashboard'}
      <Dashboard />
    {:else if currentView === 'strategy'}
      <StrategyLab />
    {:else if currentView === 'ledger'}
      <Ledger />
    {:else if currentView === 'advisor'}
      <Advisor />
    {/if}
  </main>
</div>

<style>
  :global(*) {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
  }

  :global(:root) {
    /* Kraken-derived dark trading dashboard palette */
    --bg-base: #0c0d12;
    --bg-surface: #15161e;
    --bg-surface-hover: #1c1d27;
    --bg-inset: #0a0b0f;
    --border: #252631;
    --border-light: #2e2f3d;
    --text-primary: #ececf1;
    --text-secondary: #8b8d9a;
    --text-muted: #5c5e6e;
    --accent: #7132f5;
    --accent-dark: #5741d8;
    --accent-subtle: rgba(113, 50, 245, 0.12);
    --accent-glow: rgba(113, 50, 245, 0.25);
    --green: #149e61;
    --green-subtle: rgba(20, 158, 97, 0.14);
    --red: #e5484d;
    --red-subtle: rgba(229, 72, 77, 0.14);
    --yellow: #d29922;
    --yellow-subtle: rgba(210, 153, 34, 0.14);
    --radius: 12px;
    --radius-sm: 8px;
    --radius-xs: 6px;
    --shadow: 0 4px 24px rgba(0, 0, 0, 0.25);
    --shadow-sm: 0 1px 4px rgba(0, 0, 0, 0.2);
    --font: 'Inter', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif;
    --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', monospace;
  }

  :global(body) {
    font-family: var(--font);
    background: var(--bg-base);
    color: var(--text-primary);
    font-size: 14px;
    line-height: 1.4;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }

  :global(::-webkit-scrollbar) {
    width: 6px;
    height: 6px;
  }
  :global(::-webkit-scrollbar-track) {
    background: transparent;
  }
  :global(::-webkit-scrollbar-thumb) {
    background: var(--border-light);
    border-radius: 3px;
  }
  :global(::-webkit-scrollbar-thumb:hover) {
    background: var(--text-muted);
  }

  .app-shell {
    display: flex;
    min-height: 100vh;
  }

  .sidebar {
    width: 220px;
    background: var(--bg-surface);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    padding: 0;
    flex-shrink: 0;
    z-index: 100;
  }

  .logo {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 1.1rem 1.2rem;
    border-bottom: 1px solid var(--border);
  }

  .logo-mark {
    width: 28px;
    height: 28px;
    background: var(--accent);
    color: #fff;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 0.95rem;
    flex-shrink: 0;
  }

  .logo-text {
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .sidebar ul {
    list-style: none;
    flex: 1;
    padding: 0.5rem 0;
  }

  .sidebar li {
    list-style: none;
  }

  .nav-btn {
    width: 100%;
    text-align: left;
    padding: 0.65rem 1.2rem;
    cursor: pointer;
    color: var(--text-secondary);
    transition: all 0.15s;
    display: flex;
    align-items: center;
    gap: 0.6rem;
    background: none;
    border: none;
    border-left: 3px solid transparent;
    font-size: 0.875rem;
    font-weight: 500;
    font-family: inherit;
  }

  .nav-btn:hover {
    background: var(--bg-surface-hover);
    color: var(--text-primary);
  }

  .nav-btn.active {
    background: var(--accent-subtle);
    color: var(--accent);
    border-left-color: var(--accent);
  }

  .nav-btn svg {
    flex-shrink: 0;
    opacity: 0.8;
  }

  .sidebar-footer {
    padding: 0.8rem 1.2rem;
    border-top: 1px solid var(--border);
  }

  .mode-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.7rem;
    border-radius: var(--radius-xs);
    font-size: 0.7rem;
    font-weight: 600;
    letter-spacing: 0.03em;
  }

  .mode-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .mode-paper {
    background: var(--accent-subtle);
    color: var(--accent);
  }
  .mode-paper .mode-dot {
    background: var(--accent);
  }

  .mode-live {
    background: var(--red-subtle);
    color: var(--red);
  }
  .mode-live .mode-dot {
    background: var(--red);
    box-shadow: 0 0 6px var(--red);
  }

  .content {
    flex: 1;
    min-width: 0;
    overflow-x: hidden;
  }

  .placeholder {
    padding: 3rem 2rem;
    text-align: center;
  }

  .placeholder h1 {
    color: var(--text-primary);
    margin-bottom: 0.5rem;
    font-size: 1.4rem;
    font-weight: 600;
  }

  .placeholder p {
    color: var(--text-secondary);
  }

  .menu-toggle {
    display: none;
    position: fixed;
    top: 0.75rem;
    left: 0.75rem;
    z-index: 200;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    color: var(--text-primary);
    width: 36px;
    height: 36px;
    border-radius: var(--radius-xs);
    cursor: pointer;
    align-items: center;
    justify-content: center;
  }

  .overlay {
    display: none;
  }

  @media (max-width: 768px) {
    .menu-toggle {
      display: flex;
    }

    .sidebar {
      position: fixed;
      top: 0;
      left: 0;
      bottom: 0;
      transform: translateX(-100%);
      transition: transform 0.2s ease;
    }

    .sidebar.open {
      transform: translateX(0);
    }

    .sidebar-open .overlay {
      display: block;
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.5);
      z-index: 99;
    }

    .content {
      padding-top: 3rem;
    }
  }
</style>
