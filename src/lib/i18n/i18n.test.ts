import { describe, expect, it } from "vitest";

import { en } from "./en";
import { resolveLanguage, translate } from "./index";
import { ja } from "./ja";

describe("i18n", () => {
  it("ja と en のキーが一致している", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(ja).sort());
  });

  it("空の翻訳がない", () => {
    for (const dict of [ja, en]) {
      for (const [key, value] of Object.entries(dict)) {
        expect(value, key).not.toBe("");
      }
    }
  });

  it("設定値があればそれを優先する", () => {
    expect(resolveLanguage("en")).toBe("en");
    expect(resolveLanguage("ja")).toBe("ja");
  });

  it("translate は指定言語の文言を返す", () => {
    expect(translate("ja", "nav.settings")).toBe("設定");
    expect(translate("en", "nav.settings")).toBe("Settings");
  });
});
