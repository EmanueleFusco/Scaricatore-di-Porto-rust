<script lang="ts">
  /** Application navigation (spec §11.2). Collapses to icons under 800 px. */
  // Icons come from Lucide (ISC) and are rendered as **inline SVG**, as spec §1
  // requires: no icon font, no remote asset, no hand-written path data.
  // `lucide-svelte/icons/<name>` imports a single icon, and the package is
  // `sideEffects: false`, so only the icons we actually use reach the bundle.
  import DownloadIcon from "lucide-svelte/icons/download";
  import ListVideoIcon from "lucide-svelte/icons/list-video";
  import SettingsIcon from "lucide-svelte/icons/settings";
  import { router } from "$lib/router.svelte";
  import { tt } from "$lib/stores/settings.svelte";

  // Every Lucide icon has the same signature, so one of them types the array.
  const links: { href: string; key: string; icon: typeof ListVideoIcon }[] = [
    { href: "/", key: "nav.queue", icon: ListVideoIcon },
    { href: "/settings", key: "nav.settings", icon: SettingsIcon },
  ];
</script>

<nav class="sidebar" aria-label={tt("a11y.sidebar")}>
  <a class="sidebar__brand" href="/" draggable="false" onclick={(event) => router.follow(event, "/")}>
    <DownloadIcon size={24} aria-hidden="true" focusable="false" />
    <span class="sidebar__title">{tt("app.name")}</span>
  </a>

  <ul class="sidebar__list">
    {#each links as link (link.href)}
      <li>
        <a
          class="sidebar__link"
          href={link.href}
          draggable="false"
          aria-current={router.isActive(link.href) ? "page" : undefined}
          onclick={(event) => router.follow(event, link.href)}
        >
          <link.icon size={20} aria-hidden="true" focusable="false" />
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
    user-select: none;
    -webkit-user-select: none;
    -webkit-user-drag: none;
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
    user-select: none;
    -webkit-user-select: none;
    -webkit-user-drag: none;
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