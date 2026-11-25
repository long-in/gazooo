import { beforeEach, describe, expect, it, vi } from "vitest";

import type { UserSummary } from "@/lib/tauri/user";

const api = vi.hoisted(() => ({
  userList: vi.fn(),
  userCurrent: vi.fn(),
  userCreate: vi.fn(),
  userLogin: vi.fn(),
  userLogout: vi.fn(),
  userRename: vi.fn(),
  userDelete: vi.fn(),
  userLockSet: vi.fn(),
  userLockClear: vi.fn(),
}));
vi.mock("@/lib/tauri/user", () => api);

import { reloadWindow, useUserStore } from "./user-store";

const user = (over: Partial<UserSummary> = {}): UserSummary => ({
  id: "u1",
  name: "Taro",
  color: "blue",
  authKind: "none",
  pinLength: null,
  lockedUntil: null,
  lastLoginAt: null,
  ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  api.userList.mockResolvedValue([user()]);
  api.userCurrent.mockResolvedValue(null);
  useUserStore.setState({
    phase: "loading",
    users: [],
    current: null,
    loadError: null,
  });
});

describe("user store", () => {
  it("shows the gate at start when nobody is signed in", async () => {
    await useUserStore.getState().init();
    const s = useUserStore.getState();
    expect(s.phase).toBe("gate");
    expect(s.users).toHaveLength(1);
  });

  it("skips the gate when a user is already signed in", async () => {
    api.userCurrent.mockResolvedValue(user());
    await useUserStore.getState().init();
    expect(useUserStore.getState().phase).toBe("in");
  });

  it("falls back to the gate when loading fails", async () => {
    api.userList.mockRejectedValue({ code: "INTERNAL", message: "x" });
    await useUserStore.getState().init();
    const s = useUserStore.getState();
    expect(s.phase).toBe("gate");
    expect(s.loadError?.code).toBe("INTERNAL");
  });

  it("enters the app after a successful login", async () => {
    api.userLogin.mockResolvedValue(user());
    await useUserStore.getState().login("u1", "1234");
    expect(api.userLogin).toHaveBeenCalledWith("u1", "1234");
    const s = useUserStore.getState();
    expect(s.phase).toBe("in");
    expect(s.current?.id).toBe("u1");
  });

  it("stays on the gate and refreshes the wait after a failed login", async () => {
    const err = { code: "AUTH_FAILED", message: "no" };
    api.userLogin.mockRejectedValue(err);
    api.userList.mockResolvedValue([user({ lockedUntil: 99 })]);
    await expect(useUserStore.getState().login("u1", "0000")).rejects.toEqual(
      err,
    );
    await vi.waitFor(() =>
      expect(useUserStore.getState().users[0]?.lockedUntil).toBe(99),
    );
    expect(useUserStore.getState().phase).toBe("loading");
  });

  it("logs out and reloads the window to drop all in-memory state", async () => {
    const reload = vi.spyOn(reloadWindow, "run").mockImplementation(() => {});
    api.userLogout.mockResolvedValue(undefined);
    await useUserStore.getState().switchUser(true);
    expect(api.userLogout).toHaveBeenCalledWith(true);
    expect(reload).toHaveBeenCalledOnce();
  });

  it("does not reload when logout is refused because a job is running", async () => {
    const reload = vi.spyOn(reloadWindow, "run").mockImplementation(() => {});
    api.userLogout.mockRejectedValue({ code: "USER_BUSY", message: "busy" });
    await expect(useUserStore.getState().switchUser(false)).rejects.toMatchObject(
      { code: "USER_BUSY" },
    );
    expect(reload).not.toHaveBeenCalled();
  });

  it("updates the current user after changing the lock", async () => {
    api.userCurrent.mockResolvedValue(user());
    await useUserStore.getState().init();
    api.userLockSet.mockResolvedValue(user({ authKind: "pin", pinLength: 4 }));
    await useUserStore
      .getState()
      .setLock({ kind: "pin", secret: "1234", pinLength: 4 });
    expect(useUserStore.getState().current?.authKind).toBe("pin");
  });

  it("returns an external images folder reported by a delete", async () => {
    api.userDelete.mockResolvedValue({ externalImagesDir: "D:\\img" });
    const dir = await useUserStore.getState().remove("u2", { secret: "x" });
    expect(dir).toBe("D:\\img");
    expect(api.userDelete).toHaveBeenCalledWith("u2", { secret: "x" });
  });
});
