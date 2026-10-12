<script lang="ts">
  /** Application navigation (spec §11.2). Collapses to icons under 800 px. */
  import { router } from "$lib/router.svelte";
  import { tt } from "$lib/stores/settings.svelte";

  const links = [
    { href: "/", key: "nav.queue", icon: "queue" },
    { href: "/settings", key: "nav.settings", icon: "gear" },
  ] as const;
</script>

<nav class="sidebar" aria-label={tt("a11y.sidebar")}>
  <a
    class="sidebar__brand"
    href="/"
    onclick={(event) => router.follow(event, "/")}
  >
    <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true" focusable="false">
      <path d="M12 3v11m0 0 4-4m-4 4-4-4" fill="none" stroke="currentColor" stroke-width="2"
        stroke-linecap="round" stroke-linejoin="round" />
      <path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" fill="none" stroke="currentColor"
        stroke-width="2" stroke-linecap="round" />
    </svg>
    <span class="sidebar__title">{tt("app.name")}</span>
  </a>

  <ul class="sidebar__list">
    {#each links as link (link.href)}
      <li>
        <a
          class="sidebar__link"
          href={link.href}
          aria-current={router.isActive(link.href) ? "page" : undefined}
          onclick={(event) => router.follow(event, link.href)}
        >
          {#if link.icon === "queue"}
            <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" focusable="false">
              <path
                d="M4 6h16M4 12h16M4 18h10"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
              />
            </svg>
          {:else}
            <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" focusable="false">
              <circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="2" />
              <path
                d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-2.9 1.2 2 2 0 1 1-4 0A1.7 1.7 0 0 0 7 19.4a1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0-1.2-2.9 2 2 0 1 1 0-4 1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H8a1.7 1.7 0 0 0 1-1.5 2 2 0 1 1 4 0A1.7 1.7 0 0 0 15 4.6a1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
            </svg>
          {/if}
          <span class="sidebar__label">{tt(link.key)}</span>
        </a>
      </li>
    {/each}
  </ul>
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: 216px;
    flex: 0 0 auto;
    padding: var(--space-4) var(--space-3);
    background: var(--surface);
    border-right: 1px solid var(--border);
    overflow: auto;
  }

  .sidebar__brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    color: var(--text);
    text-decoration: none;
    font-size: 13px;
    font-weight: 700;
    line-height: 1.25;
  }

  .sidebar__title {
    /* Wrap rather than truncate: the app name is the only word here. */
    overflow-wrap: anywhere;
  }

  .sidebar__list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .sidebar__link {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: var(--row-height);
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
    color: var(--text);
    text-decoration: none;
    transition: background var(--transition);
  }

  .sidebar__link:hover {
    background: var(--surface-hover);
  }

  /* Current page: bold text plus a left bar, never colour alone. */
  .sidebar__link[aria-current="page"] {
    background: var(--surface-hover);
    box-shadow: inset 3px 0 0 var(--accent);
    font-weight: 700;
  }

  /* Under ~800 px the sidebar collapses to icons (spec §11.2). */
  @media (max-width: 800px) {
    .sidebar {
      width: 56px;
      padding-inline: var(--space-2);
    }

    .sidebar__title,
    .sidebar__label {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0 0 0 0);
      white-space: nowrap;
    }

    .sidebar__brand,
    .sidebar__link {
      justify-content: center;
      padding-inline: var(--space-2);
    }
  }
</style>