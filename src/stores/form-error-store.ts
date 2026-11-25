import { create } from "zustand";

import type { AppError } from "@/lib/tauri/errors";

/**
 * 生成の投入で返った入力エラー(`VALIDATION`)。フォームと、別の場所に置いた拡張機能のパネルの両方で
 * 該当の項目にエラーを出すため、ストアで共有する。
 */
interface FormErrorState {
  error: AppError | null;
  setError: (error: AppError | null) => void;
}

export const useFormErrorStore = create<FormErrorState>((set) => ({
  error: null,
  setError: (error) => set({ error }),
}));

/** 項目名からエラーメッセージを引く関数(該当しなければ null)。 */
export function fieldErrorOf(error: AppError | null) {
  return (field: string): string | null =>
    error?.code === "VALIDATION" && error.details?.field === field ? error.message : null;
}
