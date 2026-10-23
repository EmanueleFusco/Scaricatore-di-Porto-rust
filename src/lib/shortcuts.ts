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

/** Is the event coming from a control that owns the keyboard? */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (target === null || typeof target !== "object") return false;
  const element = target as {
    tagName?: unknown;
    isContentEditable?: unknown;
    getAttribute?: unknown;
  };
  const tag = typeof element.tagName === "string" ? element.tagName.toUpperCase() : "";
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return element.isContentEditable === true;
}

/**
 * Which app action a key press means, if any.
 *
 * `typing` is passed in rather than read from `event.target`, because callers
 * sometimes know better (for example a dialog that owns focus).
 */
export function shortcutAction(event: ShortcutEvent, typing: boolean): ShortcutAction | null {
  if (typing) return null;
  if (event.altKey) return null;
  const mod = event.ctrlKey || event.metaKey;
  if (mod && !event.shiftKey && event.key.toLocaleLowerCase() === "f") return "filter";
  if (mod && !event.shiftKey && event.key === ",") return "settings";
  if (mod) return null;
  if (event.key === "Delete") return "removeSelected";
  if (event.key === " " || event.key === "Spacebar") return "togglePause";
  return null;
}
