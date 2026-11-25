import { beforeEach, describe, expect, it } from "vitest";

import { defaultParams } from "@/lib/generation/defaults";

import { useFormStore } from "./form-store";

beforeEach(() => {
  useFormStore.setState({ params: defaultParams("sdxl"), count: 1, hydrated: false });
});

describe("form-store.loadParams", () => {
  it("画像のパラメータを読み込み、シードは固定にして復元済みにする", () => {
    const saved = { ...defaultParams("flux_gguf"), prompt: "a cat", seed: 123, seedRandomize: true };
    useFormStore.getState().loadParams(saved);
    const s = useFormStore.getState();
    expect(s.params).toMatchObject({ prompt: "a cat", seed: 123, seedRandomize: false });
    expect(s.params.model.kind).toBe("flux_gguf");
    expect(s.hydrated).toBe(true);
  });

  it("読み込み後は、設定からの初回復元で上書きされない", () => {
    useFormStore.getState().loadParams({ ...defaultParams("sd15"), prompt: "kept" });
    useFormStore.getState().hydrate({ prompt: "from settings" });
    expect(useFormStore.getState().params.prompt).toBe("kept");
  });

  it("欠けた項目は既定値で補う", () => {
    useFormStore.getState().loadParams({ prompt: "x", model: { kind: "sd15", name: "m", clip: [], vae: null } });
    expect(useFormStore.getState().params).toMatchObject({ width: 512, steps: 20 });
  });
});

describe("form-store.applyPreset", () => {
  it("シードのランダム設定を含めてそのまま復元する(loadParams との違い)", () => {
    const preset = { ...defaultParams("sd15"), prompt: "p", seedRandomize: true, seed: 5 };
    useFormStore.getState().applyPreset(preset);
    expect(useFormStore.getState().params).toMatchObject({ prompt: "p", seedRandomize: true, seed: 5 });
    expect(useFormStore.getState().hydrated).toBe(true);
  });
});

describe("form-store.patchFace", () => {
  it("未設定なら既定値から作って更新する", () => {
    useFormStore.getState().loadParams(defaultParams("sd15"));
    expect(useFormStore.getState().params.faceDetailer).toBeUndefined();
    useFormStore.getState().patchFace({ enabled: true, denoise: 0.55 });
    expect(useFormStore.getState().params.faceDetailer).toEqual({
      enabled: true,
      detector: "",
      denoise: 0.55,
      prompt: "",
      confidence: 0.5,
    });
  });

  it("一部だけ更新し、ほかの項目は保つ", () => {
    useFormStore.getState().loadParams(defaultParams("sd15"));
    useFormStore.getState().patchFace({ enabled: true, denoise: 0.55 });
    useFormStore.getState().patchFace({ prompt: "detailed eyes" });
    useFormStore.getState().patchFace({ confidence: 0.3 });
    expect(useFormStore.getState().params.faceDetailer).toMatchObject({
      enabled: true,
      denoise: 0.55,
      prompt: "detailed eyes",
      confidence: 0.3,
    });
  });
});

describe("form-store: ControlNet", () => {
  const CNS = ["control_canny.safetensors", "control_depth.safetensors"];

  it("ユニットが未設定なら既定から作り、前処理を変えると合うモデルへ切り替える", () => {
    useFormStore.getState().loadParams(defaultParams("sd15"));
    useFormStore.getState().patchControlnetUnit({ image: "inputs/a.png" }, CNS);
    expect(useFormStore.getState().params.controlnet).toEqual({
      enabled: false,
      units: [
        { image: "inputs/a.png", model: "control_canny.safetensors", preprocessor: "canny", strength: 1, startPercent: 0, endPercent: 1 },
      ],
    });
    useFormStore.getState().patchControlnetUnit({ preprocessor: "depth" }, CNS);
    expect(useFormStore.getState().params.controlnet?.units[0]).toMatchObject({
      preprocessor: "depth",
      model: "control_depth.safetensors",
      image: "inputs/a.png",
    });
    // モデルを直接選んだときは、前処理に合わせて勝手に戻さない
    useFormStore.getState().patchControlnetUnit({ model: "control_canny.safetensors" }, CNS);
    expect(useFormStore.getState().params.controlnet?.units[0].model).toBe("control_canny.safetensors");
  });

  it("有効・無効の切り替えで、ユニットの設定は残る", () => {
    useFormStore.getState().loadParams(defaultParams("sd15"));
    useFormStore.getState().patchControlnetUnit({ image: "inputs/a.png", strength: 0.6 }, CNS);
    useFormStore.getState().setControlnetEnabled(true);
    useFormStore.getState().setControlnetEnabled(false);
    expect(useFormStore.getState().params.controlnet).toMatchObject({
      enabled: false,
      units: [{ image: "inputs/a.png", strength: 0.6 }],
    });
  });

  it("setReferenceImage: 画像を設定して有効にする。Flux では有効にしない", () => {
    useFormStore.getState().loadParams(defaultParams("sdxl"));
    expect(useFormStore.getState().setReferenceImage("inputs/g.png", CNS)).toBe(true);
    expect(useFormStore.getState().params.controlnet).toMatchObject({
      enabled: true,
      units: [{ image: "inputs/g.png", model: "control_canny.safetensors" }],
    });

    useFormStore.getState().loadParams(defaultParams("flux"));
    expect(useFormStore.getState().setReferenceImage("inputs/g.png", CNS)).toBe(false);
    expect(useFormStore.getState().params.controlnet).toMatchObject({
      enabled: false,
      units: [{ image: "inputs/g.png" }],
    });
  });
});

describe("applySuggestion / undoSuggestion", () => {
  it("applies usable changes, then undoes only those fields", () => {
    useFormStore.setState({ params: { ...defaultParams("sd15"), steps: 20, width: 512 }, suggestionUndo: null });
    const models = {
      checkpoints: [],
      unets: [],
      ggufUnets: [],
      clips: [],
      vaes: [],
      loras: [],
      faceDetectors: [],
      controlnets: [],
      samplers: ["euler"],
      schedulers: ["normal"],
      taggers: [],
    };
    const ok = useFormStore.getState().applySuggestion({ steps: 40, sampler: "bad" }, models);
    expect(ok).toBe(true);
    expect(useFormStore.getState().params.steps).toBe(40);
    expect(useFormStore.getState().params.sampler).toBe("euler");

    useFormStore.getState().patch({ width: 768 }); // 適用後の手動編集
    useFormStore.getState().undoSuggestion();
    expect(useFormStore.getState().params.steps).toBe(20);
    expect(useFormStore.getState().params.width).toBe(768);
    expect(useFormStore.getState().suggestionUndo).toBeNull();
  });

  it("returns false and changes nothing when nothing is usable", () => {
    useFormStore.setState({ params: defaultParams("flux"), suggestionUndo: null });
    expect(useFormStore.getState().applySuggestion({ cfg: 7 }, null)).toBe(false);
    expect(useFormStore.getState().suggestionUndo).toBeNull();
  });
});

describe("inpaint", () => {
  beforeEach(() => {
    useFormStore.setState({ params: defaultParams("sdxl"), suggestionUndo: null });
  });

  it("setInpaintImage adopts the image size (64-multiples) and drops the previous mask", () => {
    const s = useFormStore.getState();
    s.patchInpaint({ mask: "inputs/old-mask.png" });
    s.setInpaintImage("inputs/a.png", 1000, 600);
    const p = useFormStore.getState().params;
    expect([p.width, p.height]).toEqual([1024, 576]);
    expect(p.inpaint).toMatchObject({ image: "inputs/a.png", mask: "" });
  });

  it("enabling inpaint turns face fix off (they cannot be combined)", () => {
    const s = useFormStore.getState();
    s.patchFace({ enabled: true, detector: "bbox/face.pt" });
    s.setInpaintEnabled(true);
    const p = useFormStore.getState().params;
    expect(p.inpaint?.enabled).toBe(true);
    expect(p.faceDetailer).toMatchObject({ enabled: false, detector: "bbox/face.pt" });
  });

  it("switching to Flux disables inpaint but keeps its settings", () => {
    const s = useFormStore.getState();
    s.setInpaintImage("inputs/a.png", 512, 512);
    s.patchInpaint({ mask: "inputs/m.png" });
    s.setInpaintEnabled(true);
    s.setKind("flux");
    expect(useFormStore.getState().params.inpaint).toMatchObject({
      enabled: false,
      image: "inputs/a.png",
      mask: "inputs/m.png",
    });
  });

  it("restores saved inpaint settings over the defaults", () => {
    useFormStore.getState().applyPreset({ ...defaultParams("sdxl"), inpaint: { enabled: true, image: "inputs/a.png" } as never });
    expect(useFormStore.getState().params.inpaint).toEqual({
      enabled: true,
      image: "inputs/a.png",
      mask: "",
      denoise: 0.7,
      growMask: 8,
      feather: 8,
      compositeBack: true,
    });
  });
});

describe("form-store.applyBuiltinPreset(docs/16 §5)", () => {
  const models = {
    checkpoints: ["NoobAI-XL-v1.1.safetensors"],
    ggufs: [],
    clips: [],
    vaes: [],
    samplers: ["euler", "euler_ancestral"],
    schedulers: ["normal", "karras"],
    taggers: [],
    controlnets: [],
    faceDetectors: [],
  } as unknown as Parameters<ReturnType<typeof useFormStore.getState>["applyBuiltinPreset"]>[1];

  it("プロンプトを保ち、サイズ・ステップ・サンプラーを反映し、空のモデルは一覧から補う", async () => {
    const { BUILTIN_PRESETS } = await import("@/lib/generation/builtin-presets");
    useFormStore.setState({ params: { ...defaultParams("sd15"), prompt: "1girl" }, hydrated: false });
    useFormStore.getState().applyBuiltinPreset(BUILTIN_PRESETS[0], models);
    const { params, hydrated } = useFormStore.getState();
    expect(hydrated).toBe(true);
    expect(params.prompt).toBe("1girl");
    expect(params.model.kind).toBe("sdxl");
    expect(params.model.name).toBe("NoobAI-XL-v1.1.safetensors");
    expect([params.width, params.height, params.steps, params.sampler]).toEqual([832, 1216, 28, "euler_ancestral"]);
  });
});

describe("form-store: モデルの説明・AI のモデル提案(docs/18)", () => {
  const engineModels = {
    checkpoints: ["NoobAI-XL-v1.1.safetensors"],
    unets: [],
    ggufUnets: [],
    clips: [],
    vaes: [],
    loras: [],
    faceDetectors: [],
    controlnets: [],
    samplers: ["euler", "euler_ancestral", "dpmpp_2m"],
    schedulers: ["normal", "karras", "simple"],
    taggers: [],
  };

  it("推奨設定の適用: プロンプトとモデルの選択は変えない", () => {
    useFormStore.setState({
      params: { ...defaultParams("sdxl"), prompt: "1girl", model: { kind: "sdxl", name: "NoobAI-XL-v1.1.safetensors", clip: [], vae: null } },
    });
    useFormStore
      .getState()
      .applyRecommended({ width: 832, height: 1216, steps: 28, cfg: 5.5, sampler: "euler_ancestral", scheduler: "normal" }, engineModels);
    const { params } = useFormStore.getState();
    expect(params.prompt).toBe("1girl");
    expect(params.model.name).toBe("NoobAI-XL-v1.1.safetensors");
    expect([params.width, params.height, params.steps, params.cfg]).toEqual([832, 1216, 28, 5.5]);
    expect([params.sampler, params.scheduler]).toEqual(["euler_ancestral", "normal"]);
  });

  it("モデルの切り替え: 種別とモデル名が変わり、プロンプトとシードは引き継ぐ。使えない拡張機能は無効になる", () => {
    useFormStore.setState({
      params: {
        ...defaultParams("sd15"),
        prompt: "1girl, solo",
        seed: 7,
        seedRandomize: false,
        faceDetailer: { enabled: true, detector: "bbox/face.pt", denoise: 0.4, prompt: "", confidence: 0.5 },
      },
    });
    useFormStore.getState().switchModel("sdxl", "NoobAI-XL-v1.1.safetensors", engineModels);
    const { params } = useFormStore.getState();
    expect(params.model.kind).toBe("sdxl");
    expect(params.model.name).toBe("NoobAI-XL-v1.1.safetensors");
    expect(params.prompt).toBe("1girl, solo");
    expect([params.seed, params.seedRandomize]).toEqual([7, false]);
    expect(params.faceDetailer?.enabled).toBe(true); // SDXL は顔補正に対応
  });
});

describe("form-store.setWorkTab(docs/45)", () => {
  it("タブの切り替えで排他の機能が切り替わり、設定は保持される", () => {
    const s = () => useFormStore.getState();
    s().setWorkTab("img2img", true);
    expect(s().params.img2img?.enabled).toBe(true);
    s().setWorkTab("inpaint", true);
    expect([s().params.img2img?.enabled, s().params.inpaint?.enabled]).toEqual([false, true]);
    s().setWorkTab("text", false);
    expect([s().params.img2img?.enabled, s().params.inpaint?.enabled]).toEqual([false, false]);
    expect(s().params.img2img).toBeDefined(); // 設定は消さない
  });

  it("ControlNet は画像から・部分修正と併用でき、それ以外のタブへ移ると切れる", () => {
    const s = () => useFormStore.getState();
    s().setWorkTab("controlnet", true);
    expect(s().params.controlnet?.enabled).toBe(true);
    s().setWorkTab("img2img", true);
    expect([s().params.img2img?.enabled, s().params.controlnet?.enabled]).toEqual([true, true]);
    s().setWorkTab("inpaint", true);
    expect([s().params.inpaint?.enabled, s().params.controlnet?.enabled]).toEqual([true, true]);
    s().setWorkTab("controlnet", true);
    expect([s().params.inpaint?.enabled, s().params.controlnet?.enabled]).toEqual([false, true]);
    s().setWorkTab("text", false);
    expect(s().params.controlnet?.enabled).toBe(false);
    expect(s().workTab).toBe("text");
  });

  it("オンにできない(enable=false)ときは、他の機能を切るだけ", () => {
    const s = () => useFormStore.getState();
    s().setWorkTab("img2img", true);
    s().setWorkTab("inpaint", false);
    expect([s().params.img2img?.enabled, s().params.inpaint?.enabled]).toEqual([false, undefined]);
  });
});
