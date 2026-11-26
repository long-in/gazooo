import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SettingsGroup } from "@/lib/tauri/settings";

const get = vi.fn<() => Promise<SettingsGroup[]>>();
const setOverride = vi.fn<(g: SettingsGroup, on: boolean) => Promise<void>>();
const reset = vi.fn<(g: SettingsGroup) => Promise<void>>();
const reload = vi.fn<() => Promise<void>>();
vi.mock("@/lib/tauri/settings", () => ({
  settingsScopeGet: () => get(),
  settingsScopeSet: (g: SettingsGroup, on: boolean) => setOverride(g, on),
  settingsScopeResetGlobal: (g: SettingsGroup) => reset(g),
}));
vi.mock("@/stores/settings-store", () => ({ useSettingsStore: { getState: () => ({ load: () => reload() }) } }));

import { useSettingsScopeStore } from "./settings-scope-store";

const state = () => useSettingsScopeStore.getState();

describe("settings scope store", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    useSettingsScopeStore.setState({ overridden: [], revision: 0 });
  });

  it("reloads the values after switching between global and own settings", async () => {
    get.mockResolvedValue(["theme"]);
    await state().setOverride("theme", true);
    expect(setOverride).toHaveBeenCalledWith("theme", true);
    expect(state().overridden).toEqual(["theme"]);
    expect(reload).toHaveBeenCalled();
    expect(state().revision).toBe(1);

    get.mockResolvedValue([]);
    await state().resetGlobal("theme");
    expect(reset).toHaveBeenCalledWith("theme");
    expect(state().revision).toBe(2);
  });

  it("keeps the current state when switching fails", async () => {
    setOverride.mockRejectedValue({ code: "STORAGE_BUSY", message: "busy" });
    await expect(state().setOverride("theme", true)).rejects.toMatchObject({ code: "STORAGE_BUSY" });
    expect(state().revision).toBe(0);
  });
});
