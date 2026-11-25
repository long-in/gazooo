"use client";

import { type ReactNode, useCallback, useState } from "react";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import type { Project, ProjectColor } from "@/lib/tauri/projects";
import { useProjectStore } from "@/stores/project-store";

import { ProjectNameDialog } from "./project-name-dialog";

type Dialog = { kind: "create"; onCreated?: (p: Project) => void } | { kind: "edit"; project: Project } | { kind: "delete"; project: Project };

/**
 * プロジェクトの作成・名前/色の変更・削除のダイアログ(docs/40 §5.4)。
 * 返り値の `element` を画面のどこかに描画し、`open` で開く。
 */
export function useProjectDialogs(): { open: (d: Dialog) => void; element: ReactNode } {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const open = useCallback((d: Dialog) => {
    setError(null);
    setDialog(d);
  }, []);
  const close = useCallback(() => setDialog(null), []);

  async function submit(name: string, color: ProjectColor | null) {
    if (!dialog || dialog.kind === "delete") return;
    setBusy(true);
    setError(null);
    try {
      const { create, update } = useProjectStore.getState();
      if (dialog.kind === "create") {
        const p = await create(name, color);
        toast.success(t("project.created"));
        dialog.onCreated?.(p);
      } else {
        await update(dialog.project.id, name, color);
      }
      close();
    } catch (e) {
      setError(errorMessage(toAppError(e)));
    } finally {
      setBusy(false);
    }
  }

  async function confirmDelete(project: Project) {
    close();
    try {
      await useProjectStore.getState().remove(project.id);
      toast.success(t("project.deleted"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const element = (
    <>
      {dialog && dialog.kind !== "delete" && (
        <ProjectNameDialog
          mode={dialog.kind}
          initialName={dialog.kind === "edit" ? dialog.project.name : ""}
          initialColor={dialog.kind === "edit" ? dialog.project.color : null}
          error={error}
          busy={busy}
          onOpenChange={(o) => !o && close()}
          onSubmit={(n, c) => void submit(n, c)}
        />
      )}
      <ConfirmDialog
        open={dialog?.kind === "delete"}
        title={t("project.deleteTitle")}
        description={t("project.deleteDesc")}
        confirmLabel={t("project.deleteConfirm")}
        cancelLabel={t("project.cancel")}
        onOpenChange={(o) => !o && close()}
        onConfirm={() => dialog?.kind === "delete" && void confirmDelete(dialog.project)}
      />
    </>
  );

  return { open, element };
}
