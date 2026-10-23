/**
 * Keyboard shortcuts shared by the shell and the queue (spec §11.5, §11.9).
 *
 * Pure and testable: the components only map an action to a handler, and the
 * rules stay the same on every platform (`Ctrl` and `Cmd` are both accepted).
 * A shortcut never fires while the user is typing in a form control, so
 * Delete still deletes a character in the add bar instead of a row.
 */

export type ShortcutAction = "filter" | "removeSelected" | "togglePause" | "settings";

/** The subset of `KeyboardEvent` the rules need; keeps the tests small. */
export interface ShortcutEvent {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

/** Input types that consume typed characters (and therefore shortcuts). */
const TEXT_INPUT_TYPES = new Set([
  "text",
  "search",
  "url",
  "tel",
  "email",
  "password",
  "number",
  "date",
  "datetime-local",
  "month",
  "week",
  "time",
]);

function elementOf(target: EventTarget | null): {
  tagName?: unknown;
  type?: unknown;
  isContentEditable?: unknown;
  getAttribute?: unknown;
} | null {
  if (target === null || typeof target !== "object") return null;
  return target as {
    tagName?: unknown;
    type?: unknown;
    isContentEditable?: unknown;
    getAttribute?: unknown;
  };
}

function tagOf(element: ReturnType<typeof elementOf>): string {
  return element !== null && typeof element.tagName === "string"
    ? element.tagName.toUpperCase()
    : "";
}

/**
 * Is the event coming from a control that owns typed characters?
 *
 * Checkboxes and radio buttons are inputs but do not consume text, so the
 * modified shortcuts (Ctrl/Cmd+F, Ctrl/Cmd+,) still work after the user
 * clicks one. Their unmodified keys are handled by `isInteractiveTarget`.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
  const element = elementOf(target);
  if (element === null) return false;
  const tag = tagOf(element);
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = typeof element.type === "string" ? element.type.toLowerCase() : "text";
    return TEXT_INPUT_TYPES.has(type);
  }
  return element.isContentEditable === true;
}

/**
 * Is the event coming from any control with a native meaning for Delete or
 * Space? Buttons, links and checkboxes must keep those keys for themselves.
 */
export function isInteractiveTarget(target: EventTarget | null): boolean {
  const element = elementOf(target);
  if (element === null) return false;
  const tag = tagOf(element);
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || tag === "BUTTON") {
    return true;
  }
  if (tag === "A" && typeof element.getAttribute === "function") {
    return element.getAttribute("href") !== null;
  }
  return element.isContentEditable === true;
}

/**
 * Which app action a key press means, if any.
 *
 * `typing` and `interactive` are passed in rather than read from
 * `event.target`, because callers sometimes know better (for example a dialog
 * that owns focus). `interactive` defaults to `typing`, which keeps the
 * two-argument calls correct.
 */
export function shortcutAction(
  event: ShortcutEvent,
  typing: boolean,
  interactive: boolean = typing,
): ShortcutAction | null {
  if (event.altKey) return null;
  const mod = event.ctrlKey || event.metaKey;
  if (mod) {
    if (typing) return null;
    if (!event.shiftKey && event.key.toLocaleLowerCase() === "f") return "filter";
    if (!event.shiftKey && event.key === ",") return "settings";
    return null;
  }
  if (interactive) return null;
  if (event.key === "Delete") return "removeSelected";
  if (event.key === " " || event.key === "Spacebar") return "togglePause";
  return null;
}
