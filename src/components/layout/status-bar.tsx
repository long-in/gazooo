"use client";

import { Pause } from "lucide-react";
import Link from "next/link";

import { EngineStatusBadge } from "@/components/engine/engine-status-badge";
import { Progress } from "@/components/ui/progress";
import { useT } from "@/lib/i18n/use-t";
import { useDebugStore } from "@/stores/debug-store";
import { useJobStore } from "@/stores/job-store";

/** ステータスバー(docs/29 §4): エンジンの接続状態・実行中のジョブ・一時停止中。 */
export function StatusBar() {
  return (
    <footer className="flex h-6 shrink-0 items-center gap-4 border-t bg-panel-header px-3 text-[11px]">
      <EngineStatusBadge />
      <JobStatus />
      <DebugIndicator />
    </footer>
  );
}

/** デバッグログの出力中(付けっぱなしに気づけるように。docs/31 §5)。クリックで設定 > その他へ。 */
function DebugIndicator() {
  const t = useT();
  const enabled = useDebugStore((s) => s.status?.enabled === true);
  if (!enabled) return null;
  return (
    <Link
      href="/settings?scope=global&tab=other"
      title={t("statusbar.debug.title")}
      className="ml-auto rounded bg-warning/20 px-1.5 font-medium text-warning hover:bg-warning/30"
    >
      {t("statusbar.debug")}
    </Link>
  );
}

/** 実行状況(生成中・待機・一時停止中)。クリックで生成ページ(キューの帯)へ移る。 */
function JobStatus() {
  const t = useT();
  const runningId = useJobStore((s) => s.jobs.find((j) => j.status === "running")?.id);
  const running = runningId !== undefined;
  const waiting = useJobStore((s) => s.jobs.filter((j) => j.status === "queued").length);
  const paused = useJobStore((s) => s.paused);
  const progress = useJobStore((s) => (runningId ? s.progress[runningId] : undefined));

  if (!running && waiting === 0 && !paused) return null;

  const percent = progress && progress.totalSteps > 0 ? Math.round((progress.step / progress.totalSteps) * 100) : null;

  return (
    <Link href="/" title={t("jobbar.open")} aria-live="polite" className="flex min-w-0 items-center gap-2 hover:text-foreground">
      {paused && (
        <span className="flex items-center gap-1 font-medium text-warning">
          <Pause className="size-3" />
          {t("jobbar.paused")}
        </span>
      )}
      {running && (
        <>
          <span className="font-medium">
            {progress?.phase === "face_detail" ? t("generate.running.face") : t("generate.running")}
            {progress && (
              <span className="ml-1 font-normal text-muted-foreground">
                {progress.step}/{progress.totalSteps}
              </span>
            )}
          </span>
          <Progress value={percent} className="w-24" />
          {percent !== null && <span className="font-medium tabular-nums">{percent}%</span>}
        </>
      )}
      {waiting > 0 && (
        <span className="text-muted-foreground">
          {t("generate.waitingCount")}: {waiting}
        </span>
      )}
    </Link>
  );
}
