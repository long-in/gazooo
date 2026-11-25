import { beforeEach, describe, expect, it, vi } from "vitest";

import type { EngineProcState, EngineSuggestions, ManagedView } from "@/lib/tauri/engine-process";

const get = vi.fn<() => Promise<ManagedView>>();
const suggest = vi.fn<() => Promise<EngineSuggestions>>();
const restart = vi.fn<(force?: boolean) => Promise<EngineProcState>>();
const start = vi.fn<() => Promise<EngineProcState>>();

vi.mock("@/lib/tauri/engine-process", () => ({
  engineManagedGet: () => get(),
  engineManagedSet: vi.fn(),
  engineManagedSuggest: () => suggest(),
  engineProcessStart: () => start(),
  engineProcessStop: vi.fn(),
  engineProcessRestart: (force?: boolean) => restart(force),
  engineProcessLogs: vi.fn(async () => ({ lines: ["a", "b"] })),
}));

import { LOG_LINES, useEngineProcessStore } from "./engine-process-store";

const running: EngineProcState = { status: "running", managedByApp: true, pid: 1 };
const view: ManagedView = {
  config: { enabled: true, mainPy: "m", python: "p", baseDirectory: null, port: 8188, autoStart: false, autoRecover: false },
  state: { status: "stopped", managedByApp: false },
};

beforeEach(() => {
  vi.clearAllMocks();
  useEngineProcessStore.setState({ view: null, state: null, suggestions: null, logs: [], busy: false });
});

describe("engine process store", () => {
  it("loads the view and its state", async () => {
    get.mockResolvedValue(view);
    await useEngineProcessStore.getState().load();
    expect(useEngineProcessStore.getState().state?.status).toBe("stopped");
    expect(useEngineProcessStore.getState().view?.config.port).toBe(8188);
  });

  it("loads suggestions separately and treats a failure as none", async () => {
    suggest.mockResolvedValueOnce({ mainPy: "C:\c\main.py" });
    await useEngineProcessStore.getState().loadSuggestions();
    expect(useEngineProcessStore.getState().suggestions).toEqual({ mainPy: "C:\c\main.py" });

    useEngineProcessStore.setState({ suggestions: null });
    suggest.mockRejectedValueOnce({ code: "INTERNAL", message: "x" });
    await useEngineProcessStore.getState().loadSuggestions();
    expect(useEngineProcessStore.getState().suggestions).toEqual({});
  });

  it("applies the result of start and clears busy", async () => {
    start.mockResolvedValue(running);
    await useEngineProcessStore.getState().start();
    const s = useEngineProcessStore.getState();
    expect(s.state?.status).toBe("running");
    expect(s.busy).toBe(false);
  });

  it("rethrows a busy error from restart and clears busy", async () => {
    const busy = { code: "ENGINE_BUSY", message: "x", details: { running: 1, queued: 2 } };
    restart.mockRejectedValueOnce(busy);
    await expect(useEngineProcessStore.getState().restart()).rejects.toEqual(busy);
    expect(useEngineProcessStore.getState().busy).toBe(false);

    restart.mockResolvedValueOnce(running);
    await useEngineProcessStore.getState().restart(true);
    expect(restart).toHaveBeenLastCalledWith(true);
    expect(useEngineProcessStore.getState().state?.status).toBe("running");
  });

  it("keeps only the latest log lines", () => {
    for (let i = 0; i < LOG_LINES + 20; i++) useEngineProcessStore.getState().appendLog(`l${i}`);
    const logs = useEngineProcessStore.getState().logs;
    expect(logs).toHaveLength(LOG_LINES);
    expect(logs.at(-1)).toBe(`l${LOG_LINES + 19}`);
  });
});
