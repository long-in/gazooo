/** 画面の状態(パネルの開閉・幅・表示の密度。docs/29 §6)。`settings.uiState` に JSON で保存する。 */

export type Density = "compact" | "comfortable";

/** 生成ページのモード(docs/38 §2)。かんたん = 初級者向け、詳細 = すべての項目。 */
export type GenerateMode = "simple" | "advanced";

export interface UiState {
  density: Density;
  generateMode: GenerateMode;
  /** パネル id → 開いているか。 */
  panels: Record<string, boolean>;
  /** ペイン id → 幅(px)。 */
  widths: Record<string, number>;
  /** 生成ページで選んでいるプロジェクト(null = 未分類。docs/40 §2)。 */
  currentProjectId: string | null;
  /** ギャラリーのスコープ("all" / "favorites" / "unassigned" / "project:<id>"。docs/40 §5.1)。 */
  galleryScope: string;
}

export const defaultUiState: UiState = {
  density: "compact",
  generateMode: "simple",
  panels: {},
  widths: {},
  currentProjectId: null,
  galleryScope: "all",
};

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

/** 保存された値を検証して取り込む。壊れた項目は既定に戻す(起動不能にしない)。 */
export function parseUiState(raw: unknown): UiState {
  if (!isRecord(raw)) return defaultUiState;
  const panels: Record<string, boolean> = {};
  if (isRecord(raw.panels)) {
    for (const [k, v] of Object.entries(raw.panels)) if (typeof v === "boolean") panels[k] = v;
  }
  const widths: Record<string, number> = {};
  if (isRecord(raw.widths)) {
    for (const [k, v] of Object.entries(raw.widths)) if (typeof v === "number" && Number.isFinite(v) && v > 0) widths[k] = v;
  }
  return {
    density: raw.density === "comfortable" ? "comfortable" : "compact",
    generateMode: raw.generateMode === "advanced" ? "advanced" : "simple",
    panels,
    widths,
    currentProjectId: typeof raw.currentProjectId === "string" && raw.currentProjectId !== "" ? raw.currentProjectId : null,
    // 形式の検証は復元側(`decodeEntry`)が行う。文字列以外は既定に戻す
    galleryScope: typeof raw.galleryScope === "string" ? raw.galleryScope : "all",
  };
}

/** 幅を最小・最大に収める。 */
export function clampWidth(width: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(width)));
}
