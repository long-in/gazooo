import { create } from "zustand";

import type { AppError } from "@/lib/tauri/errors";
import { toAppError } from "@/lib/tauri/errors";
import {
  type UserSummary,
  userCreate,
  userCurrent,
  userDelete,
  userList,
  userLockClear,
  userLockSet,
  userLogin,
  userLogout,
  userRename,
  userSetColor,
  type LockSetInput,
} from "@/lib/tauri/user";

/**
 * ユーザーの状態(docs/36)。`loading`: 起動直後の確認中 / `gate`: ユーザー選択画面 / `in`: ログイン済み。
 *
 * ユーザーを切り替えるときはウィンドウをリロードし、メモリ上の状態(各ストア)を破棄する(docs/36 §5.4)。
 */
export type UserPhase = "loading" | "gate" | "in";

interface UserState {
  phase: UserPhase;
  users: UserSummary[];
  current: UserSummary | null;
  loadError: AppError | null;

  /** 起動時: ログイン済みか(通常は未ログイン)と、ユーザー一覧を取得する。 */
  init: () => Promise<void>;
  refresh: () => Promise<void>;
  /** 失敗時は `AppError` を throw する(呼び出し側で表示)。 */
  login: (id: string, secret?: string) => Promise<void>;
  create: (name: string, color: string) => Promise<UserSummary>;
  /** ログアウトして、ウィンドウをリロードする。 */
  switchUser: (cancelJobs?: boolean) => Promise<void>;
  rename: (id: string, name: string, secret?: string) => Promise<void>;
  setColor: (id: string, color: string) => Promise<void>;
  remove: (
    id: string,
    opts?: { secret?: string; confirmName?: string },
  ) => Promise<string | null>;
  setLock: (input: LockSetInput) => Promise<void>;
  clearLock: (current?: string) => Promise<void>;
}

/** テストで差し替えられるよう、リロードは関数として切り出す。 */
export const reloadWindow = { run: () => window.location.reload() };

export const useUserStore = create<UserState>((set, get) => ({
  phase: "loading",
  users: [],
  current: null,
  loadError: null,

  init: async () => {
    try {
      const [current, users] = await Promise.all([userCurrent(), userList()]);
      set({
        current,
        users,
        phase: current ? "in" : "gate",
        loadError: null,
      });
    } catch (e) {
      set({ loadError: toAppError(e), phase: "gate" });
    }
  },

  refresh: async () => {
    set({ users: await userList() });
  },

  login: async (id, secret) => {
    try {
      const user = await userLogin(id, secret);
      set({ current: user, phase: "in" });
    } catch (e) {
      // 失敗回数・待機が変わるため、一覧を取り直す(表示は失敗しても続ける)
      void get()
        .refresh()
        .catch(() => {});
      throw e;
    }
  },

  create: async (name, color) => {
    const user = await userCreate(name, color);
    await get().refresh();
    return user;
  },

  switchUser: async (cancelJobs = false) => {
    await userLogout(cancelJobs);
    reloadWindow.run();
  },

  rename: async (id, name, secret) => {
    const user = await userRename(id, name, secret);
    set((s) => ({
      users: s.users.map((u) => (u.id === id ? user : u)),
      current: s.current?.id === id ? user : s.current,
    }));
  },

  setColor: async (id, color) => {
    const user = await userSetColor(id, color);
    set((s) => ({
      users: s.users.map((u) => (u.id === id ? user : u)),
      current: s.current?.id === id ? user : s.current,
    }));
  },

  remove: async (id, opts) => {
    const result = await userDelete(id, opts);
    await get().refresh();
    return result.externalImagesDir;
  },

  setLock: async (input) => {
    const user = await userLockSet(input);
    set({ current: user });
    await get().refresh();
  },

  clearLock: async (current) => {
    const user = await userLockClear(current);
    set({ current: user });
    await get().refresh();
  },
}));
