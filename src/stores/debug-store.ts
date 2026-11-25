import { create } from "zustand";

import {
  type DebugStatus,
  debugClearLogs,
  debugExportLogs,
  debugOpenLogDir,
  debugSetEnabled,
  debugStatus,
} from "@/lib/tauri/debug";

interface DebugStoreState {
  status: DebugStatus | null;
  /** 保存・削除などの操作中。 */
  busy: boolean;

  /** 失敗しても何もしない(状態が取れないだけ。表示側は `status === null` で隠す)。 */
  load: () => Promise<void>;
  /** 失敗時は `AppError` を throw する(呼び出し側で表示)。 */
  setEnabled: (enabled: boolean) => Promise<void>;
  /** 保存したパス。キャンセルは `null`。 */
  exportLogs: () => Promise<string | null>;
  openDir: () => Promise<void>;
  clear: () => Promise<void>;
}

export const useDebugStore = create<DebugStoreState>((set) => {
  async function run<T>(op: () => Promise<T>): Promise<T> {
    set({ busy: true });
    try {
      return await op();
    } finally {
      set({ busy: false });
    }
  }

  return {
    status: null,
    busy: false,

    load: async () => {
      try {
        set({ status: await debugStatus() });
      } catch {
        // Tauri の外(テスト・ブラウザ)などでは取得できない
      }
    },
    setEnabled: (enabled) => run(async () => set({ status: await debugSetEnabled(enabled) })),
    exportLogs: () =>
      run(async () => {
        const r = await debugExportLogs();
        // 保存後は件数・サイズが変わる(ログに「exported」が追記される)ので取り直す
        if (r) void useDebugStore.getState().load();
        return r?.path ?? null;
      }),
    openDir: () => debugOpenLogDir(),
    clear: () => run(async () => set({ status: await debugClearLogs() })),
  };
});
