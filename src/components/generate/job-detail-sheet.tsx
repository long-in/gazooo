"use client";

import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import type { MessageKey } from "@/lib/i18n";
import { useT } from "@/lib/i18n/use-t";
import { durationMs, formatDuration } from "@/lib/queue";
import { isOutOfMemory } from "@/lib/tauri/errors";
import type { Job } from "@/lib/tauri/jobs";

/** 失敗の理由。コードに対応する説明を主に、エンジンの原文を小さく併記する。 */
function JobErrorNote({ code, message }: { code: string; message: string }) {
  const t = useT();
  const friendly = t(`error.${code}` as MessageKey); // 未知のコードは undefined(原文のみ表示)
  return (
    <div role="alert" className="flex flex-col gap-1 text-xs">
      {friendly && <p className="font-medium text-destructive">{friendly}</p>}
      {isOutOfMemory(message) && <p className="text-amber-600 dark:text-amber-400">{t("error.hint.oom")}</p>}
      <p className="break-words font-mono text-muted-foreground">{message}</p>
    </div>
  );
}

/** 有効な拡張機能の名前。 */
function extras(job: Job, t: (k: MessageKey) => string): string[] {
  const p = job.params;
  const out: string[] = [];
  if (p.faceSwap?.enabled) out.push(`${t("gallery.params.faceSwap")} (${p.faceSwap.region})`);
  if (p.reference?.enabled) out.push(`${t("gallery.params.reference")} (${p.reference.images.length})`);
  if (p.img2img?.enabled) out.push(`${t("gallery.params.img2img")} (denoise ${p.img2img.denoise})`);
  if (p.controlnet?.enabled) out.push("ControlNet");
  if (p.inpaint?.enabled) out.push(`Inpaint (denoise ${p.inpaint.denoise})`);
  if (p.faceDetailer?.enabled) out.push(`${t("gallery.params.face")} (denoise ${p.faceDetailer.denoise})`);
  return out;
}

/** キューのカードの詳細(docs/28 §3.3)。設定・所要時間・エラーの全文。 */
export function JobDetailSheet({ job, onClose }: { job: Job | null; onClose: () => void }) {
  const t = useT();
  const params = job?.params;
  const took = job ? durationMs(job) : null;
  const ext = job ? extras(job, t) : [];
  const faceSwapOn = params?.faceSwap?.enabled === true;

  return (
    <Sheet open={job !== null} onOpenChange={(open) => !open && onClose()}>
      <SheetContent side="right" className="w-full overflow-y-auto sm:max-w-xl">
        <SheetHeader>
          <SheetTitle>{t("queue.details")}</SheetTitle>
          <SheetDescription className="sr-only">{t("queue.details")}</SheetDescription>
        </SheetHeader>
        {job && params && (
          <div className="flex flex-col gap-3 px-4 pb-6">
            <p className="text-sm font-medium">{t(`status.${job.status}` as MessageKey)}</p>
            {job.error && <JobErrorNote code={job.error.code} message={job.error.message} />}
            <dl className="grid grid-cols-[7rem_1fr] gap-x-3 gap-y-2 text-sm">
              {!faceSwapOn && (
                <>
                  <dt className="text-muted-foreground">{t("generate.prompt")}</dt>
                  <dd className="whitespace-pre-wrap break-words">{params.prompt}</dd>
                </>
              )}
              {!faceSwapOn && params.negativePrompt && (
                <>
                  <dt className="text-muted-foreground">{t("generate.negative")}</dt>
                  <dd className="whitespace-pre-wrap break-words">{params.negativePrompt}</dd>
                </>
              )}
              <dt className="text-muted-foreground">{t("generate.model")}</dt>
              <dd className="break-all">
                {params.model.name} ({params.model.kind})
              </dd>
              <dt className="text-muted-foreground">{t("queue.detail.sampling")}</dt>
              <dd>
                {params.sampler} / {params.scheduler} · {params.steps} steps · CFG {params.cfg}
                {params.model.kind.startsWith("flux") && ` · guidance ${params.guidance}`}
              </dd>
              <dt className="text-muted-foreground">{t("generate.size")}</dt>
              <dd>
                {faceSwapOn
                  ? `seed ${params.seed}`
                  : `${params.width}×${params.height} · ${t("generate.batch")} ${params.batchSize} · seed ${params.seed}`}
              </dd>
              {ext.length > 0 && (
                <>
                  <dt className="text-muted-foreground">{t("queue.detail.extras")}</dt>
                  <dd>{ext.join(" / ")}</dd>
                </>
              )}
              {took !== null && (
                <>
                  <dt className="text-muted-foreground">{t("queue.detail.took")}</dt>
                  <dd>{formatDuration(took)}</dd>
                </>
              )}
            </dl>
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}
