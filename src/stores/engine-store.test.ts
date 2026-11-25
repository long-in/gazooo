import { beforeEach, describe, expect, it, vi } from "vitest";

import type { EngineModels, EngineStatus } from "@/lib/tauri/engine";

const listModels = vi.fn<() => Promise<EngineModels>>();
vi.mock("@/lib/tauri/engine", () => ({
  engineStatus: vi.fn(),
  engineReconnect: vi.fn(),
  engineListModels: () => listModels(),
}));

import { useEngineStore } from "./engine-store";

const MODELS: EngineModels = {
  checkpoints: ["sdxl.safetensors"],
  unets: [],
  ggufUnets: [],
  clips: [],
  vaes: [],
  loras: [],
  faceDetectors: [],
  controlnets: [],
  samplers: ["euler"],
  schedulers: ["simple"],
  taggers: [],
};

const status = (over: Partial<EngineStatus> = {}): EngineStatus => ({
  connected: true,
  url: "http://127.0.0.1:8188",
  features: { ggufNode: false, faceDetailer: false, controlnetPreprocessors: [], inpaint: false, img2img: false, referenceEdit: false, faceSwap: false, faceSwapComposite: false, imageTag: false },
  ...over,
});

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  listModels.mockReset();
  listModels.mockResolvedValue(MODELS);
  useEngineStore.setState({
    status: null,
    models: null,
    modelsLoading: false,
    modelsError: null,
    reconnecting: false,
  });
});

describe("engine-store", () => {
  it("接続が確立したらモデル一覧を取得する", async () => {
    useEngineStore.getState().applyStatus(status());
    await flush();
    expect(listModels).toHaveBeenCalledTimes(1);
    expect(useEngineStore.getState().models).toEqual(MODELS);
  });

  it("接続中の同じ状態の再通知では再取得しない", async () => {
    useEngineStore.getState().applyStatus(status());
    await flush();
    useEngineStore.getState().applyStatus(status({ vramFree: 1 }));
    await flush();
    expect(listModels).toHaveBeenCalledTimes(1);
  });

  it("切断でモデル一覧をクリアし、再接続で取り直す", async () => {
    const { applyStatus } = useEngineStore.getState();
    applyStatus(status());
    await flush();
    applyStatus(status({ connected: false, error: "unreachable" }));
    expect(useEngineStore.getState().models).toBeNull();
    applyStatus(status());
    await flush();
    expect(listModels).toHaveBeenCalledTimes(2);
  });

  it("接続先 URL が変わったら取り直す", async () => {
    const { applyStatus } = useEngineStore.getState();
    applyStatus(status());
    await flush();
    applyStatus(status({ url: "http://localhost:9999" }));
    await flush();
    expect(listModels).toHaveBeenCalledTimes(2);
  });

  it("モデル取得に失敗したらエラーを保持する", async () => {
    listModels.mockRejectedValue({ code: "ENGINE_UNREACHABLE", message: "x" });
    useEngineStore.getState().applyStatus(status());
    await flush();
    const s = useEngineStore.getState();
    expect(s.modelsError?.code).toBe("ENGINE_UNREACHABLE");
    expect(s.modelsLoading).toBe(false);
  });
});
