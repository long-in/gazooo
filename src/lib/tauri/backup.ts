import { invoke } from "./invoke";

/** バックアップ(docs/35、docs/04-ipc-api.md)。型は Rust 側が正。 */

/** 取得・復元の区分。 */
export interface BackupInclude {
  settings: boolean;
  gallery: boolean;
  presets: boolean;
}

export interface BackupEstimate {
  settings: number;
  conversations: number;
  jobs: number;
  images: number;
  presets: number;
  /** ギャラリーを含めるときの、画像ファイルの合計バイト数。 */
  bytes: number;
}

export interface BackupExportReport {
  path: string;
  images: number;
  /** ディスクに無く、含められなかったファイル数。 */
  skipped: number;
  bytes: number;
}

export interface BackupManifest {
  format: number;
  app: string;
  appVersion: string;
  schemaVersion: number;
  createdAt: number;
  include: BackupInclude & { apiKey: boolean };
  counts: {
    settings: number;
    conversations: number;
    jobs: number;
    images: number;
    inputs: number;
    presets: number;
  };
  bytes: { images: number };
}

export interface BackupOrphans {
  count: number;
  bytes: number;
}

export type BackupPhase = "snapshot" | "files" | "swap";

export interface BackupProgress {
  phase: BackupPhase;
  done: number;
  total: number;
}

/** 区分ごとの件数と容量の見込み(ファイルのメタデータのみで、読み込みはしない)。 */
export const backupEstimate = (include: BackupInclude) =>
  invoke<BackupEstimate>("backup_estimate", { include });

/** Rust 側で保存ダイアログを開いて書き出す。キャンセル・中止は `null`。 */
export const backupExport = (include: BackupInclude, includeApiKey: boolean) =>
  invoke<BackupExportReport | null>("backup_export", {
    include,
    includeApiKey,
  });

/** Rust 側でファイル選択ダイアログを開いて検査する。キャンセルは `null`。復元の対象として Rust 側が保持する。 */
export const backupInspect = () =>
  invoke<BackupManifest | null>("backup_inspect");

/** 復元した場合に「バックアップに無い画像ファイル」になるものの数と容量。 */
export const backupOrphans = (include: BackupInclude) =>
  invoke<BackupOrphans>("backup_orphans", { include });

export interface BackupRestoreArgs {
  include: BackupInclude;
  restoreApiKey: boolean;
  /** 待機中・実行中のジョブを取り消してから復元する。 */
  stopJobs: boolean;
  deleteOrphans: boolean;
}

export const backupRestore = (args: BackupRestoreArgs) =>
  invoke<{ restartRequired: boolean }>("backup_restore", { ...args });

export const backupCancel = () => invoke<void>("backup_cancel");

/** アプリを再起動する(復元の反映)。 */
export const backupRestart = () => invoke<void>("backup_restart");
