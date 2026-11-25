import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CatalogView, DownloadState } from "@/lib/tauri/models";

const catalog = vi.fn<() => Promise<CatalogView>>();
const list = vi.fn<() => Promise<DownloadState[]>>();
const start = vi.fn();
const loadEngineModels = vi.fn();

vi.mock("@/lib/tauri/models", () => ({
  modelsCatalog: () => catalog(),
  modelsDownloadList: () => list(),
  modelsDownloadStart: (...a: unknown[]) => start(...a),
  modelsDownloadCancel: vi.fn(),
  modelsSetDir: vi.fn().mockResolvedValue({ modelsDir: "/m", source: "setting" }),
  modelsVerify: vi.fn(),
}));
vi.mock("@/stores/engine-store", () => ({
  useEngineStore: { getState: () => ({ loadModels: loadEngineModels }) },
}));

import { useModelsStore } from "./models-store";

const VIEW: CatalogView = {
  catalog: { schemaVersion: 1, families: [] },
  installed: [],
  modelsDir: "/m",
  modelsDirSource: "detected",
  freeBytes: 1e12,
};

const dl = (over: Partial<DownloadState> = {}): DownloadState => ({
  downloadId: "d1",
  familyId: "controlnet-canny",
  variantId: "sdxl-xinsir",
  status: "downloading",
  receivedBytes: 10,
  totalBytes: 100,
  ...over,
});

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  catalog.mockReset();
  catalog.mockResolvedValue(VIEW);
  list.mockReset();
  list.mockResolvedValue([]);
  start.mockReset();
  start.mockResolvedValue({ downloadId: "d1" });
  loadEngineModels.mockReset();
  useModelsStore.setState({
    view: null,
    loading: false,
    error: null,
    downloads: [],
    category: "checkpoint",
    base: "all",
    chosen: {},
  });
});

describe("models-store", () => {
  it("load はカタログを保持し、失敗はエラーとして保持する", async () => {
    await useModelsStore.getState().load();
    expect(useModelsStore.getState().view).toEqual(VIEW);

    catalog.mockRejectedValueOnce({ code: "INTERNAL", message: "x" });
    await useModelsStore.getState().load();
    expect(useModelsStore.getState().error?.code).toBe("INTERNAL");
  });

  it("進捗イベントは同じ ID を置き換え、新しい ID は追加する", () => {
    const { applyDownload } = useModelsStore.getState();
    applyDownload(dl());
    applyDownload(dl({ receivedBytes: 50 }));
    applyDownload(dl({ downloadId: "d2", familyId: "other" }));
    const ds = useModelsStore.getState().downloads;
    expect(ds).toHaveLength(2);
    expect(ds.find((d) => d.downloadId === "d1")?.receivedBytes).toBe(50);
  });

  it("同じモデルを再取得すると、終了済みの過去の結果は置き換わる", () => {
    const { applyDownload } = useModelsStore.getState();
    applyDownload(dl({ status: "failed", error: { code: "MODELS_DOWNLOAD_FAILED", message: "x" } }));
    applyDownload(dl({ downloadId: "d2", status: "queued" }));
    const ds = useModelsStore.getState().downloads;
    expect(ds.map((d) => d.downloadId)).toEqual(["d2"]);
  });

  it("完了したら導入状況とエンジンのモデル一覧を取り直す(1 回だけ)", async () => {
    const { applyDownload } = useModelsStore.getState();
    applyDownload(dl({ status: "done", receivedBytes: 100 }));
    await flush();
    expect(catalog).toHaveBeenCalledTimes(1);
    expect(loadEngineModels).toHaveBeenCalledTimes(1);

    // 同じ完了通知が再度届いても取り直さない
    applyDownload(dl({ status: "done", receivedBytes: 100 }));
    await flush();
    expect(catalog).toHaveBeenCalledTimes(1);
    expect(loadEngineModels).toHaveBeenCalledTimes(1);
  });

  it("途中の状態では取り直さない", async () => {
    useModelsStore.getState().applyDownload(dl());
    await flush();
    expect(catalog).not.toHaveBeenCalled();
    expect(loadEngineModels).not.toHaveBeenCalled();
  });

  it("start は同意の指定を渡し、一覧を取り直す。失敗は呼び出し側へ投げる", async () => {
    list.mockResolvedValue([dl({ status: "queued" })]);
    await useModelsStore.getState().start("controlnet-canny", "sdxl-xinsir", true);
    expect(start).toHaveBeenCalledWith("controlnet-canny", "sdxl-xinsir", true);
    expect(useModelsStore.getState().downloads).toHaveLength(1);

    start.mockRejectedValueOnce({ code: "MODELS_NO_CONSENT", message: "x" });
    await expect(useModelsStore.getState().start("a", "b")).rejects.toMatchObject({
      code: "MODELS_NO_CONSENT",
    });
  });

  it("一覧の取得は、先に届いたイベントの新しい状態を古い値で上書きしない", async () => {
    useModelsStore.getState().applyDownload(dl({ receivedBytes: 90 }));
    list.mockResolvedValue([dl({ receivedBytes: 10 })]);
    await useModelsStore.getState().loadDownloads();
    expect(useModelsStore.getState().downloads[0].receivedBytes).toBe(90);
  });

  it("variant の選択・カテゴリ・ベースモデルを保持する", () => {
    const s = useModelsStore.getState();
    s.choose("controlnet-canny", "sd15");
    s.setCategory("controlnet");
    s.setBase("sdxl");
    const now = useModelsStore.getState();
    expect(now.chosen).toEqual({ "controlnet-canny": "sd15" });
    expect(now.category).toBe("controlnet");
    expect(now.base).toBe("sdxl");
  });
});
