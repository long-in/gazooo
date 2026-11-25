import type { AssistantStatus } from "@/lib/tauri/assistant";
import type { LlmInstallState, LlmState, LlmStatus } from "@/lib/tauri/llm";

/** 相談パネルが最初に出す画面(docs/11 §5・§13.2、docs/30 §6)。 */
export type PanelGate = "loading" | "noKey" | "consent" | "notInstalled" | "chat";

/**
 * 送信先ごとの前提を満たしているか。`anthropic` はキーと同意が要る。
 * `local`(アプリ管理)は導入済みであればよく、`local_external` は何も要らない(外部へ送らないため)。
 */
export function panelGate(status: AssistantStatus | null, llm: LlmStatus | null): PanelGate {
  if (!status) return "loading";
  switch (status.provider) {
    case "anthropic":
      if (!status.hasKey) return "noKey";
      return status.consent ? "chat" : "consent";
    case "local":
      // 状態が未取得の間は、チャットを出して送信時のエラーに任せる(誤って案内を出さない)
      return llm && !llm.installed ? "notInstalled" : "chat";
    case "local_external":
      return "chat";
  }
}

/** 外部(Anthropic)へ送るか。料金の注記・送信先の表示に使う。 */
export const sendsExternally = (status: AssistantStatus | null): boolean => status?.provider === "anthropic";

/** 導入が実行中か。 */
export const isInstalling = (s: LlmInstallState | null | undefined): boolean => s?.status === "running";

/** 導入全体の進捗(0〜100)。3 手順(runtime / extract / model)をサイズで重み付けせず、手順の進みで概算する。 */
export function installPercent(s: LlmInstallState): number | null {
  if (s.status === "done") return 100;
  if (s.status !== "running" || s.totalSteps === 0) return null;
  const within = s.total && s.received != null ? Math.min(1, s.received / s.total) : 0;
  // 重み: ランタイム 5% / 展開 5% / モデル 90%(ダウンロードの大半はモデル)
  const weights = [0.05, 0.05, 0.9];
  const idx = Math.max(0, Math.min(weights.length - 1, s.step - 1));
  const done = weights.slice(0, idx).reduce((a, b) => a + b, 0);
  return Math.round((done + weights[idx] * (s.stepId === "extract" ? 0.5 : within)) * 100);
}

/** 起動中か(モデルの読み込み待ちを示す)。 */
export const isLoading = (state: LlmState | undefined): boolean => state === "starting";

/** メモリを解放できる状態か(動いている・起動中)。 */
export const canRelease = (state: LlmState | undefined): boolean => state === "ready" || state === "starting";

/** 起動できる状態か。 */
export const canStart = (llm: LlmStatus | null): boolean =>
  !!llm && llm.installed && (llm.state === "stopped" || llm.state === "error");

/** MB を「約 6.5 GB」「800 MB」の形にする。 */
export function formatMemory(mb: number): string {
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}
