import { describe, expect, it } from "vitest";

import type { EngineStatus } from "@/lib/tauri/engine";
import type { GenerationParams } from "@/lib/tauri/jobs";

import { activeTabOf, availableTabs, canEnableTab, currentTab, tabForKind } from "./work-tab";

const status = (over: Partial<EngineStatus["features"]> = {}): EngineStatus => ({
  connected: true,
  url: "",
  features: {
    ggufNode: true,
    faceDetailer: true,
    controlnetPreprocessors: [],
    inpaint: true,
    img2img: true,
    referenceEdit: true,
    faceSwap: true,
    faceSwapComposite: true,
    imageTag: false,
    ...over,
  },
});

const p = (over: Partial<GenerationParams>) => over as GenerationParams;

describe("作業タブ(docs/45)", () => {
  it("モデル種別で出るタブが決まる(使えない機能は出さない)", () => {
    expect(availableTabs("sdxl")).toEqual(["text", "img2img", "inpaint", "controlnet"]);
    expect(availableTabs("sd15")).toEqual(["text", "img2img", "inpaint", "controlnet"]);
    expect(availableTabs("qwen_image")).toEqual(["text", "reference", "faceSwap"]);
    expect(availableTabs("flux_gguf")).toEqual([]);
    expect(availableTabs("anima")).toEqual([]);
    expect(availableTabs("krea2")).toEqual([]);
  });

  it("enabled フラグから今のタブを決める", () => {
    expect(activeTabOf(p({}))).toBe("text");
    expect(activeTabOf(p({ img2img: { enabled: true } as GenerationParams["img2img"] }))).toBe("img2img");
    expect(activeTabOf(p({ inpaint: { enabled: true } as GenerationParams["inpaint"] }))).toBe("inpaint");
    expect(activeTabOf(p({ reference: { enabled: true } as GenerationParams["reference"] }))).toBe("reference");
    expect(
      activeTabOf(
        p({
          reference: { enabled: true } as GenerationParams["reference"],
          faceSwap: { enabled: true } as GenerationParams["faceSwap"],
        }),
      ),
    ).toBe("faceSwap");
    expect(activeTabOf(p({ img2img: { enabled: false } as GenerationParams["img2img"] }))).toBe("text");
    // ControlNet 単独は「構図指定」。画像から・部分修正と併用しているときは、そちらのタブ
    expect(activeTabOf(p({ controlnet: { enabled: true } as GenerationParams["controlnet"] }))).toBe("controlnet");
    expect(
      activeTabOf(
        p({
          controlnet: { enabled: true } as GenerationParams["controlnet"],
          img2img: { enabled: true } as GenerationParams["img2img"],
        }),
      ),
    ).toBe("img2img");
  });

  it("画面に出すタブ: オンの機能が優先、すべてオフなら選んだタブ、使えなければ文章から", () => {
    const sdxl = (over: Partial<GenerationParams>) => p({ model: { kind: "sdxl" } as GenerationParams["model"], ...over });
    expect(currentTab(sdxl({}), "inpaint")).toBe("inpaint");
    expect(currentTab(sdxl({ img2img: { enabled: true } as GenerationParams["img2img"] }), "inpaint")).toBe("img2img");
    expect(currentTab(sdxl({}), "faceSwap")).toBe("text");
  });

  it("使えないタブは文章からへ戻る", () => {
    expect(tabForKind("inpaint", "sdxl")).toBe("inpaint");
    expect(tabForKind("inpaint", "qwen_image")).toBe("text");
    expect(tabForKind("faceSwap", "flux_gguf")).toBe("text");
  });

  it("ノード・モデルが無ければ、タブを選んでもオンにしない", () => {
    expect(canEnableTab("img2img", "sdxl", status())).toBe(true);
    expect(canEnableTab("img2img", "sdxl", status({ img2img: false }))).toBe(false);
    expect(canEnableTab("inpaint", "sdxl", status({ inpaint: false }))).toBe(false);
    expect(canEnableTab("faceSwap", "qwen_image", status({ faceSwap: false }))).toBe(false);
    expect(canEnableTab("reference", "qwen_image", status())).toBe(true);
    expect(canEnableTab("reference", "sdxl", status())).toBe(false);
    expect(canEnableTab("img2img", "sdxl", null)).toBe(false);
    expect(canEnableTab("text", "sdxl", status())).toBe(false);
    expect(canEnableTab("controlnet", "sdxl", status())).toBe(false); // 前処理ノードなし
    expect(canEnableTab("controlnet", "sdxl", status({ controlnetPreprocessors: ["canny"] }))).toBe(true);
    expect(canEnableTab("controlnet", "sdxl", status({ controlnetPreprocessors: ["canny"] }), false)).toBe(false);
    expect(canEnableTab("controlnet", "qwen_image", status({ controlnetPreprocessors: ["canny"] }))).toBe(false);
  });
});
