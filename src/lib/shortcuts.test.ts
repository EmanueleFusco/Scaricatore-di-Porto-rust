/**
 * Tests for the keyboard shortcut rules (spec §11.5, §11.9).
 */
import { describe, expect, it } from "vitest";
import { isTypingTarget, shortcutAction, type ShortcutEvent } from "$lib/shortcuts";

function press(overrides: Partial<ShortcutEvent> = {}): ShortcutEvent {
  return {
    key: "",
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...overrides,
  };
}

describe("the app shortcuts", () => {
  it("maps Ctrl/Cmd+F to the queue filter", () => {
    expect(shortcutAction(press({ key: "f", ctrlKey: true }), false)).toBe("filter");
    expect(shortcutAction(press({ key: "F", metaKey: true }), false)).toBe("filter");
  });

  it("maps Ctrl/Cmd+, to settings", () => {
    expect(shortcutAction(press({ key: ",", ctrlKey: true }), false)).toBe("settings");
    expect(shortcutAction(press({ key: ",", metaKey: true }), false)).toBe("settings");
  });

  it("maps Delete and Space to the selection actions", () => {
    expect(shortcutAction(press({ key: "Delete" }), false)).toBe("removeSelected");
    expect(shortcutAction(press({ key: " " }), false)).toBe("togglePause");
  });

  it("does nothing while the user is typing", () => {
    expect(shortcutAction(press({ key: "Delete" }), true)).toBeNull();
    expect(shortcutAction(press({ key: " ", ctrlKey: true }), true)).toBeNull();
    expect(shortcutAction(press({ key: "f", ctrlKey: true }), true)).toBeNull();
  });

  it("does not steal modified keys or Alt combinations", () => {
    expect(shortcutAction(press({ key: "Delete", ctrlKey: true }), false)).toBeNull();
    expect(shortcutAction(press({ key: " ", altKey: true }), false)).toBeNull();
    expect(shortcutAction(press({ key: "f", ctrlKey: true, shiftKey: true }), false)).toBeNull();
  });

  it("ignores ordinary typing keys", () => {
    expect(shortcutAction(press({ key: "a" }), false)).toBeNull();
    expect(shortcutAction(press({ key: "Enter" }), false)).toBeNull();
  });

  it("recognises the controls that own the keyboard", () => {
    expect(isTypingTarget({ tagName: "INPUT" } as unknown as EventTarget)).toBe(true);
    expect(isTypingTarget({ tagName: "textarea" } as unknown as EventTarget)).toBe(true);
    expect(isTypingTarget({ tagName: "SELECT" } as unknown as EventTarget)).toBe(true);
    expect(isTypingTarget({ tagName: "BUTTON" } as unknown as EventTarget)).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });
});
