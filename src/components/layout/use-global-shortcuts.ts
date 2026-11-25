"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect } from "react";
import { toast } from "sonner";

import { useErrorMessage } from "@/lib/i18n/use-t";
import { detectMac, navTargetFor, shouldCancelOnEscape } from "@/lib/shortcuts";
import { toAppError } from "@/lib/tauri/errors";
import { useJobStore } from "@/stores/job-store";

const dialogIsOpen = () => document.querySelector('[role="dialog"], [role="alertdialog"]') !== null;

/**
 * アプリ全体のショートカット(docs/05-ui-design.md §3)。
 * - `Ctrl/Cmd + 1..5`: 画面切替
 * - `Esc`: 生成画面で、ダイアログが無いとき実行中のジョブを取り消す
 * (`Ctrl/Cmd + Enter` での生成は、フォームが自身で処理する)
 */
export function useGlobalShortcuts() {
  const router = useRouter();
  const pathname = usePathname();
  const errorMessage = useErrorMessage();

  useEffect(() => {
    const isMac = detectMac();

    function onKeyDown(e: KeyboardEvent) {
      if (e.defaultPrevented) return;

      const target = navTargetFor(e, isMac);
      if (target !== null) {
        if (dialogIsOpen()) return; // モーダルの背後で画面遷移しない
        e.preventDefault();
        router.push(target);
        return;
      }

      const running = useJobStore.getState().jobs.find((j) => j.status === "running");
      if (shouldCancelOnEscape(e, { pathname, dialogOpen: dialogIsOpen(), hasRunningJob: running !== undefined })) {
        e.preventDefault();
        useJobStore
          .getState()
          .cancel(running!.id)
          .catch((err) => toast.error(errorMessage(toAppError(err))));
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [router, pathname, errorMessage]);
}
