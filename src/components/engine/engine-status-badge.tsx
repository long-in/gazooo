"use client";

import { Play, RefreshCw } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { cn } from "@/lib/utils";
import { useEngineProcessStore } from "@/stores/engine-process-store";
import { useEngineStore } from "@/stores/engine-store";

/** ステータスバーの ComfyUI 接続状態表示(docs/29 §4)。起動管理が有効なら、起動の状態も示す(docs/15 §9)。 */
export function EngineStatusBadge() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const status = useEngineStore((s) => s.status);
  const reconnecting = useEngineStore((s) => s.reconnecting);
  const reconnect = useEngineStore((s) => s.reconnect);
  const managed = useEngineProcessStore((s) => s.view?.config.enabled === true);
  const proc = useEngineProcessStore((s) => s.state);
  const procBusy = useEngineProcessStore((s) => s.busy);

  // 起動・停止の途中や異常終了は、接続状態より優先して示す
  const procLabel: MessageKey | null =
    managed && proc && (proc.status === "starting" || proc.status === "stopping" || proc.status === "crashed")
      ? (`engine.proc.status.${proc.status}` as MessageKey)
      : null;
  const canStart = managed && !status?.connected && (proc?.status === "stopped" || proc?.status === "crashed");

  const label = procLabel
    ? t(procLabel)
    : !status
      ? t("engine.checking")
      : status.connected
        ? t("engine.connected")
        : t("engine.disconnected");

  async function start() {
    try {
      await useEngineProcessStore.getState().start();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  return (
    <div className="flex items-center gap-1.5">
      <span
        aria-hidden
        className={cn(
          "size-2 shrink-0 rounded-full",
          !status && "bg-muted-foreground/50",
          status?.connected && "bg-success",
          status && !status.connected && "bg-destructive",
          (proc?.status === "starting" || proc?.status === "stopping") && managed && "animate-pulse bg-warning",
        )}
      />
      <span className="min-w-0 truncate" role="status" title={status?.url}>
        {label}
      </span>
      {canStart && (
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={t("engine.proc.start")}
          title={t("engine.proc.start")}
          disabled={procBusy}
          onClick={() => void start()}
        >
          <Play className="size-3" />
        </Button>
      )}
      <Button
        type="button"
        variant="ghost"
        size="icon-xs"
        aria-label={t("engine.reconnect")}
        title={t("engine.reconnect")}
        disabled={reconnecting}
        onClick={() => void reconnect()}
      >
        <RefreshCw className={cn("size-3", reconnecting && "animate-spin")} />
      </Button>
    </div>
  );
}
