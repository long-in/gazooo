import type { AppError } from "@/lib/tauri/errors";

/** アバターの色(Rust 側の `PALETTE` と一致させる。docs/36 §3)。 */
export const USER_COLORS = [
  "red",
  "orange",
  "amber",
  "green",
  "teal",
  "blue",
  "violet",
  "pink",
] as const;

export type UserColor = (typeof USER_COLORS)[number];

/** Tailwind はクラス名を静的に拾うため、色ごとに完全な文字列を持つ。 */
const COLOR_CLASS: Record<UserColor, string> = {
  red: "bg-red-500 text-white",
  orange: "bg-orange-500 text-white",
  amber: "bg-amber-500 text-black",
  green: "bg-green-600 text-white",
  teal: "bg-teal-600 text-white",
  blue: "bg-blue-600 text-white",
  violet: "bg-violet-600 text-white",
  pink: "bg-pink-500 text-white",
};

export function colorClass(color: string): string {
  return COLOR_CLASS[color as UserColor] ?? COLOR_CLASS.blue;
}

/** アバターに出す頭文字(名前の最初の 1 文字。サロゲートペアも 1 文字として数える)。 */
export function initialOf(name: string): string {
  const first = Array.from(name.trim())[0];
  return first ? first.toUpperCase() : "?";
}

export const PIN_MIN = 4;
export const PIN_MAX = 8;
export const PASSPHRASE_MIN = 8;

/** キーボードの入力を PIN パッドの操作へ変換する(数字・テンキー・Backspace・Enter・Esc)。 */
export type PinKey =
  | { kind: "digit"; digit: string }
  | { kind: "backspace" }
  | { kind: "clear" }
  | { kind: "enter" };

export function pinKeyFromEvent(key: string): PinKey | null {
  if (/^[0-9]$/.test(key)) return { kind: "digit", digit: key };
  if (key === "Backspace") return { kind: "backspace" };
  if (key === "Escape") return { kind: "clear" };
  if (key === "Enter") return { kind: "enter" };
  return null;
}

/** 入力済みの PIN に操作を適用する。`length` を超える桁は入らない。 */
export function applyPinKey(value: string, key: PinKey, length: number): string {
  switch (key.kind) {
    case "digit":
      return value.length < length ? value + key.digit : value;
    case "backspace":
      return value.slice(0, -1);
    case "clear":
      return "";
    case "enter":
      return value;
  }
}

/** 待機の残り時間(ミリ秒)。待機中でなければ 0。 */
export function lockRemainingMs(lockedUntil: number | null, now: number): number {
  return lockedUntil && lockedUntil > now ? lockedUntil - now : 0;
}

/** 「1:05」のような残り時間の表示。 */
export function formatRemaining(ms: number): string {
  const total = Math.ceil(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/** エラーから待機の終了時刻を取り出す(`AUTH_LOCKED_OUT` の `retryAfterMs`)。 */
export function lockedUntilFromError(e: AppError, now: number): number | null {
  if (e.code !== "AUTH_LOCKED_OUT") return null;
  const ms = (e.details as { retryAfterMs?: number } | null | undefined)
    ?.retryAfterMs;
  return typeof ms === "number" ? now + ms : null;
}

/** 単純すぎる PIN(警告のみ。登録は妨げない)。 */
export function isWeakPin(pin: string): boolean {
  if (pin.length === 0) return false;
  if (/^(\d)\1+$/.test(pin)) return true;
  const asc = "0123456789";
  return asc.includes(pin) || [...asc].reverse().join("").includes(pin);
}
