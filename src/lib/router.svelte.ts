/*
 * Minimal path-based router.
 *
 * The app has two screens, so a full routing framework would be dead weight
 * (and SvelteKit's SPA bootstrap is an inline <script>, which the strict CSP
 * of tauri.conf.json would block). Real <a href> elements are still rendered,
 * so links keep working with the keyboard and with middle-click.
 */
import type { Snippet } from "svelte";

export interface Route {
  path: string;
  render: () => Snippet;
}

function currentPath(): string {
  if (typeof window === "undefined") return "/";
  return window.location.pathname || "/";
}

class Router {
  path = $state(currentPath());

  /** Attach the popstate listener. Safe to call once. */
  start(): void {
    window.addEventListener("popstate", () => {
      this.path = currentPath();
    });
  }

  go(path: string): void {
    if (path === this.path) return;
    window.history.pushState({}, "", path);
    this.path = path;
  }

  /** Does `href` match the current path? The root only matches exactly. */
  isActive(href: string): boolean {
    return href === "/" ? this.path === "/" : this.path.startsWith(href);
  }

  /** Handler for an internal <a>: keep modified clicks native. */
  follow(event: MouseEvent, path: string): void {
    if (event.defaultPrevented || event.button !== 0) return;
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    this.go(path);
  }
}

export const router = new Router();