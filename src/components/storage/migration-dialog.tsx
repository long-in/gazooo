"use client";

import { useEffect } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { formatBytes } from "@/lib/models/catalog";
import { toAppError } from "@/lib/tauri/errors";
import {
  type MigrationPlan,
  type MigrationReport,
  type MigrationTarget,
  onMigrationCloseRequested,
  onMigrationProgress,
  pickStorageDir,
  storageChangeImagesDir,
  storageMigrationQuit,
} from "@/lib/tauri/storage";
import { useEngineStore } from "@/stores/engine-store";
import { useGalleryStore } from "@/stores/gallery-store";
import { useMigrationStore } from "@/stores/migration-store";
import { useModelsStore } from "@/stores/models-store";
import { useSettingsStore } from "@/stores/settings-store";

/** 保存先が変わったあと、画面の情報を取り直す。 */
export async function reloadAfterStorageChange(target: MigrationTarget) {
  if (target !== "models") {
    await useSettingsStore.getState().load();
    // 画像の絶対パスが変わるため、表示中の一覧を取り直す
    void useGalleryStore.getState().reload();
  } else {
    await useModelsStore.getState().load();
    void useModelsStore.getState().loadEngineDir();
    void useEngineStore.getState().loadModels();
  }
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className={mono ? "break-all font-mono text-xs" : "text-sm"}>{value}</dd>
    </div>
  );
}

const warning = "rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs";

/** 空のパスは「アプリのデータ内(標準の場所)」。 */
function usePathLabel() {
  const t = useT();
  return (path: string) => path || t("settings.storage.default");
}

function PlanView({ plan, target }: { plan: MigrationPlan; target: MigrationTarget }) {
  const t = useT();
  const label = usePathLabel();
  const engine = plan.engine;
  const engineNote = !engine
    ? null
    : !engine.canApply
      ? `${t(`migration.engine.reason.${engine.reason ?? "engine_unknown"}`)} ${t("migration.engine.keepSource")}`
      : engine.managedRunning
        ? t("migration.engine.managed")
        : engine.externalRunning
          ? t("migration.engine.external")
          : null;

  return (
    <div className="flex flex-col gap-3">
      <dl className="flex flex-col gap-2">
        <Row label={t("migration.source")} value={target === "models" ? plan.sourceDir || "-" : label(plan.sourceDir)} mono />
        <Row label={t("migration.dest")} value={label(plan.destDir)} mono />
        <Row
          label={t("migration.files")}
          value={`${plan.fileCount} ${t("migration.filesUnit")} / ${formatBytes(plan.totalBytes)}`}
        />
        <Row
          label={t("migration.space")}
          value={`${t("migration.need")} ${formatBytes(plan.copyBytes)} / ${t("migration.free")} ${plan.freeBytes === null ? "-" : formatBytes(plan.freeBytes)}`}
        />
      </dl>
      {target === "imagesGlobal" && (
        <div className="flex flex-col gap-1 text-xs">
          <p>{`${t("migration.users")}: ${plan.users.length} / ${t("migration.excludedUsers")}: ${plan.excludedUsers}`}</p>
          <ul className="max-h-24 overflow-y-auto">
            {plan.users.map((u) => (
              <li key={u.name}>{`${u.name}: ${u.fileCount} ${t("migration.filesUnit")} / ${formatBytes(u.totalBytes)}`}</li>
            ))}
          </ul>
        </div>
      )}
      {plan.sameSizeCount > 0 && <p className="text-xs">{`${t("migration.sameSize")}: ${plan.sameSizeCount}`}</p>}
      {plan.missing > 0 && <p className="text-xs">{`${t("migration.missing")}: ${plan.missing}`}</p>}
      {plan.skippedFolders.length > 0 && (
        <p className="break-all text-xs">{`${t("migration.skippedFolders")}: ${plan.skippedFolders.join(", ")}`}</p>
      )}
      {plan.skippedLinks > 0 && <p className="text-xs">{`${t("migration.skippedLinks")}: ${plan.skippedLinks}`}</p>}
      {engineNote && <p className={warning}>{engineNote}</p>}
      {plan.blockers.length > 0 ? (
        <div role="alert" className="flex flex-col gap-1 text-sm text-destructive">
          {plan.blockers.map((b) => (
            <p key={b}>{t(`migration.blocker.${b}`)}</p>
          ))}
          {plan.conflicts.length > 0 && (
            <ul className="max-h-24 overflow-y-auto break-all font-mono text-xs">
              {plan.conflicts.map((c) => (
                <li key={c}>{c}</li>
              ))}
            </ul>
          )}
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">{t("migration.afterNote")}</p>
      )}
    </div>
  );
}

function ReportView({ report }: { report: MigrationReport }) {
  const t = useT();
  const label = usePathLabel();
  return (
    <div className="flex flex-col gap-3">
      <dl className="flex flex-col gap-2">
        <Row label={t("migration.current")} value={label(report.destDir)} mono />
        <Row label={t("migration.moved")} value={`${report.moved} ${t("migration.filesUnit")}`} />
      </dl>
      {report.missing > 0 && <p className="text-xs">{`${t("migration.missing")}: ${report.missing}`}</p>}
      {report.engineApplied === true && <p className="text-xs">{t("migration.engine.applied")}</p>}
      {report.engineApplied === false && (
        <p role="alert" className="text-sm text-destructive">{t("migration.engine.notApplied")}</p>
      )}
      {report.engineRestarted === true && <p className="text-xs">{t("migration.engine.restarted")}</p>}
      {report.engineRestarted === false && (
        <p role="alert" className="text-sm text-destructive">{t("migration.engine.restartFailed")}</p>
      )}
      {report.engineError && <p className="break-all text-xs text-muted-foreground">{report.engineError}</p>}
      {report.leftInSource > 0 ? (
        <div className={warning}>
          <p>{`${t("migration.leftInSource")}: ${report.leftInSource}`}</p>
          {report.leftDirs.map((d) => (
            <p key={d} className="break-all font-mono">
              {d}
            </p>
          ))}
        </div>
      ) : null}
      {report.moved > 0 && report.engineApplied !== false && (
        <p className="text-xs text-muted-foreground">{t("migration.trashNote")}</p>
      )}
    </div>
  );
}

/**
 * 保存先のデータ移行(docs/51 §3.2)。画面を移っても消えないよう、アプリの外枠に 1 つだけ置く。
 */
export function MigrationDialog() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const { step, target, newDir, plan, progress, report, error, cancelling, quitAsked } = useMigrationStore();
  const store = useMigrationStore.getState;

  useEffect(() => {
    const subscriptions = [
      onMigrationProgress((p) => useMigrationStore.getState().setProgress(p)),
      onMigrationCloseRequested(() => useMigrationStore.getState().askQuit(true)),
    ];
    return () => {
      for (const s of subscriptions) void s.then((off) => off());
    };
  }, []);

  async function pick() {
    try {
      const dir = await pickStorageDir();
      if (dir !== null) await store().choose(dir);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  async function run() {
    const result = await store().start();
    if (result) await reloadAfterStorageChange(target);
  }

  /** 画像の保存先を戻すとき、ファイルを移さずに設定だけ戻す。 */
  async function resetOnly() {
    try {
      await storageChangeImagesDir(target === "imagesGlobal" ? "global" : "user", null);
      store().close();
      await reloadAfterStorageChange(target);
      toast.success(t("settings.storage.changed"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const running = step === "running";
  const [done, total] =
    progress?.phase === "copying"
      ? [progress.doneBytes, progress.totalBytes]
      : [progress?.doneFiles ?? 0, progress?.totalFiles ?? 0];
  const percent = total > 0 ? Math.floor((done / total) * 100) : null;

  return (
    <Dialog open={step !== "closed"} onOpenChange={(open) => !open && store().close()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg" showCloseButton={!running}>
        <DialogHeader>
          <DialogTitle>{t(step === "done" ? "migration.doneTitle" : `migration.title.${target}`)}</DialogTitle>
          <DialogDescription>{t(step === "done" ? "migration.doneDesc" : "migration.desc")}</DialogDescription>
        </DialogHeader>

        {step === "planning" && (
          <p role="status" className="text-sm text-muted-foreground">
            {t("migration.planning")}
          </p>
        )}
        {step === "confirm" && plan && <PlanView plan={plan} target={target} />}
        {step === "done" && report && <ReportView report={report} />}
        {running && (
          <div role="status" className="flex min-w-0 flex-col gap-2">
            <p className="text-sm">{t(`migration.phase.${progress?.phase ?? "preparing"}` as MessageKey)}</p>
            <Progress value={percent} aria-label={t("migration.progress")} />
            {progress && (
              <p className="text-xs text-muted-foreground">
                {progress.phase === "copying"
                  ? `${formatBytes(progress.doneBytes)} / ${formatBytes(progress.totalBytes)} (${percent ?? 0}%)`
                  : `${progress.doneFiles} / ${progress.totalFiles}`}
              </p>
            )}
            {progress?.currentFile && (
              <p className="truncate font-mono text-xs text-muted-foreground">{progress.currentFile}</p>
            )}
            <p className="text-xs text-muted-foreground">{t("migration.keepOpen")}</p>
            {quitAsked && (
              <div role="alert" className={`flex flex-col gap-2 ${warning}`}>
                <p>{t("migration.quit.body")}</p>
                <div className="flex gap-2">
                  <Button variant="destructive" size="sm" onClick={() => void storageMigrationQuit()}>
                    {t("migration.quit.confirm")}
                  </Button>
                  <Button variant="outline" size="sm" onClick={() => store().askQuit(false)}>
                    {t("migration.quit.continue")}
                  </Button>
                </div>
              </div>
            )}
          </div>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(error)}
          </p>
        )}

        <DialogFooter>
          {running ? (
            <Button
              variant="outline"
              disabled={cancelling || (progress !== null && progress.phase !== "copying")}
              onClick={() => void store().cancel()}
            >
              {t(cancelling ? "migration.cancelling" : "migration.abort")}
            </Button>
          ) : step === "done" ? (
            <Button onClick={() => store().close()}>{t("migration.close")}</Button>
          ) : (
            <>
              <Button variant="outline" onClick={() => store().close()}>
                {t("preset.cancel")}
              </Button>
              {target !== "models" && newDir === null && step === "confirm" && (
                <Button variant="outline" onClick={() => void resetOnly()}>
                  {t("migration.resetOnly")}
                </Button>
              )}
              <Button
                variant={step === "pick" ? "default" : "outline"}
                disabled={step === "planning"}
                onClick={() => void pick()}
              >
                {t(step === "pick" ? "migration.pick" : "migration.repick")}
              </Button>
              {step === "confirm" && (
                <Button disabled={!plan || plan.blockers.length > 0} onClick={() => void run()}>
                  {t("migration.start")}
                </Button>
              )}
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
