"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { BackupSection } from "@/components/settings/backup-section";
import { LockSection } from "@/components/settings/lock-section";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { formatBytes } from "@/lib/models/catalog";
import { toAppError } from "@/lib/tauri/errors";
import { useDebugStore } from "@/stores/debug-store";

/** 設定 > その他: 調査用の詳細ログ(docs/31 §5)。操作はすぐ反映し、[保存] ボタンの対象にしない。 */
export function OtherSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const status = useDebugStore((s) => s.status);
  const busy = useDebugStore((s) => s.busy);
  const [clearOpen, setClearOpen] = useState(false);

  // タブを開いたとき(マウント時)に件数・サイズを取り直す。常時ポーリングはしない
  useEffect(() => {
    void useDebugStore.getState().load();
  }, []);

  async function attempt(op: () => Promise<void>) {
    try {
      await op();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const st = () => useDebugStore.getState();

  return (
    <div className="flex flex-col gap-8">
      <LockSection />
      <section className="flex flex-col gap-3">
        <h2 className="text-lg font-medium">{t("settings.debug.title")}</h2>

        <div className="flex items-start gap-3">
          <Switch
            id="debug-enabled"
            checked={status?.enabled ?? false}
            disabled={!status || busy}
            onCheckedChange={(v) => void attempt(() => st().setEnabled(v))}
          />
          <div className="flex flex-col gap-0.5">
            <Label htmlFor="debug-enabled">{t("settings.debug.enable")}</Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.debug.hint")}
            </p>
          </div>
        </div>

        {status && (
          <>
            <p className="text-sm" data-testid="debug-usage">
              {t("settings.debug.usage")
                .replace("{count}", String(status.fileCount))
                .replace("{size}", formatBytes(status.totalBytes))
                .replace("{limit}", formatBytes(status.limitBytes))}
            </p>
            <p className="text-xs break-all text-muted-foreground">
              {t("settings.debug.dir")}: {status.logDir}
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="outline"
                disabled={busy}
                onClick={() =>
                  void attempt(async () => {
                    const path = await st().exportLogs();
                    if (path)
                      toast.success(
                        t("settings.debug.exported").replace("{path}", path),
                      );
                  })
                }
              >
                {t("settings.debug.export")}
              </Button>
              <Button
                variant="outline"
                disabled={busy}
                onClick={() => void attempt(() => st().openDir())}
              >
                {t("settings.debug.openDir")}
              </Button>
              <Button
                variant="outline"
                disabled={busy}
                onClick={() => setClearOpen(true)}
              >
                {t("settings.debug.clear")}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              {t("settings.debug.exportNote")}
            </p>
          </>
        )}

        <ConfirmDialog
          open={clearOpen}
          title={t("settings.debug.clear.title")}
          description={t("settings.debug.clear.desc")}
          confirmLabel={t("settings.debug.clear.confirm")}
          cancelLabel={t("common.cancel")}
          onOpenChange={setClearOpen}
          onConfirm={() => {
            setClearOpen(false);
            void attempt(() => st().clear());
          }}
        />
      </section>
      <BackupSection />
    </div>
  );
}
