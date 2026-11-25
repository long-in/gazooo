import type { EngineStatus } from "@/lib/tauri/engine";
import type { GenerationParams } from "@/lib/tauri/jobs";

import { supportsExtras } from "./defaults";
import { supportsFaceSwap } from "./face-swap";
import { supportsReference } from "./reference";

/**
 * 生成フォームの作業タブ(docs/45)。排他の機能(画像から生成・Inpaint・参照画像・顔の入れ替え)を
 * 「何をしたいか」で選ぶ。タブは表示上の選択で、保存する設定(`enabled` フラグ)の形式は変えない(Q-387)。
 * ControlNet は「構図指定」のタブ(単独)と、画像から・部分修正のタブ内の「構図指定を併用」(併用)で使う。
 * 顔補正はタブにせず、「後処理」の折りたたみに置く(docs/45 段階 2・3)。
 */
export type WorkTab = "text" | "img2img" | "inpaint" | "controlnet" | "reference" | "faceSwap";

/** 排他の機能のタブ(「文章から」を除く)。 */
export const FEATURE_TABS = ["img2img", "inpaint", "controlnet", "reference", "faceSwap"] as const;
export type FeatureTab = (typeof FEATURE_TABS)[number];

type Kind = GenerationParams["model"]["kind"];

/** モデル種別で使えるタブ。「文章から」以外が無いモデル(Flux・Anima・Krea 2)は空(タブ自体を出さない。Q-386)。 */
export function availableTabs(kind: Kind): WorkTab[] {
  if (supportsExtras(kind)) return ["text", "img2img", "inpaint", "controlnet"];
  if (supportsReference(kind) || supportsFaceSwap(kind)) return ["text", "reference", "faceSwap"];
  return [];
}

/** 設定の `enabled` フラグから、今のタブを決める(プリセット・ジョブの復元に使う)。 */
export function activeTabOf(params: GenerationParams): WorkTab {
  if (params.faceSwap?.enabled) return "faceSwap";
  if (params.reference?.enabled) return "reference";
  if (params.inpaint?.enabled) return "inpaint";
  if (params.img2img?.enabled) return "img2img";
  if (params.controlnet?.enabled) return "controlnet";
  return "text";
}

/** タブを選んだとき、その機能を実際にオンにできるか(ノード・モデルの導入状況。各セクションの理由と同じ条件)。 */
export function canEnableTab(
  tab: WorkTab,
  kind: Kind,
  status: EngineStatus | null,
  hasControlnetModel = true,
): boolean {
  if (tab === "text" || status?.connected !== true) return false;
  const f = status.features;
  switch (tab) {
    case "img2img":
      return supportsExtras(kind) && f.img2img;
    case "inpaint":
      return supportsExtras(kind) && f.inpaint;
    case "controlnet":
      return supportsExtras(kind) && f.controlnetPreprocessors.length > 0 && hasControlnetModel;
    case "reference":
      return supportsReference(kind) && f.referenceEdit;
    case "faceSwap":
      return supportsFaceSwap(kind) && f.referenceEdit && f.faceSwap;
  }
}

/** 画面に出すタブ。機能がオンならそのタブ(設定の復元に追従)、すべてオフなら選んだタブ。使えなければ「文章から」。 */
export function currentTab(params: GenerationParams, picked: WorkTab): WorkTab {
  const derived = activeTabOf(params);
  return tabForKind(derived !== "text" ? derived : picked, params.model.kind);
}

/** 今のタブがそのモデルで使えないなら「文章から」へ戻す(設定は保持。Q-384)。 */
export function tabForKind(tab: WorkTab, kind: Kind): WorkTab {
  return availableTabs(kind).includes(tab) ? tab : "text";
}
