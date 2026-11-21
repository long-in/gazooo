import { invoke } from "./invoke";

/** ローカル LLM の導入・起動・メモリ解放(docs/30、docs/04-ipc-api.md「ローカル LLM」)。型は Rust 側が正。 */

export type LlmState = "not_installed" | "stopped" | "starting" | "ready" | "stopping" | "error";

export type LlmInstallStatus = "idle" | "running" | "done" | "failed" | "cancelled";

export interface LlmInstallState {
  installId: string | null;
  status: LlmInstallStatus;
  /** 現在の手順(1 始まり)。実行中以外は 0。 */
  step: number;
  totalSteps: number;
  /** `runtime` / `extract` / `model` / `license` */
  stepId: string | null;
  received: number | null;
  total: number | null;
  error: { code: string; message: string } | null;
}

export interface LlmStatus {
  /** この OS で使えるか(Windows x64 / macOS Apple Silicon)。 */
  supported: boolean;
  installed: boolean;
  updateAvailable: boolean;
  runtime: string | null;
  model: string | null;
  runtimeLicense: string;
  modelLicense: string;
  /** 導入済みのライセンス全文のファイル名(翻訳専用モデル。docs/34 §9.13)。 */
  licenseFiles: string[];
  /** 導入・更新でダウンロードするサイズの目安(MB)。 */
  downloadMb: number;
  state: LlmState;
  message: string | null;
  idleMinutes: number;
  releaseBeforeGeneration: boolean;
  /** 起動中・準備完了のときの、メモリ使用量の目安(MB)。 */
  memoryHintMb: number | null;
  install: LlmInstallState;
}

/** 自動で解放するまでの分数の選択肢(0 = 自動停止なし)。Rust 側と同じ。 */
export const IDLE_MINUTES_CHOICES = [1, 5, 15, 30, 0] as const;

export const llmStatus = () => invoke<LlmStatus>("llm_status");

export const llmInstall = () => invoke<{ installId: string }>("llm_install");

export const llmInstallCancel = (installId: string) => invoke<void>("llm_install_cancel", { installId });

export const llmRemove = () => invoke<LlmStatus>("llm_remove");

/** 準備完了は待たない(状態は `llm://state` で届く)。 */
export const llmStart = () => invoke<LlmStatus>("llm_start");

/** 停止してメモリを解放する。 */
export const llmStop = () => invoke<LlmStatus>("llm_stop");

export const llmLogs = () => invoke<string[]>("llm_logs");

export const llmUpdateSettings = (patch: { idleMinutes?: number; releaseBeforeGeneration?: boolean }) =>
  invoke<LlmStatus>("llm_update_settings", patch);
