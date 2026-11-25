"use client";

import { useSyncExternalStore } from "react";

/** ウィンドウ幅が `px` 以上か。サーバー側の事前レンダリングでは false(ハイドレーションの不一致を避ける)。 */
export function useMinWidth(px: number): boolean {
  return useSyncExternalStore(
    (cb) => {
      const mq = window.matchMedia(`(min-width: ${px}px)`);
      mq.addEventListener("change", cb);
      return () => mq.removeEventListener("change", cb);
    },
    () => window.matchMedia(`(min-width: ${px}px)`).matches,
    () => false,
  );
}
