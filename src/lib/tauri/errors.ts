/** Rust 側 `AppError` のシリアライズ形式(docs/04-ipc-api.md)。 */
export interface AppError {
  code: string;
  message: string;
  details?: { field?: string; running?: number; queued?: number } | null;
}

export function isAppError(e: unknown): e is AppError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as AppError).code === "string" &&
    typeof (e as AppError).message === "string"
  );
}

export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  return {
    code: "INTERNAL",
    message: e instanceof Error ? e.message : String(e),
  };
}

/** エンジンの実行エラー文からメモリ不足を判定する(解像度を下げるヒントを出すため)。 */
export function isOutOfMemory(message: string): boolean {
  return /out of memory|\bOOM\b|insufficient memory|failed to allocate/i.test(message);
}
