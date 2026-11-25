import { create } from "zustand";

import {
  autofill,
  defaultControlnet,
  defaultControlnetUnit,
  defaultFaceDetailer,
  defaultNegative,
  defaultParams,
  pickControlnetModel,
  resetParams,
  restoreParams,
  supportsControlnet,
  switchKind,
} from "@/lib/generation/defaults";
import { applyBuiltinPreset, type BuiltinPreset } from "@/lib/generation/builtin-presets";
import type { RecommendedPatch } from "@/lib/models/profile";
import { withCompositionLock, withImg2imgEnabled, withImg2imgImage } from "@/lib/generation/img2img";
import { defaultImg2img } from "@/lib/generation/img2img-defaults";
import { defaultInpaint, sizeForImage } from "@/lib/generation/inpaint";
import type { WorkTab } from "@/lib/generation/work-tab";
import { addTargets, targetsFromParams, withFaceSwap, withFaceSwapEnabled } from "@/lib/generation/face-swap";
import {
  withReferenceEnabled,
  withReferenceImageAdded,
  withReferenceImageMoved,
  withReferenceImageRemoved,
  withReferenceResolution,
} from "@/lib/generation/reference";
import { applyChanges, usableChanges } from "@/lib/generation/suggestions";
import { useModelsStore } from "@/stores/models-store";
import type { SuggestedChanges } from "@/lib/tauri/assistant";
import type { EngineModels } from "@/lib/tauri/engine";
import type {
  ControlnetUnit,
  FaceDetailerParams,
  FaceSwapParams,
  GenerationParams,
  Img2imgParams,
  InpaintParams,
  ModelKind,
  ModelSpec,
} from "@/lib/tauri/jobs";

/** モデルの絞り込みに使うカタログ(未取得なら `null` = 絞り込まない)。 */
const catalogNow = () => useModelsStore.getState().view?.catalog ?? null;

interface FormState {
  params: GenerationParams;
  /** 同じ設定で連続して生成するジョブ数(「枚数」)。下書きには保存しない。 */
  count: number;
  /** 顔の入れ替えの対象画像("inputs/<uuid>.png")。ジョブは 1 枚ずつ投入する(docs/42 §4.3)。下書きには保存しない。 */
  faceSwapTargets: string[];
  hydrated: boolean;
  /** 直近に適用した AI の提案を元に戻すための値(適用した項目の元の値だけ)。 */
  suggestionUndo: SuggestedChanges | null;

  /** 保存済みの下書きから復元する(初回のみ)。 */
  hydrate: (saved: Partial<GenerationParams> | null) => void;
  patch: (patch: Partial<Omit<GenerationParams, "model">>) => void;
  patchModel: (patch: Partial<ModelSpec>) => void;
  /** 顔補正の設定を部分更新する(未設定なら既定値から作る)。 */
  patchFace: (patch: Partial<FaceDetailerParams>) => void;
  setControlnetEnabled: (enabled: boolean) => void;
  /** ControlNet のユニットを部分更新する(未設定なら既定値から作る)。前処理を変えたときは、合うモデルへ切り替える。 */
  patchControlnetUnit: (patch: Partial<ControlnetUnit>, controlnets?: string[]) => void;
  /** 参照画像を設定して ControlNet を有効にする(非対応の種別では無効のまま。有効にできたかを返す)。 */
  setReferenceImage: (path: string, controlnets?: string[]) => boolean;
  /** Inpaint を切り替える。有効にすると顔補正は無効にする(同時に使えない。docs/13 §3.2)。 */
  setInpaintEnabled: (enabled: boolean) => void;
  patchInpaint: (patch: Partial<InpaintParams>) => void;
  /** Inpaint の参照画像を設定する。生成サイズを画像に合わせ、前のマスクは捨てる。 */
  setInpaintImage: (path: string, width: number, height: number) => void;
  /** 画像から生成(img2img)を切り替える。有効にすると Inpaint は無効にする(同時に使えない。docs/19 §4)。 */
  setImg2imgEnabled: (enabled: boolean) => void;
  patchImg2img: (patch: Partial<Img2imgParams>) => void;
  /** 元画像を設定する。生成サイズを画像に合わせる(構図の固定中は ControlNet の画像も揃える)。 */
  setImg2imgImage: (path: string, width: number, height: number) => void;
  /** 構図の固定(ControlNet の Canny を元画像で自動設定)を入れる/切る。 */
  setCompositionLock: (on: boolean, controlnets: string[]) => void;
  /** 参照画像つき生成(Qwen-Image 2.1。docs/24)を切り替える。 */
  setReferenceEnabled: (enabled: boolean) => void;
  /** 参照画像を末尾に足す(上限 4 枚)。1 枚目を足すときは生成サイズを画像に合わせる。 */
  addReferenceImage: (path: string, width: number, height: number) => void;
  removeReferenceImage: (index: number) => void;
  moveReferenceImage: (index: number, delta: -1 | 1) => void;
  setReferenceResolution: (resolution: number) => void;
  /** 顔の入れ替え(Qwen-Image 2.1。docs/42)を切り替える。入れると参照画像つき生成は切り、雛形・ステップ等を既定にする。 */
  setFaceSwapEnabled: (enabled: boolean) => void;
  /** 作業タブの選択(docs/45)。排他の機能をすべて切り、`enable` のときだけ選んだ機能をオンにする。 */
  setWorkTab: (tab: WorkTab, enable: boolean) => void;
  /** 選んだ作業タブ(表示用。下書きには保存しない)。機能がオンならそのタブが優先される(`currentTab`)。 */
  workTab: WorkTab;
  patchFaceSwap: (patch: Partial<FaceSwapParams>) => void;
  /** 対象画像を末尾に足す(重複は無視。上限あり)。 */
  addFaceSwapTargets: (paths: string[]) => void;
  removeFaceSwapTarget: (index: number) => void;
  clearFaceSwapTargets: () => void;
  /** ネガティブを種別の既定値に戻す(docs/21 §2.1)。 */
  resetNegative: () => void;
  /** プロンプト・サイズ等を推奨値に戻し、拡張機能を外す。モデルの選択は保持する(docs/21 §2.2)。 */
  resetAll: () => void;
  setKind: (kind: ModelKind) => void;
  setCount: (count: number) => void;
  /** ギャラリーの「この設定で生成」: 画像のパラメータをフォームに読み込む(シードは固定して再現する)。 */
  loadParams: (params: Partial<GenerationParams>) => void;
  /** プリセットの適用: 保存された設定をそのまま復元する(シードのランダム設定も含む)。 */
  applyPreset: (params: Partial<GenerationParams>) => void;
  /** モデルの推奨設定(サイズ・ステップ・CFG・サンプラー)を適用する(docs/18 §5)。プロンプトは変えない。 */
  applyRecommended: (patch: RecommendedPatch, models: EngineModels | null) => void;
  /** 種別とモデルを切り替える(AI のモデル提案の反映。docs/18 §6.3)。プロンプトなどは引き継ぐ。 */
  switchModel: (kind: ModelKind, name: string, models: EngineModels | null) => void;
  /** 組み込みプリセットを適用する(docs/16 §5)。プロンプトなどは保ち、エンジンの一覧でモデルを補う。 */
  applyBuiltinPreset: (preset: BuiltinPreset, models: EngineModels | null) => void;
  /** AI の提案を適用する(適用できた項目があれば true)。 */
  applySuggestion: (changes: SuggestedChanges, models: EngineModels | null) => boolean;
  /** 直近の提案の適用を、その項目だけ元に戻す。 */
  undoSuggestion: () => void;
  /** エンジンのモデル一覧が取れた(変わった)ときに、未選択・不在の項目を埋める。 */
  applyModels: (models: EngineModels | null) => void;
}

export const useFormStore = create<FormState>((set, get) => ({
  params: defaultParams("sdxl"),
  count: 1,
  faceSwapTargets: [],
  workTab: "text",
  hydrated: false,
  suggestionUndo: null,

  hydrate: (saved) => {
    if (get().hydrated) return;
    set({ params: restoreParams(saved), hydrated: true });
  },

  patch: (patch) => set((s) => ({ params: { ...s.params, ...patch } })),

  patchModel: (patch) => set((s) => ({ params: { ...s.params, model: { ...s.params.model, ...patch } } })),

  patchFace: (patch) =>
    set((s) => ({
      params: { ...s.params, faceDetailer: { ...(s.params.faceDetailer ?? defaultFaceDetailer()), ...patch } },
    })),

  setControlnetEnabled: (enabled) =>
    set((s) => ({
      params: { ...s.params, controlnet: { ...(s.params.controlnet ?? defaultControlnet()), enabled } },
    })),

  patchControlnetUnit: (patch, controlnets = []) =>
    set((s) => {
      const cn = s.params.controlnet ?? defaultControlnet();
      const unit = { ...(cn.units[0] ?? defaultControlnetUnit()), ...patch };
      if (patch.preprocessor !== undefined || unit.model === "") {
        unit.model = pickControlnetModel(controlnets, unit.preprocessor, unit.model);
      }
      return { params: { ...s.params, controlnet: { ...cn, units: [unit] } } };
    }),

  setReferenceImage: (path, controlnets = []) => {
    get().patchControlnetUnit({ image: path }, controlnets);
    const enabled = supportsControlnet(get().params.model.kind);
    get().setControlnetEnabled(enabled);
    return enabled;
  },

  setInpaintEnabled: (enabled) =>
    set((s) => ({
      params: {
        ...s.params,
        inpaint: { ...(s.params.inpaint ?? defaultInpaint()), enabled },
        ...(enabled && s.params.faceDetailer && { faceDetailer: { ...s.params.faceDetailer, enabled: false } }),
        ...(enabled && s.params.img2img && { img2img: { ...s.params.img2img, enabled: false } }),
      },
    })),

  setImg2imgEnabled: (enabled) => set((s) => ({ params: withImg2imgEnabled(s.params, enabled) })),

  patchImg2img: (patch) =>
    set((s) => ({ params: { ...s.params, img2img: { ...(s.params.img2img ?? defaultImg2img()), ...patch } } })),

  setImg2imgImage: (path, width, height) => set((s) => ({ params: withImg2imgImage(s.params, path, width, height) })),

  setCompositionLock: (on, controlnets) => set((s) => ({ params: withCompositionLock(s.params, on, controlnets) })),

  patchInpaint: (patch) =>
    set((s) => ({ params: { ...s.params, inpaint: { ...(s.params.inpaint ?? defaultInpaint()), ...patch } } })),

  setInpaintImage: (path, width, height) =>
    set((s) => {
      const size = sizeForImage(width, height);
      return {
        params: {
          ...s.params,
          width: size.width,
          height: size.height,
          inpaint: { ...(s.params.inpaint ?? defaultInpaint()), image: path, mask: "" },
        },
      };
    }),

  setReferenceEnabled: (enabled) => set((s) => ({ params: withReferenceEnabled(s.params, enabled) })),

  addReferenceImage: (path, width, height) =>
    set((s) => ({ params: withReferenceImageAdded(s.params, path, width, height) })),

  removeReferenceImage: (index) => set((s) => ({ params: withReferenceImageRemoved(s.params, index) })),

  moveReferenceImage: (index, delta) => set((s) => ({ params: withReferenceImageMoved(s.params, index, delta) })),

  setReferenceResolution: (resolution) => set((s) => ({ params: withReferenceResolution(s.params, resolution) })),

  setFaceSwapEnabled: (enabled) => set((s) => ({ params: withFaceSwapEnabled(s.params, enabled) })),

  setWorkTab: (tab, enable) => {
    const s = get();
    set({ workTab: tab });
    // 今オンの機能だけを切る(未設定の機能に既定の設定を作らない)
    if (s.params.faceSwap?.enabled && tab !== "faceSwap") s.setFaceSwapEnabled(false);
    if (s.params.reference?.enabled && tab !== "reference") s.setReferenceEnabled(false);
    if (s.params.inpaint?.enabled && tab !== "inpaint") s.setInpaintEnabled(false);
    if (s.params.img2img?.enabled && tab !== "img2img") s.setImg2imgEnabled(false);
    // ControlNet は画像から・部分修正と併用できるので、それ以外のタブへ移るときだけ切る(構図指定のタブでは残す)
    if (s.params.controlnet?.enabled && !["img2img", "inpaint", "controlnet"].includes(tab)) s.setControlnetEnabled(false);
    if (!enable) return;
    if (tab === "img2img") s.setImg2imgEnabled(true);
    else if (tab === "inpaint") s.setInpaintEnabled(true);
    else if (tab === "controlnet") s.setControlnetEnabled(true);
    else if (tab === "reference") s.setReferenceEnabled(true);
    else if (tab === "faceSwap") s.setFaceSwapEnabled(true);
  },

  patchFaceSwap: (patch) => set((s) => ({ params: withFaceSwap(s.params, patch) })),

  addFaceSwapTargets: (paths) => set((s) => ({ faceSwapTargets: addTargets(s.faceSwapTargets, paths) })),

  removeFaceSwapTarget: (index) => set((s) => ({ faceSwapTargets: s.faceSwapTargets.filter((_, i) => i !== index) })),

  clearFaceSwapTargets: () => set({ faceSwapTargets: [] }),

  resetNegative: () => set((s) => ({ params: { ...s.params, negativePrompt: defaultNegative(s.params.model.kind) } })),

  resetAll: () =>
    set((s) => ({ params: resetParams(s.params), count: 1, faceSwapTargets: [], suggestionUndo: null })),

  setKind: (kind) => set((s) => ({ params: switchKind(s.params, kind) })),

  setCount: (count) => set({ count }),

  applyPreset: (params) =>
    set({ params: restoreParams(params), faceSwapTargets: targetsFromParams(params), hydrated: true }),

  applyRecommended: (patch, models) =>
    set((s) => ({ params: autofill({ ...s.params, ...patch }, models, catalogNow()) })),

  switchModel: (kind, name, models) =>
    set((s) => {
      const switched = switchKind(s.params, kind);
      return { params: autofill({ ...switched, model: { ...switched.model, name } }, models, catalogNow()) };
    }),

  applyBuiltinPreset: (preset, models) =>
    set((s) => ({ params: autofill(applyBuiltinPreset(s.params, preset), models, catalogNow()), hydrated: true })),

  loadParams: (params) =>
    set({
      params: { ...restoreParams(params), seedRandomize: false },
      faceSwapTargets: targetsFromParams(params),
      hydrated: true,
    }),

  applySuggestion: (changes, models) => {
    const params = get().params;
    const usable = usableChanges(params, changes, models);
    if (Object.keys(usable).length === 0) return false;
    const { next, inverse } = applyChanges(params, usable);
    set({ params: autofill(next, models, catalogNow()), suggestionUndo: inverse });
    return true;
  },

  undoSuggestion: () => {
    const undo = get().suggestionUndo;
    if (!undo) return;
    set((s) => ({ params: applyChanges(s.params, undo).next, suggestionUndo: null }));
  },

  applyModels: (models) =>
    set((s) => {
      const next = autofill(s.params, models, catalogNow());
      return next === s.params ? s : { params: next };
    }),
}));
