"use client";

import { useCallback, useSyncExternalStore } from "react";

import { useSettingsStore } from "@/stores/settings-store";
import type { AppError } from "@/lib/tauri/errors";
import type { Language } from "@/lib/tauri/settings";

import { type MessageKey, translate } from "./index";

const subscribe = () => () => {};
const getClientLanguage = (): Language =>
  navigator.language.startsWith("ja") ? "ja" : "en";
// 静的エクスポートの事前レンダリング時は ja で固定し、ハイドレーション不一致を避ける
const getServerLanguage = (): Language => "ja";

export function useLanguage(): Language {
  const setting = useSettingsStore((s) => s.settings.language);
  const osLanguage = useSyncExternalStore(
    subscribe,
    getClientLanguage,
    getServerLanguage,
  );
  return setting ?? osLanguage;
}

export function useT() {
  const lang = useLanguage();
  return useCallback((key: MessageKey) => translate(lang, key), [lang]);
}

/** `AppError` をユーザー向けメッセージに変換する。 */
export function useErrorMessage() {
  const t = useT();
  return useCallback(
    (e: AppError) => {
      // 検証エラーは Rust 側がフィールド別の具体的な文言を持つ
      if (e.code === "VALIDATION") return e.message;
      const key = `error.${e.code}` as MessageKey;
      try {
        return t(key) ?? e.message;
      } catch {
        return e.message;
      }
    },
    [t],
  );
}
