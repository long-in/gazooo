"use client";

import { useEffect, useState } from "react";

import { Progress } from "@/components/ui/progress";
import { useT } from "@/lib/i18n/use-t";
import { formatDuration, runningTiming } from "@/lib/queue";
import { useJobStore } from "@/stores/job-store";

/** 1 秒ごとに現在時刻を返す(実行中のときだけ更新する)。 */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const first = setTimeout(() => setNow(Date.now()), 0);
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      clearTimeout(first);
      clearInterval(id);
    };
  }, [active]);
  return now;
}

/** 帯の 1 行のサマリ(docs/28 §3.4): 実行中の進捗・経過・残りの目安と、待機件数。 */
export function QueueSummary() {
  const t = useT();
  const running = useJobStore((s) => s.jobs.find((j) => j.status === "running"));
  const waiting = useJobStore((s) => s.jobs.filter((j) => j.status === "queued").length);
  const progress = useJobStore((s) => (running ? s.progress[running.id] : undefined));
  const now = useNow(running !== undefined);

  const percent = progress && progress.totalSteps > 0 ? Math.round((progress.step / progress.totalSteps) * 100) : null;
  const timing = running ? runningTiming(running, progress, now) : null;

  return (
    <div aria-live="polite" className="flex min-w-0 items-center gap-3 text-xs">
      {running && timing ? (
        <>
          <span className="font-medium">
            {progress?.phase === "face_detail" ? t("generate.running.face") : t("generate.running")}
            {progress && (
              <span className="ml-1.5 font-normal text-muted-foreground">
                {progress.step} / {progress.totalSteps}
              </span>
            )}
          </span>
          <Progress value={percent} className="w-32" />
          {percent !== null && <span className="font-medium tabular-nums">{percent}%</span>}
          <span className="text-muted-foreground">
            {t("queue.elapsed")} {formatDuration(timing.elapsedMs)}
            {timing.remainingMs !== null && ` · ${t("queue.remaining")} ${formatDuration(timing.remainingMs)}`}
          </span>
        </>
      ) : (
        <span className="text-muted-foreground">{t("generate.idle")}</span>
      )}
      {waiting > 0 && <span className="text-muted-foreground">{t("library.queueCount").replace("{n}", String(waiting))}</span>}
    </div>
  );
}
