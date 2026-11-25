import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DebugStatus } from "@/lib/tauri/debug";

const api = vi.hoisted(() => ({
  debugStatus: vi.fn(),
  debugSetEnabled: vi.fn(),
  debugExportLogs: vi.fn(),
  debugOpenLogDir: vi.fn(),
  debugClearLogs: vi.fn(),
}));
vi.mock("@/lib/tauri/debug", () => api);

import { useDebugStore } from "./debug-store";

const status = (over: Partial<DebugStatus> = {}): DebugStatus => ({
  enabled: false,
  logDir: "C:\logs",
  fileCount: 3,
  totalBytes: 1000,
  limitBytes: 1_073_741_824,
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  useDebugStore.setState({ status: null, busy: false });
});

describe("debug store", () => {
  it("loads the status, and stays empty when it cannot be read", async () => {
    api.debugStatus.mockResolvedValueOnce(status());
    await useDebugStore.getState().load();
    expect(useDebugStore.getState().status?.fileCount).toBe(3);

    useDebugStore.setState({ status: null });
    api.debugStatus.mockRejectedValueOnce(new Error("no tauri"));
    await useDebugStore.getState().load();
    expect(useDebugStore.getState().status).toBeNull();
  });

  it("toggling calls the command and reflects the returned status", async () => {
    api.debugSetEnabled.mockResolvedValueOnce(status({ enabled: true }));
    await useDebugStore.getState().setEnabled(true);
    expect(api.debugSetEnabled).toHaveBeenCalledWith(true);
    expect(useDebugStore.getState().status?.enabled).toBe(true);
    expect(useDebugStore.getState().busy).toBe(false);
  });

  it("a failed toggle throws, keeps the old status and clears busy", async () => {
    useDebugStore.setState({ status: status() });
    api.debugSetEnabled.mockRejectedValueOnce({ code: "INTERNAL", message: "x" });
    await expect(useDebugStore.getState().setEnabled(true)).rejects.toBeTruthy();
    expect(useDebugStore.getState().status?.enabled).toBe(false);
    expect(useDebugStore.getState().busy).toBe(false);
  });

  it("export returns the saved path, or null when the dialog is cancelled", async () => {
    api.debugExportLogs.mockResolvedValueOnce({ path: "C:\out.zip" });
    api.debugStatus.mockResolvedValue(status());
    expect(await useDebugStore.getState().exportLogs()).toBe("C:\out.zip");
    api.debugExportLogs.mockResolvedValueOnce(null);
    expect(await useDebugStore.getState().exportLogs()).toBeNull();
  });

  it("clear reflects the new status", async () => {
    api.debugClearLogs.mockResolvedValueOnce(status({ fileCount: 1, totalBytes: 0 }));
    await useDebugStore.getState().clear();
    expect(useDebugStore.getState().status?.totalBytes).toBe(0);
  });
});
