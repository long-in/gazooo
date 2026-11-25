import { beforeEach, describe, expect, it, vi } from "vitest";

import { defaultParams } from "@/lib/generation/defaults";
import type { Preset } from "@/lib/tauri/presets";

const presetList = vi.fn<() => Promise<Preset[]>>();
const presetSave = vi.fn<(a: { id?: string; name: string }) => Promise<Preset>>();
const presetDelete = vi.fn<(id: string) => Promise<void>>();
vi.mock("@/lib/tauri/presets", () => ({
  presetList: () => presetList(),
  presetSave: (a: { id?: string; name: string }) => presetSave(a),
  presetDelete: (id: string) => presetDelete(id),
}));

import { usePresetStore } from "./preset-store";

const preset = (id: string, name: string): Preset => ({
  id,
  name,
  params: defaultParams("sdxl"),
  createdAt: 0,
  updatedAt: 0,
});

beforeEach(() => {
  presetList.mockReset();
  presetSave.mockReset();
  presetDelete.mockReset().mockResolvedValue();
  usePresetStore.setState({ presets: [], loaded: false });
});

describe("preset-store", () => {
  it("一覧を読み込む。失敗しても loaded になる", async () => {
    presetList.mockResolvedValueOnce([preset("1", "A")]);
    await usePresetStore.getState().load();
    expect(usePresetStore.getState()).toMatchObject({ loaded: true, presets: [expect.objectContaining({ id: "1" })] });

    presetList.mockRejectedValueOnce(new Error("x"));
    usePresetStore.setState({ loaded: false });
    await usePresetStore.getState().load();
    expect(usePresetStore.getState().loaded).toBe(true);
  });

  it("保存すると名前順(大文字小文字を区別しない)に並び、同じ id は置き換わる", async () => {
    usePresetStore.setState({ presets: [preset("1", "banana")] });
    presetSave.mockResolvedValueOnce(preset("2", "Apple"));
    await usePresetStore.getState().save({ name: "Apple", params: defaultParams("sdxl") });
    expect(usePresetStore.getState().presets.map((p) => p.name)).toEqual(["Apple", "banana"]);

    presetSave.mockResolvedValueOnce(preset("1", "cherry"));
    await usePresetStore.getState().save({ id: "1", name: "cherry", params: defaultParams("sdxl") });
    expect(usePresetStore.getState().presets.map((p) => p.name)).toEqual(["Apple", "cherry"]);
  });

  it("保存に失敗したら一覧は変えず、エラーを伝える", async () => {
    usePresetStore.setState({ presets: [preset("1", "A")] });
    presetSave.mockRejectedValueOnce({ code: "VALIDATION", message: "dup", details: { field: "name" } });
    await expect(usePresetStore.getState().save({ name: "A", params: defaultParams("sdxl") })).rejects.toMatchObject({
      code: "VALIDATION",
    });
    expect(usePresetStore.getState().presets).toHaveLength(1);
  });

  it("削除は IPC 成功後に一覧から外す。失敗なら残す", async () => {
    usePresetStore.setState({ presets: [preset("1", "A"), preset("2", "B")] });
    await usePresetStore.getState().remove("1");
    expect(usePresetStore.getState().presets.map((p) => p.id)).toEqual(["2"]);

    presetDelete.mockRejectedValueOnce({ code: "PRESET_NOT_FOUND", message: "x" });
    await expect(usePresetStore.getState().remove("2")).rejects.toBeTruthy();
    expect(usePresetStore.getState().presets).toHaveLength(1);
  });
});
