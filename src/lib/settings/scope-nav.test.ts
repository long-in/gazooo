import { describe, expect, it } from "vitest";

import { parseSettingsLocation, settingsHref, tabsFor } from "./scope-nav";

describe("settings scope navigation", () => {
  it("opens the user settings by default", () => {
    expect(parseSettingsLocation(null, null)).toEqual({ scope: "user", tab: "storage" });
    expect(parseSettingsLocation(null, "assistant")).toEqual({ scope: "user", tab: "assistant" });
    expect(parseSettingsLocation("x", "x")).toEqual({ scope: "user", tab: "storage" });
  });

  it("sends engine links to the global settings, which alone has that tab", () => {
    expect(parseSettingsLocation(null, "engine")).toEqual({ scope: "global", tab: "engine" });
    expect(parseSettingsLocation("user", "engine")).toEqual({ scope: "user", tab: "storage" });
    expect(tabsFor("user")).not.toContain("engine");
    expect(tabsFor("global")[0]).toBe("engine");
  });

  it("keeps an explicit scope and builds links", () => {
    expect(parseSettingsLocation("global", "other")).toEqual({ scope: "global", tab: "other" });
    expect(settingsHref("global", "storage")).toBe("/settings?scope=global&tab=storage");
  });
});
