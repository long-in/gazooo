import { beforeEach, describe, expect, it, vi } from "vitest";

import type { LlmInstallState, LlmStatus } from "@/lib/tauri/llm";

const api = vi.hoisted(() => ({
  llmStatus: vi.fn(),
  llmInstall: vi.fn(),
  llmInstallCancel: vi.fn(),
  llmRemove: vi.fn(),
  llmStart: vi.fn(),
  llmStop: vi.fn(),
  llmLogs: vi.fn(),
  llmUpdateSettings: vi.fn(),
}));
vi.mock("@/lib/tauri/llm", () => api);

import { useLlmStore } from "./llm-store";

const idle: LlmInstallState = {
  installId: null,
  status: "idle",
  step: 0,
  totalSteps: 3,
  stepId: null,
  received: null,
  total: null,
  error: null,
};

const status = (over: Partial<LlmStatus> = {}): LlmStatus => ({
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
  install: idle,
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  useLlmStore.setState({ status: null, logs: [], busy: false });
});

describe("llm-store", () => {
  it("loads the status and ignores load failures", async () => {
    api.llmStatus.mockResolvedValueOnce(status());
    await useLlmStore.getState().load();
    expect(useLlmStore.getState().status?.installed).toBe(true);

    api.llmStatus.mockRejectedValueOnce({ code: "INTERNAL", message: "x" });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    await useLlmStore.getState().load();
    expect(useLlmStore.getState().status?.installed).toBe(true);
    warn.mockRestore();
  });

  it("applies state events only once installed", () => {
    useLlmStore.getState().applyState("ready", null);
    expect(useLlmStore.getState().status).toBeNull();

    useLlmStore.setState({ status: status({ installed: false, state: "not_installed" }) });
    useLlmStore.getState().applyState("starting", null);
    expect(useLlmStore.getState().status?.state).toBe("not_installed");

    useLlmStore.setState({ status: status() });
    useLlmStore.getState().applyState("starting", null);
    expect(useLlmStore.getState().status?.state).toBe("starting");
    useLlmStore.getState().applyState("error", "起動できません");
    expect(useLlmStore.getState().status).toMatchObject({ state: "error", message: "起動できません" });
  });

  it("reloads the status when an install finishes, fails or is cancelled", () => {
    useLlmStore.setState({ status: status({ installed: false, state: "not_installed" }) });
    api.llmStatus.mockResolvedValue(status());

    useLlmStore.getState().applyInstall({ ...idle, status: "running", step: 3, received: 1, total: 10 });
    expect(api.llmStatus).not.toHaveBeenCalled();
    expect(useLlmStore.getState().status?.install.status).toBe("running");

    for (const s of ["done", "failed", "cancelled"] as const) {
      api.llmStatus.mockClear();
      useLlmStore.getState().applyInstall({ ...idle, status: s });
      expect(api.llmStatus).toHaveBeenCalledTimes(1);
    }
  });

  it("start / stop / remove set busy during the call and store the result", async () => {
    let resolve!: (s: LlmStatus) => void;
    api.llmStop.mockReturnValueOnce(new Promise<LlmStatus>((r) => (resolve = r)));
    useLlmStore.setState({ status: status({ state: "ready" }) });
    const p = useLlmStore.getState().stop();
    expect(useLlmStore.getState().busy).toBe(true);
    resolve(status({ state: "stopped" }));
    await p;
    expect(useLlmStore.getState()).toMatchObject({ busy: false });
    expect(useLlmStore.getState().status?.state).toBe("stopped");

    api.llmStart.mockResolvedValueOnce(status({ state: "starting" }));
    await useLlmStore.getState().start();
    expect(useLlmStore.getState().status?.state).toBe("starting");

    api.llmRemove.mockResolvedValueOnce(status({ installed: false, state: "not_installed" }));
    await useLlmStore.getState().remove();
    expect(useLlmStore.getState().status?.installed).toBe(false);
  });

  it("clears busy and rethrows when an operation fails", async () => {
    api.llmStart.mockRejectedValueOnce({ code: "LLM_BUSY", message: "x" });
    await expect(useLlmStore.getState().start()).rejects.toMatchObject({ code: "LLM_BUSY" });
    expect(useLlmStore.getState().busy).toBe(false);
  });

  it("cancels the running install by its id", async () => {
    useLlmStore.setState({ status: status({ install: { ...idle, status: "running", installId: "abc" } }) });
    await useLlmStore.getState().cancelInstall();
    expect(api.llmInstallCancel).toHaveBeenCalledWith("abc");

    api.llmInstallCancel.mockClear();
    useLlmStore.setState({ status: status() });
    await useLlmStore.getState().cancelInstall();
    expect(api.llmInstallCancel).not.toHaveBeenCalled();
  });

  it("stores the settings returned by the backend", async () => {
    useLlmStore.setState({ status: status() });
    api.llmUpdateSettings.mockResolvedValueOnce(status({ idleMinutes: 15 }));
    await useLlmStore.getState().updateSettings({ idleMinutes: 15 });
    expect(api.llmUpdateSettings).toHaveBeenCalledWith({ idleMinutes: 15 });
    expect(useLlmStore.getState().status?.idleMinutes).toBe(15);
  });
});
