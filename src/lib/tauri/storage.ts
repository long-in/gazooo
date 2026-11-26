import { isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

import { invoke } from "./invoke";
import type { SettingsScope } from "./settings";

/** `imagesGlobal` = 全体の画像の保存先、`imagesUser` = ログイン中のユーザーの個別の保存先。 */
export type MigrationTarget = "imagesGlobal" | "imagesUser" | "models";

/** 開始できない理由(docs/51 §5)。 */
export type MigrationBlocker =
  | "SOURCE_UNKNOWN"
  | "SOURCE_MISSING"
  | "DEST_NOT_ABSOLUTE"
  | "DEST_NOT_WRITABLE"
  | "DEST_IN_APP_DATA"
  | "DEST_IN_USE"
  | "USER_UNREADABLE"
  | "SAME_DIR"
  | "NESTED"
  | "NO_SPACE"
  | "CONFLICT"
  | "JOB_RUNNING"
  | "DOWNLOAD_ACTIVE"
  | "BUSY";

export type EngineBlock = "remote" | "engine_unknown" | "foreign" | "custom_config";

export interface MigrationPlan {
  sourceDir: string;
  destDir: string;
  fileCount: number;
  totalBytes: number;
  copyBytes: number;
  freeBytes: number | null;
  sameSizeCount: number;
  missing: number;
  conflicts: string[];
  skippedFolders: string[];
  skippedLinks: number;
  blockers: MigrationBlocker[];
  /** 全体の移行の対象ユーザー。 */
  users: { name: string; fileCount: number; totalBytes: number }[];
  /** 個別設定のため、全体の移行の対象外のユーザー数。 */
  excludedUsers: number;
  /** モデルのみ。 */
  engine?: {
    managedRunning: boolean;
    externalRunning: boolean;
    canApply: boolean;
    reason?: EngineBlock;
  };
}

export interface MigrationReport {
  sourceDir: string;
  destDir: string;
  moved: number;
  missing: number;
  keptSameSize: number;
  leftInSource: number;
  /** ファイルを残した元のフォルダ。 */
  leftDirs: string[];
  engineApplied?: boolean;
  engineRestarted?: boolean;
  engineError?: string;
}

export interface MigrationProgress {
  phase: "copying" | "switching" | "cleaning";
  doneBytes: number;
  totalBytes: number;
  doneFiles: number;
  totalFiles: number;
  currentFile?: string;
}

/** 画像の保存先を切り替える(ファイルは移さない)。`newDir = null` は、個別なら全体設定、全体なら標準値に戻す。 */
export const storageChangeImagesDir = (scope: SettingsScope, newDir: string | null) =>
  invoke<void>("storage_change_images_dir", { scope, newDir });

/** 移行の内容を確認する(何も変更しない)。 */
export const storageMigrationPlan = (target: MigrationTarget, newDir: string | null) =>
  invoke<MigrationPlan>("storage_migration_plan", { target, newDir });

/** 移行を実行する。完了まで返らない。 */
export const storageMigrationStart = (target: MigrationTarget, newDir: string | null) =>
  invoke<MigrationReport>("storage_migration_start", { target, newDir });

export const storageMigrationCancel = () => invoke<void>("storage_migration_cancel");

/** 移行を中止して(切り替え以降なら完了を待って)アプリを終了する。 */
export const storageMigrationQuit = () => invoke<void>("storage_migration_quit");

export async function pickStorageDir(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : null;
}

async function subscribe<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return listen<T>(event, (e) => handler(e.payload));
}

export const onMigrationProgress = (handler: (p: MigrationProgress) => void) =>
  subscribe<MigrationProgress>("storage://migration-progress", handler);

/** 移行中にアプリを閉じようとした。 */
export const onMigrationCloseRequested = (handler: () => void) =>
  subscribe<null>("storage://close-requested", () => handler());
