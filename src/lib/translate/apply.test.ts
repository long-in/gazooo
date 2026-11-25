import { describe, expect, it } from "vitest";

import { applyTranslation } from "./apply";

describe("applyTranslation", () => {
  it("置き換えは翻訳結果だけを返す", () => {
    expect(applyTranslation("old, prompt", " 1girl, solo \n", "replace")).toBe(
      "1girl, solo",
    );
  });

  it("末尾に追加はカンマ区切りで足す", () => {
    expect(applyTranslation("masterpiece", "1girl", "append")).toBe(
      "masterpiece, 1girl",
    );
  });

  it("末尾の区切りと空白は重複させない", () => {
    expect(applyTranslation("masterpiece, ", "1girl", "append")).toBe(
      "masterpiece, 1girl",
    );
    expect(applyTranslation("猫、\n", "a cat", "append")).toBe("猫, a cat");
  });

  it("現在が空なら、追加でも翻訳結果だけになる", () => {
    expect(applyTranslation("  ", "1girl", "append")).toBe("1girl");
  });

  it("翻訳結果が空なら、追加は現在を変えない", () => {
    expect(applyTranslation("masterpiece", "  ", "append")).toBe("masterpiece");
  });
});
