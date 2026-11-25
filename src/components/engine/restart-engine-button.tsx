"use client";

import { RotateCw } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { useEngineProcessStore } from "@/stores/engine-process-store";

/**
 * ComfyUI の再起動ボタン(docs/15 §5・§9)。
 * 実行中・待機中のジョブがある場合は、件数を示して確認してから再起動する。
 */
export function RestartEngineButton({
  variant = "outline",
  size = "sm",
  label,
}: {
  variant?: "default" | "outline";
  size?: "sm" | "default";
  label?: string;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const busy = useEngineProcessStore((s) => s.busy);
  const [confirm, setConfirm] = useState<{ running: number; queued: number } | null>(null);

  async function restart(force: boolean) {
    try {
      await useEngineProcessStore.getState().restart(force);
      setConfirm(null);
      toast.success(t("engine.proc.restarted"));
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "ENGINE_BUSY" && err.details?.running !== undefined) {
        setConfirm({ running: err.details.running, queued: err.details.queued ?? 0 });
        return;
      }
      setConfirm(null);
      toast.error(errorMessage(err));
    }
  }

  return (
    <>
      <Button type="button" variant={variant} size={size} disabled={busy} onClick={() => void restart(false)}>
        <RotateCw className="size-3.5" />
        {label ?? t("engine.proc.restart")}
      </Button>
      <ConfirmDialog
        open={confirm !== null}
        title={t("engine.proc.restartBusyTitle")}
        description={`${t("engine.proc.restartBusyBody")} (${t("engine.proc.running")}: ${confirm?.running ?? 0} / ${t("engine.proc.queued")}: ${confirm?.queued ?? 0})`}
        confirmLabel={t("engine.proc.restartForce")}
        cancelLabel={t("preset.cancel")}
        onOpenChange={(open) => !open && setConfirm(null)}
        onConfirm={() => void restart(true)}
      />
    </>
  );
}
