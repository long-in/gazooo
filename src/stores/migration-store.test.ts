import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MigrationPlan, MigrationReport, MigrationTarget } from "@/lib/tauri/storage";

const plan = vi.fn<(t: MigrationTarget, d: string | null) => Promise<MigrationPlan>>();
const start = vi.fn<(t: MigrationTarget, d: string | null) => Promise<MigrationReport>>();
const cancel = vi.fn<() => Promise<void>>();
vi.mock("@/lib/tauri/storage", () => ({
  storageMigrationPlan: (t: MigrationTarget, d: string | null) => plan(t, d),
  storageMigrationStart: (t: MigrationTarget, d: string | null) => start(t, d),
  storageMigrationCancel: () => cancel(),
}));

import { useMigrationStore } from "./migration-store";

const aPlan = (over: Partial<MigrationPlan> = {}): MigrationPlan => ({
  sourceDir: "/old",
  destDir: "/new",
  fileCount: 1,
  totalBytes: 10,
  copyBytes: 10,
  freeBytes: 100,
  sameSizeCount: 0,
  missing: 0,
  conflicts: [],
  skippedFolders: [],
  skippedLinks: 0,
  blockers: [],
  users: [],
  excludedUsers: 0,
  ...over,
});

const aReport: MigrationReport = {
  sourceDir: "/old",
  destDir: "/new",
  moved: 1,
  missing: 0,
  keptSameSize: 0,
  leftInSource: 0,
  leftDirs: [],
};

const state = () => useMigrationStore.getState();

describe("migration store", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    cancel.mockResolvedValue(undefined);
    useMigrationStore.setState({ step: "closed" });
    state().open("imagesUser");
  });

  it("goes from picking through confirmation to the result", async () => {
    plan.mockResolvedValue(aPlan());
    start.mockResolvedValue(aReport);
    expect(state().step).toBe("pick");
    await state().choose("/new");
    expect(state().step).toBe("confirm");
    await expect(state().start()).resolves.toEqual(aReport);
    expect(start).toHaveBeenCalledWith("imagesUser", "/new");
    expect(state().step).toBe("done");
  });

  it("does not start while there is a blocker", async () => {
    plan.mockResolvedValue(aPlan({ blockers: ["NO_SPACE"] }));
    await state().choose("/new");
    await expect(state().start()).resolves.toBeNull();
    expect(start).not.toHaveBeenCalled();
    expect(state().step).toBe("confirm");
  });

  it("cannot be closed or reopened while running, and can cancel", async () => {
    plan.mockResolvedValue(aPlan());
    let fail: (e: unknown) => void = () => {};
    start.mockReturnValue(new Promise((_, reject) => (fail = reject)));
    await state().choose("/new");
    const running = state().start();
    state().close();
    state().open("models");
    expect(state().step).toBe("running");
    expect(state().target).toBe("imagesUser");
    await state().cancel();
    expect(cancel).toHaveBeenCalled();
    fail({ code: "STORAGE_CANCELLED", message: "cancelled" });
    await running;
    expect(state().step).toBe("confirm");
    expect(state().error?.code).toBe("STORAGE_CANCELLED");
    expect(state().cancelling).toBe(false);
  });

  it("returns to picking when the check fails", async () => {
    plan.mockRejectedValue({ code: "STORAGE_BUSY", message: "busy" });
    await state().choose("/new");
    expect(state().step).toBe("pick");
    expect(state().error?.code).toBe("STORAGE_BUSY");
  });

  it("ignores a check that finishes after the dialog was closed", async () => {
    let done: (p: MigrationPlan) => void = () => {};
    plan.mockReturnValue(new Promise((resolve) => (done = resolve)));
    const pending = state().choose("/new");
    state().close();
    done(aPlan());
    await pending;
    expect(state().step).toBe("closed");
    expect(state().plan).toBeNull();
  });
});
