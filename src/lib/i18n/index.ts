import type { Language } from "@/lib/tauri/settings";

import { en } from "./en";
import { type MessageKey, ja } from "./ja";

const dictionaries: Record<Language, Record<MessageKey, string>> = { ja, en };

export type { MessageKey };

/** 設定が null(OS 追従)のときのロケール解決。 */
export function resolveLanguage(setting: Language | null): Language {
  if (setting) return setting;
  if (typeof navigator !== "undefined" && navigator.language.startsWith("ja")) {
    return "ja";
  }
  return typeof navigator === "undefined" ? "ja" : "en";
}

export function translate(lang: Language, key: MessageKey): string {
  return dictionaries[lang][key];
}
