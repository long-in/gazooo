import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";

import { type AppError, toAppError } from "./errors";

/** Tauri コマンド呼び出しの唯一の入口。失敗時は常に `AppError` を throw する。 */
export async function invoke<T>(
  command: string,
  args?: Record<string, unknown> | Uint8Array,
): Promise<T> {
  if (!isTauri()) {
    const err: AppError = {
      code: "INTERNAL",
      message: "Tauri 環境の外では実行できません",
    };
    throw err;
  }
  try {
    return await tauriInvoke<T>(command, args);
  } catch (e) {
    throw toAppError(e);
  }
}
