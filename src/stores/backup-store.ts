import { create } from "zustand";

import {
  type BackupEstimate,
  type BackupExportReport,
  type BackupInclude,
  type BackupManifest,
  type BackupOrphans,
  type BackupProgress,
  type BackupRestoreArgs,
  backupCancel,
  backupEstimate,
  backupExport,
  backupInspect,
  backupOrphans,
  backupRestore,
} from "@/lib/tauri/backup";

export type BackupBusy = "export" | "inspect" | "restore" | null;

/** 利用者が中止したときに Rust が返すコード(通知しない)。 */
export const BACKUP_CANCELLED = "BACKUP_CANCELLED";

interface BackupStoreState {
  /** 取得に含める区分。既定はすべて ON(docs/35 §1)。 */
  include: BackupInclude;
  /** API キーを含める(設定値が ON のときだけ有効。既定は OFF)。 */
  includeApiKey: boolean;
  estimate: BackupEstimate | null;
  busy: BackupBusy;
  progress: BackupProgress | null;

  setInclude: (key: keyof BackupInclude, value: boolean) => void;
  setIncludeApiKey: (value: boolean) => void;
  setProgress: (p: BackupProgress | null) => void;
  /** 失敗しても何もしない(見込みが取れないだけ)。 */
  loadEstimate: () => Promise<void>;
  /** 失敗時は `AppError` を throw する(呼び出し側で表示)。キャンセル・中止は `null`。 */
  exportBackup: () => Promise<BackupExportReport | null>;
  /** ファイルを選んで検査する。キャンセルは `null`。 */
  inspect: () => Promise<BackupManifest | null>;
  orphans: (include: BackupInclude) => Promise<BackupOrphans>;
  restore: (args: BackupRestoreArgs) => Promise<void>;
  cancel: () => Promise<void>;
}

const ALL_ON: BackupInclude = { settings: true, gallery: true, presets: true };

/** 3 つとも OFF のときは保存できない。 */
export const canExport = (include: BackupInclude) =>
  include.settings || include.gallery || include.presets;

export const useBackupStore = create<BackupStoreState>((set, get) => {
  async function run<T>(
    kind: Exclude<BackupBusy, null>,
    op: () => Promise<T>,
  ): Promise<T> {
    set({ busy: kind, progress: null });
    try {
      return await op();
    } finally {
      set({ busy: null, progress: null });
    }
  }

  return {
    include: { ...ALL_ON },
    includeApiKey: false,
    estimate: null,
    busy: null,
    progress: null,

    setInclude: (key, value) => {
      const include = { ...get().include, [key]: value };
      // 設定値を外したら、API キーの選択も外す(含める対象が無いため)
      set({
        include,
        includeApiKey: include.settings ? get().includeApiKey : false,
      });
      void get().loadEstimate();
    },
    setIncludeApiKey: (value) =>
      set({ includeApiKey: get().include.settings && value }),
    setProgress: (progress) => set({ progress }),

    loadEstimate: async () => {
      try {
        set({ estimate: await backupEstimate(get().include) });
      } catch {
        // Tauri の外(テスト・ブラウザ)などでは取得できない
      }
    },

    exportBackup: () => {
      const { include, includeApiKey } = get();
      return run("export", async () => {
        if (!canExport(include)) return null;
        return backupExport(include, include.settings && includeApiKey);
      });
    },

    inspect: () => run("inspect", () => backupInspect()),

    orphans: (include) => backupOrphans(include),

    restore: (args) =>
      run("restore", async () => {
        await backupRestore(args);
      }),

    cancel: () => backupCancel(),
  };
});
