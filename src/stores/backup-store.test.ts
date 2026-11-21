import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BackupEstimate } from "@/lib/tauri/backup";

const api = vi.hoisted(() => ({
  backupEstimate: vi.fn(),
  backupExport: vi.fn(),
  backupInspect: vi.fn(),
  backupOrphans: vi.fn(),
  backupRestore: vi.fn(),
  backupCancel: vi.fn(),
}));
vi.mock("@/lib/tauri/backup", () => api);

import { canExport, useBackupStore } from "./backup-store";

const estimate = (over: Partial<BackupEstimate> = {}): BackupEstimate => ({
  settings: 10,
  conversations: 2,
  jobs: 5,
  images: 3,
  presets: 1,
  bytes: 1000,
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  api.backupEstimate.mockResolvedValue(estimate());
  useBackupStore.setState({
    include: { settings: true, gallery: true, presets: true },
    includeApiKey: false,
    estimate: null,
    busy: null,
    progress: null,
  });
});

describe("backup store", () => {
  it("includes all three sections by default and leaves the API key out", () => {
    const s = useBackupStore.getState();
    expect(s.include).toEqual({ settings: true, gallery: true, presets: true });
    expect(s.includeApiKey).toBe(false);
  });

  it("passes the chosen sections to the export and reports a cancel as null", async () => {
    useBackupStore.getState().setInclude("gallery", false);
    api.backupExport.mockResolvedValueOnce(null);
    expect(await useBackupStore.getState().exportBackup()).toBeNull();
    expect(api.backupExport).toHaveBeenCalledWith(
      { settings: true, gallery: false, presets: true },
      false,
    );
    expect(useBackupStore.getState().busy).toBeNull();
  });

  it("sends the API key choice only while settings are included", async () => {
    api.backupExport.mockResolvedValue({
      path: "x",
      images: 0,
      skipped: 0,
      bytes: 0,
    });
    useBackupStore.getState().setIncludeApiKey(true);
    await useBackupStore.getState().exportBackup();
    expect(api.backupExport).toHaveBeenLastCalledWith(expect.anything(), true);

    // 設定値を外すと API キーの選択も外れ、再び ON にもできない
    useBackupStore.getState().setInclude("settings", false);
    expect(useBackupStore.getState().includeApiKey).toBe(false);
    useBackupStore.getState().setIncludeApiKey(true);
    expect(useBackupStore.getState().includeApiKey).toBe(false);
  });

  it("does not export when every section is off", async () => {
    const s = useBackupStore.getState();
    s.setInclude("settings", false);
    s.setInclude("gallery", false);
    s.setInclude("presets", false);
    expect(canExport(useBackupStore.getState().include)).toBe(false);
    expect(await useBackupStore.getState().exportBackup()).toBeNull();
    expect(api.backupExport).not.toHaveBeenCalled();
  });

  it("reloads the estimate when a section is toggled, and ignores failures", async () => {
    useBackupStore.getState().setInclude("gallery", false);
    await vi.waitFor(() =>
      expect(useBackupStore.getState().estimate?.images).toBe(3),
    );
    expect(api.backupEstimate).toHaveBeenCalledWith({
      settings: true,
      gallery: false,
      presets: true,
    });

    useBackupStore.setState({ estimate: null });
    api.backupEstimate.mockRejectedValueOnce(new Error("no tauri"));
    await useBackupStore.getState().loadEstimate();
    expect(useBackupStore.getState().estimate).toBeNull();
  });

  it("marks itself busy during a restore, even when it fails", async () => {
    api.backupRestore.mockRejectedValueOnce({
      code: "BACKUP_BUSY",
      message: "busy",
    });
    const p = useBackupStore.getState().restore({
      include: { settings: true, gallery: false, presets: false },
      restoreApiKey: false,
      stopJobs: true,
      deleteOrphans: false,
    });
    expect(useBackupStore.getState().busy).toBe("restore");
    await expect(p).rejects.toMatchObject({ code: "BACKUP_BUSY" });
    expect(useBackupStore.getState().busy).toBeNull();
  });

  it("inspects a file and returns null when the dialog is cancelled", async () => {
    api.backupInspect.mockResolvedValueOnce(null);
    expect(await useBackupStore.getState().inspect()).toBeNull();
    expect(useBackupStore.getState().busy).toBeNull();
  });
});
