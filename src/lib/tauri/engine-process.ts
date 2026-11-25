import { open } from "@tauri-apps/plugin-dialog";

import { invoke } from "./invoke";

/** ComfyUI の起動管理(docs/15、docs/04-ipc-api.md「ComfyUI の起動管理」)。型は Rust 側が正。 */

export interface ManagedEngineConfig {
  enabled: boolean;
  mainPy: string;
  python: string;
  baseDirectory: string | null;
  port: number;
  autoStart: boolean;
  /** 接続が 90 秒以上切れたら自動で再起動し、キューを実行し直す(docs/47)。 */
  autoRecover: boolean;
}

export type EngineProcStatus = "stopped" | "starting" | "running" | "stopping" | "crashed";

export interface EngineProcState {
  status: EngineProcStatus;
  pid?: number;
  exitCode?: number;
  message?: string;
  /** アプリが起動した子プロセスが存在するか。 */
  managedByApp: boolean;
}

export interface EngineSuggestions {
  mainPy?: string;
  python?: string;
  baseDirectory?: string;
}

export interface ManagedView {
  config: ManagedEngineConfig;
  state: EngineProcState;
}

/** 設定と状態(即時。候補は含まない。docs/32)。 */
export const engineManagedGet = () => invoke<ManagedView>("engine_managed_get");

/** 入力候補(探索に時間がかかることがあるため、`engineManagedGet` とは別に呼ぶ)。 */
export const engineManagedSuggest = () => invoke<EngineSuggestions>("engine_managed_suggest");

/** 空の項目だけに候補を入れる(入力済みの項目は上書きしない。docs/32 Q-182)。 */
export function applySuggestions(config: ManagedEngineConfig, s: EngineSuggestions): ManagedEngineConfig {
  return {
    ...config,
    mainPy: config.mainPy || s.mainPy || "",
    python: config.python || s.python || "",
    baseDirectory: config.baseDirectory || s.baseDirectory || null,
  };
}

export const engineManagedSet = (config: ManagedEngineConfig) =>
  invoke<ManagedView>("engine_managed_set", { config });

export const engineProcessStart = () => invoke<EngineProcState>("engine_process_start");

export const engineProcessStop = () => invoke<EngineProcState>("engine_process_stop");

/** 実行中・待機中のジョブがあり `force` でなければ `ENGINE_BUSY`(`details.running` / `details.queued`)。 */
export const engineProcessRestart = (force?: boolean) =>
  invoke<EngineProcState>("engine_process_restart", { force });

export const engineProcessLogs = (lines?: number) =>
  invoke<{ lines: string[] }>("engine_process_logs", { lines });

/** ファイル選択ダイアログ(main.py・Python)。キャンセルなら null。 */
export async function pickFile(): Promise<string | null> {
  const selected = await open({ directory: false, multiple: false });
  return typeof selected === "string" ? selected : null;
}

/** フォルダ選択ダイアログ(--base-directory)。キャンセルなら null。 */
export async function pickFolder(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : null;
}

/** 起動・停止などの操作中(状態の遷移待ち)か。 */
export const isTransient = (s: EngineProcStatus) => s === "starting" || s === "stopping";
