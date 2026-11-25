/** 翻訳結果の反映のしかた(docs/34 §2.1)。 */
export type ApplyMode = "replace" | "append";

/**
 * 翻訳結果を、現在のプロンプトへ反映した新しいプロンプトを返す。
 * `replace` = 置き換え、`append` = 末尾にカンマ区切りで足す(現在が空なら置き換えと同じ)。
 */
export function applyTranslation(
  current: string,
  translated: string,
  mode: ApplyMode,
): string {
  const next = translated.trim();
  if (mode === "replace" || current.trim() === "") return next;
  if (next === "") return current;
  // 末尾の区切り(カンマ・読点・空白)を揃えて、重複させない
  const head = current.replace(/[\s,、]+$/u, "");
  return `${head}, ${next}`;
}
