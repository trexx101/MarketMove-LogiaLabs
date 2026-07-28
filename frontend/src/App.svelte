<script>
  import Dashboard from './views/Dashboard.svelte';
  import Ledger from './views/Ledger.svelte';
  import { status } from './lib/stores.js';

  let currentView = 'dashboard';
  let sidebarOpen = false;

  $: mode = $status?.mode || 'PAPER';

  function nav(view) {
    currentView = view;
    sidebarOpen = false;
  }
</script>

<div class="app-shell" class:sidebar-open={sidebarOpen}>
  <button class="menu-toggle" on:click={() => sidebarOpen = !sidebarOpen}>
    ☰
  </button>

  <nav class="sidebar" class:open={sidebarOpen}>
    <div class="logo">MarketMoves</div>
    <ul>
      <li>
        <button class="nav-btn" class:active={currentView === 'dashboard'} on:click={() => nav('dashboard')}>
          <span class="nav-icon">📊</span> Dashboard
        </button>
      </li>
      <li>
        <button class="nav-btn" class:active={currentView === 'strategy'} on:click={() => nav('strategy')}>
          <span class="nav-icon">🧪</span> Strategy Lab
        </button>
      </li>
      <li>
        <button class="nav-btn" class:active={currentView === 'ledger'} on:click={() => nav('ledger')}>
          <span class="nav-icon">📒</span> Ledger
        </button>
      </li>
      <li>
        <button class="nav-btn" class:active={currentView === 'advisor'} on:click={() => nav('advisor')}>
          <span class="nav-icon">🤖</span> Advisor
        </button>
      </li>
    </ul>
    <div class="mode-badge mode-{mode.toLowerCase()}">{mode}</div>
  </nav>

  {#if sidebarOpen}
    <div class="overlay" role="button" tabindex="0"
         on:click={() => sidebarOpen = false} on:keydown={(e) => e.key === 'Enter' && (sidebarOpen = false)}></div>
  {/if}

  <main class="content">
    {#if currentView === 'dashboard'}
      <Dashboard />
    {:else if currentView === 'strategy'}
      <div class="placeholder">
        <h1>Strategy Lab</h1>
        <p>Coming in Phase 2.</p>
      </div>
    {:else if currentView === 'ledger'}
      <Ledger />
    {:else if currentView === 'advisor'}
      <div class="placeholder">
        <h1>AI Advisor</h1>
        <p>Coming in Phase 4.</p>
      </div>
    {/if}
  </main>
</div>

<style>
  :global(*) {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
  }

  :global(body) {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: #0d1117;
    color: #c9d1d9;
  }

  .app-shell {
    display: flex;
    min-height: 100vh;
  }

  .sidebar {
    width: 220px;
    background: #161b22;
    border-right: 1px solid #30363d;
    display: flex;
    flex-direction: column;
    padding: 1rem 0;
    flex-shrink: 0;
    z-index: 100;
  }

  .logo {
    font-size: 1.1rem;
    font-weight: 700;
    padding: 0 1.2rem 1rem;
    color: #58a6ff;
    border-bottom: 1px solid #30363d;
    margin-bottom: 0.5rem;
  }

  .sidebar ul {
    list-style: none;
    flex: 1;
  }

  .sidebar li {
    list-style: none;
  }

  .nav-btn {
    width: 100%;
    text-align: left;
    padding: 0.7rem 1.2rem;
    cursor: pointer;
    color: #8b949e;
    transition: all 0.15s;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: none;
    border: none;
    border-left: 3px solid transparent;
    font-size: 0.95rem;
    font-family: inherit;
  }

  .nav-btn:hover {
    background: #21262d;
    color: #c9d1d9;
  }

  .nav-btn.active {
    background: #1f6feb22;
    color: #58a6ff;
    border-left-color: #58a6ff;
  }

  .nav-icon {
    font-size: 1rem;
  }

  .mode-badge {
    margin: 0 1.2rem;
    padding: 0.4rem 0.8rem;
    border-radius: 4px;
    font-size: 0.75rem;
    font-weight: 600;
    text-align: center;
  }

  .mode-paper { background: #1f6feb22; color: #58a6ff; }
  .mode-live { background: #da363322; color: #f85149; }

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
    color: #c9d1d9;
    margin-bottom: 0.5rem;
  }

  .placeholder p {
    color: #8b949e;
  }

  .menu-toggle {
    display: none;
    position: fixed;
    top: 0.75rem;
    left: 0.75rem;
    z-index: 200;
    background: #161b22;
    border: 1px solid #30363d;
    color: #c9d1d9;
    width: 36px;
    height: 36px;
    border-radius: 6px;
    font-size: 1.1rem;
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
