# Svelte 4 A11y Warning Resolution Reference

## The Problem

The Svelte 4 compiler emits accessibility warnings for click handlers on
non-interactive elements (`<li>`, `<div>`). These are warnings, not errors —
the build succeeds — but they indicate real accessibility issues and should
be fixed, not suppressed.

## Resolution Ladder (Tested on Svelte 4 + Vite 5)

### Step 1: `on:click` on `<li>` (FAILS)

```svelte
<li on:click={() => nav('dashboard')}>Dashboard</li>
```

Warnings:
- "visible, non-interactive elements with an:click event must be accompanied
  by a keyboard event handler"
- "Non-interactive element `<li>` should not be assigned mouse or keyboard
  event listeners"

### Step 2: Add `role="button"` + `tabindex="0"` + `on:keydown` (STILL FAILS)

```svelte
<li role="button" tabindex="0"
    on:click={() => nav('dashboard')}
    on:keydown={(e) => e.key === 'Enter' && nav('dashboard')}>
  Dashboard
</li>
```

Warning:
- "Non-interactive element `<li>` cannot have interactive role 'button'"

### Step 3: Use `<button>` inside `<li>` (WORKS — zero warnings)

```svelte
<ul>
  <li>
    <button class="nav-btn" class:active={currentView === 'dashboard'}
            on:click={() => nav('dashboard')}>
      Dashboard
    </button>
  </li>
</ul>
```

CSS to make the button fill the list item:

```css
.sidebar li { list-style: none; }

.nav-btn {
  width: 100%;
  text-align: left;
  padding: 0.7rem 1.2rem;
  background: none;
  border: none;
  border-left: 3px solid transparent;
  font-family: inherit;
  font-size: 0.95rem;
  cursor: pointer;
  color: inherit;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.nav-btn:hover {
  background: var(--hover-bg);
  color: var(--hover-fg);
}

.nav-btn.active {
  background: var(--active-bg);
  color: var(--accent);
  border-left-color: var(--accent);
}
```

## Overlay Divs

The same issue applies to overlay/backdrop divs with click handlers:

```svelte
<!-- FAILS: a11y warnings -->
<div class="overlay" on:click={() => sidebarOpen = false}></div>

<!-- WORKS: button element -->
<button class="overlay" on:click={() => sidebarOpen = false}></button>

<!-- ALSO WORKS: div with role + keyboard handler -->
<div class="overlay" role="button" tabindex="0"
     on:click={() => sidebarOpen = false}
     on:keydown={(e) => e.key === 'Enter' && (sidebarOpen = false)}>
</div>
```

For overlays, `<button>` is preferred — it's semantically correct for a
dismissable backdrop and gets keyboard handling for free.

## Key Takeaway

**Never use `role="button"` on `<li>` elements in Svelte 4.** The compiler
considers `<li>` a non-interactive element that cannot be promoted to an
interactive role. Use a real `<button>` inside the `<li>` instead. This is
both the accessible solution and the one that produces zero compiler warnings.

## Modal Backdrop (Click-Outside-to-Close)

### What Doesn't Work

```svelte
<!-- FAILS: a11y warnings on inner div -->
<div class="modal-backdrop" on:click={closeModal}>
  <div class="modal" on:click|stopPropagation>
    <!-- content -->
  </div>
</div>
```

Warnings:
- "visible, non-interactive elements with on:click event must be accompanied
  by a keyboard event handler" (on both divs)
- "Non-interactive element `<div>` should not be assigned mouse or keyboard
  event listeners" (on inner div with stopPropagation)

Adding `role="dialog"` to the inner div does NOT fix it — the compiler
still sees a `<div>` with a mouse event listener.

### What Works

Remove the `stopPropagation` handler from the inner div entirely. On the
backdrop, check `e.target === e.currentTarget` before closing — clicks
that originate inside the modal will have `e.target` pointing to a child
element, not the backdrop, so they naturally won't trigger close:

```svelte
<div class="modal-backdrop"
     on:click={(e) => { if (e.target === e.currentTarget) closeModal(); }}
     on:keydown={(e) => { if (e.key === 'Escape') closeModal(); }}
     role="button" tabindex="0">
  <div class="modal" role="dialog" aria-modal="true">
    <!-- content — no click handler needed -->
  </div>
</div>
```

This produces zero a11y warnings. The `role="button"` + `tabindex="0"` on
the backdrop is valid because the backdrop IS interactive (click to close).
The inner modal has no click handler, so no warning.
