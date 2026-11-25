import { describe, expect, it } from "vitest";

import { type KeyLike, modifierLabel, navTargetFor, shouldCancelOnEscape } from "./shortcuts";

const key = (k: string, over: Partial<KeyLike> = {}): KeyLike => ({
  key: k,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...over,
});

describe("navTargetFor", () => {
  it("Ctrl/⌘ + 1..6 で画面を切り替える(OS ごとの修飾キー)", () => {
    expect(navTargetFor(key("1", { ctrlKey: true }), false)).toBe("/");
    expect(navTargetFor(key("2", { ctrlKey: true }), false)).toBe("/gallery");
    expect(navTargetFor(key("4", { ctrlKey: true }), false)).toBe("/models");
    expect(navTargetFor(key("5", { ctrlKey: true }), false)).toBe("/convert");
    expect(navTargetFor(key("6", { ctrlKey: true }), false)).toBe("/settings");
    expect(navTargetFor(key("3", { metaKey: true }), true)).toBe("/presets");
    expect(navTargetFor(key("7", { ctrlKey: true }), false)).toBeNull();
  });

  it("別 OS の修飾キーや、余計な修飾キーが付くものは無視する", () => {
    expect(navTargetFor(key("1", { metaKey: true }), false)).toBeNull(); // Windows で Win+1
    expect(navTargetFor(key("1", { ctrlKey: true }), true)).toBeNull(); // Mac で Ctrl+1
    expect(navTargetFor(key("1", { ctrlKey: true, shiftKey: true }), false)).toBeNull();
    expect(navTargetFor(key("1", { ctrlKey: true, altKey: true }), false)).toBeNull();
    expect(navTargetFor(key("1"), false)).toBeNull();
  });

  it("範囲外の数字・数字以外は無視する", () => {
    expect(navTargetFor(key("7", { ctrlKey: true }), false)).toBeNull();
    expect(navTargetFor(key("0", { ctrlKey: true }), false)).toBeNull();
    expect(navTargetFor(key("a", { ctrlKey: true }), false)).toBeNull();
  });
});

describe("shouldCancelOnEscape", () => {
  const ctx = { pathname: "/", dialogOpen: false, hasRunningJob: true };

  it("生成画面・ダイアログなし・実行中ジョブありのときだけキャンセルする", () => {
    expect(shouldCancelOnEscape(key("Escape"), ctx)).toBe(true);
    expect(shouldCancelOnEscape(key("Escape"), { ...ctx, pathname: "/gallery/" })).toBe(false);
    expect(shouldCancelOnEscape(key("Escape"), { ...ctx, dialogOpen: true })).toBe(false);
    expect(shouldCancelOnEscape(key("Escape"), { ...ctx, hasRunningJob: false })).toBe(false);
  });

  it("Escape 以外・修飾キー付きは対象外", () => {
    expect(shouldCancelOnEscape(key("Enter"), ctx)).toBe(false);
    expect(shouldCancelOnEscape(key("Escape", { ctrlKey: true }), ctx)).toBe(false);
  });

  it("末尾スラッシュ付きのルート(静的エクスポート)も生成画面として扱う", () => {
    expect(shouldCancelOnEscape(key("Escape"), { ...ctx, pathname: "//" })).toBe(true);
  });
});

describe("modifierLabel", () => {
  it("macOS は ⌘、それ以外は Ctrl+", () => {
    expect(modifierLabel(true)).toBe("⌘");
    expect(modifierLabel(false)).toBe("Ctrl+");
  });
});
