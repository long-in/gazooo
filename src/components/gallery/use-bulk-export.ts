"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { onImageExportDone, onImageExportProgress } from "@/lib/tauri/events";
import { imageExportCancel, imageExportMany, imageExportOpenDir } from "@/lib/tauri/images";

/** 一括ダウンロード(docs/44)。進捗・結果のイベントは、選択を解除しても受け取れるよう画面全体で購読する。 */
export function useBulkExport() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [state, setState] = useState<{ jobId: string; done: number; total: number } | null>(null);
  const jobRef = useRef<string | null>(null);

  useEffect(() => {
    const offs: Promise<() => void>[] = [
      onImageExportProgress((p) => {
        if (p.jobId === jobRef.current) setState({ jobId: p.jobId, done: p.done, total: p.total });
      }),
      onImageExportDone((d) => {
        if (d.jobId !== jobRef.current) return;
        jobRef.current = null;
        setState(null);
        if (d.error) {
          toast.error(d.error);
          return;
        }
        const open = { label: t("gallery.export.openDir"), onClick: () => void imageExportOpenDir().catch(() => {}) };
        const text = d.cancelled
          ? `${t("gallery.export.cancelled")}(${d.saved})`
          : `${d.saved} ${t("gallery.export.saved")}`;
        if (d.failed.length > 0) {
          toast.warning(`${text} / ${d.failed.length} ${t("gallery.export.failed")}`, { action: open });
        } else {
          toast.success(text, { action: open });
        }
      }),
    ];
    return () => {
      for (const o of offs) void o.then((off) => off());
    };
  }, [t]);

  const start = useCallback(
    async (ids: string[]) => {
      try {
        const id = await imageExportMany(ids);
        if (id) {
          jobRef.current = id;
          setState({ jobId: id, done: 0, total: ids.length });
        }
      } catch (e) {
        toast.error(errorMessage(toAppError(e)));
      }
    },
    [errorMessage],
  );

  const cancel = useCallback(() => {
    if (jobRef.current) void imageExportCancel(jobRef.current).catch(() => {});
  }, []);

  return { state, start, cancel };
}
