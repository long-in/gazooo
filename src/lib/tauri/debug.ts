import { invoke } from "./invoke";

/** 調査用の詳細ログ(docs/31、docs/04-ipc-api.md)。型は Rust 側が正。 */

export interface DebugStatus {
  /** デバッグログ(DEBUG)を出力しているか。 */
  enabled: boolean;
  logDir: string;
  fileCount: number;
  totalBytes: number;
  /** 合計の上限。超えると古いアプリのログから自動で削除される。 */
  limitBytes: number;
}

export const debugStatus = () => invoke<DebugStatus>("debug_status");

/** ログレベルを即時に切り替える(再起動不要)。設定は保存される。 */
export const debugSetEnabled = (enabled: boolean) => invoke<DebugStatus>("debug_set_enabled", { enabled });

/** Rust 側で保存ダイアログを開き、ログ一式を ZIP にして書き出す。キャンセルは `null`。 */
export const debugExportLogs = () => invoke<{ path: string } | null>("debug_export_logs");

export const debugOpenLogDir = () => invoke<void>("debug_open_log_dir");

/** アプリのログを削除する(書き込み中のファイルは空にする)。 */
export const debugClearLogs = () => invoke<DebugStatus>("debug_clear_logs");
