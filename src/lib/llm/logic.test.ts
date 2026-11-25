import { describe, expect, it } from "vitest";

import type { AssistantStatus } from "@/lib/tauri/assistant";
import type { LlmInstallState, LlmStatus } from "@/lib/tauri/llm";

import { canRelease, canStart, formatMemory, installPercent, panelGate, sendsExternally } from "./logic";

const status = (over: Partial<AssistantStatus> = {}): AssistantStatus => ({
  hasKey: false,
  verified: false,
  model: "claude-sonnet-5-5",
  consent: false,
  autoDelete: true,
  provider: "anthropic",
  localBaseUrl: "http://127.0.0.1:11434/v1",
  localModel: "qwen3.5:9b",
  ...over,
});

const install = (over: Partial<LlmInstallState> = {}): LlmInstallState => ({
  installId: "i",
  status: "running",
  step: 1,
  totalSteps: 3,
  stepId: "runtime",
  received: 0,
  total: 100,
  error: null,
  ...over,
});

const llm = (over: Partial<LlmStatus> = {}): LlmStatus => ({
  supported: true,
  licenseFiles: [],
  installed: true,
  updateAvailable: false,
  runtime: "b12426",
  model: "Qwen3.5-9B (Q4_K_M)",
  runtimeLicense: "MIT",
  modelLicense: "Apache-2.0",
  downloadMb: 5500,
  state: "stopped",
  message: null,
  idleMinutes: 5,
  releaseBeforeGeneration: true,
  memoryHintMb: null,
  install: install({ status: "idle", step: 0 }),
  ...over,
});

describe("panelGate", () => {
  it("anthropic needs a key and consent", () => {
    expect(panelGate(null, null)).toBe("loading");
    expect(panelGate(status(), null)).toBe("noKey");
    expect(panelGate(status({ hasKey: true }), null)).toBe("consent");
    expect(panelGate(status({ hasKey: true, consent: true }), null)).toBe("chat");
  });

  it("local needs neither a key nor consent, only an installation", () => {
    expect(panelGate(status({ provider: "local" }), llm({ installed: false, state: "not_installed" }))).toBe(
      "notInstalled",
    );
    expect(panelGate(status({ provider: "local" }), llm())).toBe("chat");
    // 状態が未取得の間は案内を出さない
    expect(panelGate(status({ provider: "local" }), null)).toBe("chat");
  });

  it("external local server needs nothing", () => {
    expect(panelGate(status({ provider: "local_external" }), null)).toBe("chat");
  });

  it("only anthropic sends data outside", () => {
    expect(sendsExternally(status())).toBe(true);
    expect(sendsExternally(status({ provider: "local" }))).toBe(false);
    expect(sendsExternally(status({ provider: "local_external" }))).toBe(false);
    expect(sendsExternally(null)).toBe(false);
  });
});

describe("installPercent", () => {
  it("weights the model download the most", () => {
    expect(installPercent(install({ step: 1, received: 50, total: 100 }))).toBe(3);
    expect(installPercent(install({ step: 2, stepId: "extract", received: null, total: null }))).toBe(8);
    expect(installPercent(install({ step: 3, stepId: "model", received: 50, total: 100 }))).toBe(55);
    expect(installPercent(install({ step: 3, stepId: "model", received: 100, total: 100 }))).toBe(100);
  });

  it("is null when not running and 100 when done", () => {
    expect(installPercent(install({ status: "idle", step: 0 }))).toBeNull();
    expect(installPercent(install({ status: "failed" }))).toBeNull();
    expect(installPercent(install({ status: "done", step: 3 }))).toBe(100);
  });

  it("never exceeds the step weight even if received overshoots", () => {
    expect(installPercent(install({ step: 3, stepId: "model", received: 500, total: 100 }))).toBe(100);
  });
});

describe("state helpers", () => {
  it("canRelease only while running or starting", () => {
    expect(canRelease("ready")).toBe(true);
    expect(canRelease("starting")).toBe(true);
    expect(canRelease("stopped")).toBe(false);
    expect(canRelease("not_installed")).toBe(false);
    expect(canRelease(undefined)).toBe(false);
  });

  it("canStart needs an installed, stopped (or crashed) runtime", () => {
    expect(canStart(llm())).toBe(true);
    expect(canStart(llm({ state: "error" }))).toBe(true);
    expect(canStart(llm({ state: "ready" }))).toBe(false);
    expect(canStart(llm({ installed: false, state: "not_installed" }))).toBe(false);
    expect(canStart(null)).toBe(false);
  });

  it("formats memory", () => {
    expect(formatMemory(800)).toBe("800 MB");
    expect(formatMemory(6656)).toBe("6.5 GB");
  });
});
