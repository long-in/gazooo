/** キーボードショートカットの判定(docs/05-ui-design.md §3)。DOM に依存しない純粋関数。 */

/** `Ctrl/Cmd + 1..5` で切り替える画面。サイドナビの並びと同じ。 */
export const NAV_SHORTCUT_PATHS = ["/", "/gallery", "/presets", "/models", "/convert", "/settings"] as const;

export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

export function detectMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
}

/** 表示用の修飾キー(macOS は ⌘、それ以外は Ctrl+)。 */
export function modifierLabel(isMac: boolean): string {
  return isMac ? "⌘" : "Ctrl+";
}

/** `Ctrl/Cmd + 数字` に対応する遷移先。修飾キーが過不足なく一致する場合のみ。 */
export function navTargetFor(e: KeyLike, isMac: boolean): string | null {
  const mod = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!mod || e.altKey || e.shiftKey) return null;
  if (!/^[1-9]$/.test(e.key)) return null;
  return NAV_SHORTCUT_PATHS[Number(e.key) - 1] ?? null;
}

export interface CancelContext {
  pathname: string;
  /** `role=dialog` 等が開いているか(`Esc` はまずダイアログを閉じるために使われる)。 */
  dialogOpen: boolean;
  hasRunningJob: boolean;
}

/** `Esc` で実行中ジョブをキャンセルするか。生成画面で、ダイアログが無いときだけ。 */
export function shouldCancelOnEscape(e: KeyLike, ctx: CancelContext): boolean {
  if (e.key !== "Escape" || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return false;
  return ctx.pathname.replace(/\/+$/, "") === "" && !ctx.dialogOpen && ctx.hasRunningJob;
}
