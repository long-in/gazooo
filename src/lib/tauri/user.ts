import { invoke } from "./invoke";

/** ユーザー機能(docs/36、docs/04-ipc-api.md)。型は Rust 側が正。 */

export type AuthKind = "none" | "pin" | "passphrase";

export interface UserSummary {
  id: string;
  name: string;
  /** パレットのキー(`USER_COLORS`)。 */
  color: string;
  authKind: AuthKind;
  pinLength: number | null;
  /** 待機中のとき、この時刻(UTC ミリ秒)まで試行できない。 */
  lockedUntil: number | null;
  lastLoginAt: number | null;
}

export const userList = () => invoke<UserSummary[]>("user_list");

export const userCurrent = () => invoke<UserSummary | null>("user_current");

export const userCreate = (name: string, color: string) =>
  invoke<UserSummary>("user_create", { name, color });

/** ロックなしのユーザーは `secret` 不要。失敗は `AUTH_FAILED` / `AUTH_LOCKED_OUT`。 */
export const userLogin = (id: string, secret?: string) =>
  invoke<UserSummary>("user_login", { id, secret: secret ?? null });

/** 実行中のジョブがあり `cancelJobs` が false なら `USER_BUSY`。 */
export const userLogout = (cancelJobs = false) =>
  invoke<void>("user_logout", { cancelJobs });

export const userRename = (id: string, name: string, secret?: string) =>
  invoke<UserSummary>("user_rename", { id, name, secret: secret ?? null });

/** ログイン中の本人の色だけ変更できる(他人は `FORBIDDEN`)。 */
export const userSetColor = (id: string, color: string) =>
  invoke<UserSummary>("user_set_color", { id, color });

export interface DeleteResult {
  /** 作業スペースの外にあった画像保存先(自動では削除されない)。 */
  externalImagesDir: string | null;
}

export const userDelete = (
  id: string,
  opts: { secret?: string; confirmName?: string } = {},
) =>
  invoke<DeleteResult>("user_delete", {
    id,
    secret: opts.secret ?? null,
    confirmName: opts.confirmName ?? null,
  });

export interface LockSetInput {
  kind: Exclude<AuthKind, "none">;
  secret: string;
  pinLength?: number;
  /** 既存のロックがあるときの現在の値。 */
  current?: string;
}

export const userLockSet = (input: LockSetInput) =>
  invoke<UserSummary>("user_lock_set", {
    input: {
      kind: input.kind,
      secret: input.secret,
      pinLength: input.pinLength ?? null,
      current: input.current ?? null,
    },
  });

export const userLockClear = (current?: string) =>
  invoke<UserSummary>("user_lock_clear", { current: current ?? null });
