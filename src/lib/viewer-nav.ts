/** ギャラリーのビューアの選択移動(docs/37 §4)。画像 ID の並びだけを扱う純関数。 */

export type ViewerMove = "prev" | "next" | "first" | "last";

/** 移動後の ID。端で止まる(ループしない)。現在の ID が一覧にない場合は先頭。空なら null。 */
export function moveViewer(ids: string[], current: string | null, move: ViewerMove): string | null {
  if (ids.length === 0) return null;
  if (move === "first") return ids[0];
  if (move === "last") return ids[ids.length - 1];
  const idx = current === null ? -1 : ids.indexOf(current);
  if (idx < 0) return ids[0];
  const target = move === "next" ? Math.min(ids.length - 1, idx + 1) : Math.max(0, idx - 1);
  return ids[target];
}

/** 一覧から画像が消えたあとの選択。消えていなければそのまま、消えたなら次(なければ前)、一覧が空なら null。 */
export function pickAfterRemoval(before: string[], after: string[], current: string | null): string | null {
  if (after.length === 0) return null;
  if (current !== null && after.includes(current)) return current;
  const idx = current === null ? -1 : before.indexOf(current);
  if (idx < 0) return after[0];
  const kept = new Set(after);
  for (let i = idx + 1; i < before.length; i++) if (kept.has(before[i])) return before[i];
  for (let i = idx - 1; i >= 0; i--) if (kept.has(before[i])) return before[i];
  return after[0];
}

/** キー入力をビューアの操作に対応づける。対象外・無効な状況は null。 */
export type ViewerKeyAction = ViewerMove | "open" | "favorite" | "favoriteOn" | "favoriteOff" | "delete";

/**
 * 修飾キー付きのショートカット(docs/37 §10)。macOS は ⌘ のみ、それ以外は Ctrl のみを受け付ける。
 * G = お気に入りに設定、C = 解除、Backspace = 削除。
 */
function modifiedAction(
  e: { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean },
  isMac: boolean,
): ViewerKeyAction | null {
  const mod = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!mod || e.altKey || e.shiftKey) return null;
  switch (e.key) {
    case "g":
    case "G":
      return "favoriteOn";
    case "c":
    case "C":
      return "favoriteOff";
    case "Backspace":
      return "delete";
    default:
      return null;
  }
}

export function viewerKeyAction(
  e: {
    key: string;
    isComposing?: boolean;
    ctrlKey?: boolean;
    metaKey?: boolean;
    altKey?: boolean;
    shiftKey?: boolean;
  },
  isMac = false,
): ViewerKeyAction | null {
  if (e.isComposing) return null;
  if (e.ctrlKey || e.metaKey || e.altKey) return modifiedAction(e, isMac);
  switch (e.key) {
    case "ArrowRight":
      return "next";
    case "ArrowLeft":
      return "prev";
    case "Home":
      return "first";
    case "End":
      return "last";
    case "Enter":
      return "open";
    case "f":
    case "F":
      return "favorite";
    case "Delete":
      return "delete";
    default:
      return null;
  }
}

/** キー操作を無視すべき対象(入力欄・ダイアログ内など)か。 */
export function isTypingTarget(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if ((el as HTMLElement).isContentEditable) return true;
  return el.closest('[role="dialog"], [role="alertdialog"], [role="menu"]') !== null;
}
