import { describe, expect, it } from "vitest";

import { isTypingTarget, moveViewer, pickAfterRemoval, viewerKeyAction } from "./viewer-nav";

const ids = ["a", "b", "c"];

describe("moveViewer", () => {
  it("moves and stops at the ends", () => {
    expect(moveViewer(ids, "a", "next")).toBe("b");
    expect(moveViewer(ids, "c", "next")).toBe("c");
    expect(moveViewer(ids, "a", "prev")).toBe("a");
    expect(moveViewer(ids, "b", "prev")).toBe("a");
  });
  it("jumps to first / last", () => {
    expect(moveViewer(ids, "b", "first")).toBe("a");
    expect(moveViewer(ids, "b", "last")).toBe("c");
  });
  it("falls back to the first when current is unknown, null when empty", () => {
    expect(moveViewer(ids, "zzz", "next")).toBe("a");
    expect(moveViewer(ids, null, "prev")).toBe("a");
    expect(moveViewer([], "a", "next")).toBeNull();
  });
});

describe("pickAfterRemoval", () => {
  it("keeps the selection when it still exists", () => {
    expect(pickAfterRemoval(ids, ["a", "b"], "b")).toBe("b");
  });
  it("prefers the next, then the previous", () => {
    expect(pickAfterRemoval(ids, ["a", "c"], "b")).toBe("c");
    expect(pickAfterRemoval(ids, ["a", "b"], "c")).toBe("b");
  });
  it("handles empty and unknown", () => {
    expect(pickAfterRemoval(ids, [], "a")).toBeNull();
    expect(pickAfterRemoval(ids, ["b"], null)).toBe("b");
  });
});

describe("viewerKeyAction", () => {
  it("maps keys", () => {
    expect(viewerKeyAction({ key: "ArrowRight" })).toBe("next");
    expect(viewerKeyAction({ key: "ArrowLeft" })).toBe("prev");
    expect(viewerKeyAction({ key: "f" })).toBe("favorite");
    expect(viewerKeyAction({ key: "Delete" })).toBe("delete");
    expect(viewerKeyAction({ key: "x" })).toBeNull();
  });
  it("ignores IME composition and modified keys", () => {
    expect(viewerKeyAction({ key: "Enter", isComposing: true })).toBeNull();
    expect(viewerKeyAction({ key: "ArrowRight", ctrlKey: true })).toBeNull();
  });
  it("maps modifier shortcuts per OS", () => {
    expect(viewerKeyAction({ key: "g", metaKey: true }, true)).toBe("favoriteOn");
    expect(viewerKeyAction({ key: "c", metaKey: true }, true)).toBe("favoriteOff");
    expect(viewerKeyAction({ key: "Backspace", metaKey: true }, true)).toBe("delete");
    expect(viewerKeyAction({ key: "g", ctrlKey: true }, false)).toBe("favoriteOn");
    expect(viewerKeyAction({ key: "c", ctrlKey: true }, false)).toBe("favoriteOff");
    expect(viewerKeyAction({ key: "Backspace", ctrlKey: true }, false)).toBe("delete");
  });
  it("ignores the other OS modifier, extra modifiers and IME", () => {
    expect(viewerKeyAction({ key: "g", ctrlKey: true }, true)).toBeNull();
    expect(viewerKeyAction({ key: "g", metaKey: true }, false)).toBeNull();
    expect(viewerKeyAction({ key: "g", metaKey: true, shiftKey: true }, true)).toBeNull();
    expect(viewerKeyAction({ key: "c", ctrlKey: true, altKey: true }, false)).toBeNull();
    expect(viewerKeyAction({ key: "g", metaKey: true, isComposing: true }, true)).toBeNull();
  });
});

describe("isTypingTarget", () => {
  const el = (tag: string, inDialog = false) =>
    ({ tagName: tag, isContentEditable: false, closest: () => (inDialog ? {} : null) }) as unknown as Element;
  it("detects inputs and dialogs", () => {
    expect(isTypingTarget(el("INPUT"))).toBe(true);
    expect(isTypingTarget(el("BUTTON", true))).toBe(true);
    expect(isTypingTarget(el("BUTTON"))).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });
});
