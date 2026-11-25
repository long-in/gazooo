"use client";

import Link from "next/link";

import { Button, buttonVariants } from "@/components/ui/button";
import { useT } from "@/lib/i18n/use-t";
import { cn } from "@/lib/utils";
import { useEngineInstallStore } from "@/stores/engine-install-store";
import { useEngineProcessStore } from "@/stores/engine-process-store";
import { useEngineStore } from "@/stores/engine-store";

/** 未接続時のバナー(docs/05-ui-design.md §6)。接続中・状態不明のときは何も出さない。 */
export function EngineBanner() {
  const t = useT();
  const status = useEngineStore((s) => s.status);
  const reconnecting = useEngineStore((s) => s.reconnecting);
  const reconnect = useEngineStore((s) => s.reconnect);

  // ComfyUI が無く、起動管理も未設定のときは、アプリ内のインストールを案内する(docs/23 §7。ダイアログは自動で開かない)
  const installView = useEngineInstallStore((s) => s.view);
  const managedEnabled = useEngineProcessStore((s) => s.view?.config.enabled);

  if (!status || status.connected) return null;
  const canInstall = Boolean(installView?.platform) && installView?.record === null && managedEnabled === false;

  const badResponse = status.error === "bad_response";
  return (
    <div
      role="alert"
      className="flex flex-col gap-3 rounded-xl border border-destructive/40 bg-destructive/5 p-4"
    >
      <div>
        <p className="font-medium text-destructive">
          {badResponse
            ? t("engine.banner.badResponse.title")
            : t("engine.banner.unreachable.title")}
        </p>
        <p className="mt-1 text-sm text-muted-foreground">
          {badResponse
            ? t("engine.banner.badResponse.desc")
            : t("engine.banner.unreachable.desc")}
        </p>
        <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{status.url}</p>
        {canInstall && <p className="mt-2 text-sm">{t("engine.install.banner")}</p>}
      </div>
      <div className="flex gap-2">
        <Button type="button" disabled={reconnecting} onClick={() => void reconnect()}>
          {t("engine.reconnect")}
        </Button>
        <Link href="/settings" className={cn(buttonVariants({ variant: "outline" }))}>
          {canInstall ? t("engine.install.openSettings") : t("engine.openSettings")}
        </Link>
      </div>
    </div>
  );
}
