import { describe, expect, it } from "vitest";

import { MAX_SYSTEM_PROMPT_CHARS } from "@/lib/tauri/assistant";

import { promptEditState } from "./prompt-edit";

describe("promptEditState", () => {
  it("初期値のままなら保存も初期化もできない", () => {
    expect(promptEditState("a", "a", false)).toMatchObject({ dirty: false, canSave: false, canReset: false });
  });

  it("編集すると保存・初期化できる", () => {
    expect(promptEditState("ab", "a", false)).toMatchObject({ dirty: true, canSave: true, canReset: true });
  });

  it("カスタム保存済みなら、変更が無くても初期化できる", () => {
    expect(promptEditState("a", "a", true)).toMatchObject({ canSave: false, canReset: true });
  });

  it("空は保存できない", () => {
    expect(promptEditState("  \n", "a", false)).toMatchObject({ empty: true, canSave: false });
  });

  it("上限超えは保存できない(サロゲートペアは 1 文字)", () => {
    const over = "𠮷".repeat(MAX_SYSTEM_PROMPT_CHARS + 1);
    expect(promptEditState(over, "a", false)).toMatchObject({ tooLong: true, canSave: false });
    expect(promptEditState("𠮷".repeat(MAX_SYSTEM_PROMPT_CHARS), "a", false).tooLong).toBe(false);
  });
});
