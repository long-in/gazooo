import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./invoke", () => ({ invoke: vi.fn() }));

import { type ManagedEngineConfig, applySuggestions } from "./engine-process";

const base: ManagedEngineConfig = {
  enabled: false,
  mainPy: "",
  python: "",
  baseDirectory: null,
  port: 8188,
  autoStart: false,
  autoRecover: false,
};

describe("applySuggestions", () => {
  it("fills only empty fields", () => {
    const out = applySuggestions(
      { ...base, python: "mine" },
      { mainPy: "m", python: "p", baseDirectory: "d" },
    );
    expect(out).toMatchObject({ mainPy: "m", python: "mine", baseDirectory: "d" });
  });

  it("leaves fields empty when there is no suggestion", () => {
    expect(applySuggestions(base, {})).toEqual(base);
  });
});
