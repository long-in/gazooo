"use client";

import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import {
  canProceed,
  addTotals,
  formatBytes,
  formatMb,
  isRunning,
  missingComponents,
  needsGpuConfirm,
  overallPercent,
  selectionTotals,
  stepPercent,
} from "@/lib/engine-install/logic";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { engineProcessStart } from "@/lib/tauri/engine-process";
import { useEngineInstallStore } from "@/stores/engine-install-store";
import { useEngineProcessStore } from "@/stores/engine-process-store";

type Screen = "check" | "consent" | "progress";

/**
 * ComfyUI のアプリ内インストール(docs/23 §7)。アプリ専用のフォルダに、Python・ComfyUI・PyTorch・
 * カスタムノードをすべて導入する。通信は、利用者が同意して開始したときだけ。
 */
export function EngineInstallSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const view = useEngineInstallStore((s) => s.view);
  const state = useEngineInstallStore((s) => s.state);
  const running = isRunning(state);

  const [open, setOpen] = useState(false);
  // 「追加で導入」のとき true(導入済みの部品は変えず、足りない部品だけを選ぶ)
  const [addMode, setAddMode] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const [keepData, setKeepData] = useState(true);

  useEffect(() => {
    void useEngineInstallStore.getState().load();
  }, []);

  if (!view) return null;
  if (!view.platform) {
    return (
      <section className="flex flex-col gap-2">
        <h2 className="text-lg font-medium">{t("engine.install.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("engine.install.unsupported")}</p>
      </section>
    );
  }

  const installed = view.record !== null;
  const missing = missingComponents(view);

  async function remove() {
    try {
      await useEngineInstallStore.getState().remove(keepData);
      toast.success(t("engine.install.removed"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("engine.install.title")}</h2>
      <p className="text-sm text-muted-foreground">{t("engine.install.desc")}</p>

      {installed ? (
        <div className="flex flex-col gap-2 rounded-lg border p-3 text-sm">
          <p className="font-medium">{t("engine.install.installed")}</p>
          <p className="text-muted-foreground">
            ComfyUI {view.record?.comfyuiTag} · {t("engine.install.manifest")} {view.record?.manifestVersion}
          </p>
          <p className="break-all font-mono text-xs text-muted-foreground">{view.installDir}</p>
          {missing.length > 0 && (
            <p className="text-muted-foreground">
              {t("engine.install.addAvailable")}
              {missing.map((c) => t(`engine.install.component.${c.id}` as MessageKey)).join("、")}
            </p>
          )}
          {view.updateAvailable && (
            <p className="text-amber-600 dark:text-amber-400">{t("engine.install.updateAvailable")}</p>
          )}
          <div className="flex flex-wrap gap-2">
            {view.updateAvailable && (
              <Button
                type="button"
                size="sm"
                disabled={running}
                onClick={() => {
                  setAddMode(false);
                  setOpen(true);
                }}
              >
                {t("engine.install.update")}
              </Button>
            )}
            {missing.length > 0 && (
              <Button
                type="button"
                size="sm"
                disabled={running}
                onClick={() => {
                  setAddMode(true);
                  setOpen(true);
                }}
              >
                {t("engine.install.add")}
              </Button>
            )}
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={running}
              onClick={() => {
                setAddMode(false);
                setOpen(true);
              }}
            >
              {t("engine.install.repair")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={running}
              onClick={() => setRemoveOpen(true)}
            >
              {t("engine.install.remove")}
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <Button type="button" disabled={running} onClick={() => setOpen(true)}>
              {running ? t("engine.install.running") : t("engine.install.start")}
            </Button>
            {state?.status === "failed" && (
              <span className="text-sm text-destructive">{t("engine.install.failedHint")}</span>
            )}
            {state?.status === "cancelled" && (
              <span className="text-sm text-muted-foreground">{t("engine.install.cancelledHint")}</span>
            )}
          </div>
          <p className="text-xs text-muted-foreground">{t("engine.install.useExisting")}</p>
        </div>
      )}

      {open && <InstallWizard addMode={addMode} onOpenChange={setOpen} />}

      <ConfirmDialog
        open={removeOpen}
        title={t("engine.install.remove.title")}
        description={t("engine.install.remove.desc")}
        confirmLabel={t("engine.install.remove.confirm")}
        cancelLabel={t("generate.reset.all.cancel")}
        onOpenChange={setRemoveOpen}
        onConfirm={() => void remove()}
      />
      {removeOpen && (
        <label className="-mt-1 flex items-center gap-2 text-sm">
          <Switch checked={keepData} onCheckedChange={setKeepData} />
          {t("engine.install.remove.keepData")}
        </label>
      )}
    </section>
  );
}

/** 開くたびに作り直す(状態を初期化するため、親が `open` のときだけ描画する)。 */
function InstallWizard({ addMode, onOpenChange }: { addMode: boolean; onOpenChange: (o: boolean) => void }) {
  const open = true;
  const t = useT();
  const errorMessage = useErrorMessage();
  const view = useEngineInstallStore((s) => s.view);
  const state = useEngineInstallStore((s) => s.state);
  const logs = useEngineInstallStore((s) => s.logs);
  const preflight = useEngineInstallStore((s) => s.preflight);
  const preflightLoading = useEngineInstallStore((s) => s.preflightLoading);

  const [screen, setScreen] = useState<Screen>(() =>
    useEngineInstallStore.getState().state?.status === "running" ? "progress" : "check",
  );
  const [selected, setSelected] = useState<string[] | null>(null);
  const [accepted, setAccepted] = useState(false);
  const [gpuOk, setGpuOk] = useState(false);
  const [starting, setStarting] = useState(false);

  const components = useMemo(() => view?.components ?? [], [view?.components]);
  // 既定はすべてオン。導入済みなら、導入した構成を初期値にする
  const ids = useMemo(
    () => selected ?? view?.record?.components ?? components.map((c) => c.id),
    [selected, view?.record, components],
  );

  // 事前チェックを取り直す(何もダウンロードしない)
  const idsKey = ids.join(",");
  useEffect(() => {
    if (screen === "check") {
      void useEngineInstallStore
        .getState()
        .runPreflight(idsKey === "" ? [] : idsKey.split(","))
        .catch(() => {});
    }
  }, [screen, idsKey]);

  if (!view) return null;
  const installedIds = view.record?.components ?? [];
  const hasNew = ids.some((id) => !installedIds.includes(id));
  const totals = addMode ? addTotals(view, installedIds, ids) : selectionTotals(view, ids);
  const status = state?.status ?? "idle";
  const running = status === "running";
  const gpuBlock = preflight !== null && needsGpuConfirm(preflight.gpu) && !gpuOk;
  const startable = canProceed(preflight) && !gpuBlock && (!addMode || hasNew);

  function toggle(id: string, on: boolean) {
    setSelected(on ? [...new Set([...ids, id])] : ids.filter((x) => x !== id));
  }

  async function start() {
    setStarting(true);
    try {
      await useEngineInstallStore.getState().start(ids);
      setScreen("progress");
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    } finally {
      setStarting(false);
    }
  }

  async function launch() {
    try {
      await engineProcessStart();
      void useEngineProcessStore.getState().load();
      onOpenChange(false);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const stepLabel = state?.stepId ? t(`engine.install.step.${state.stepId}` as MessageKey) : "";
  const pct = state ? overallPercent(state) : 0;
  const inStep = state ? stepPercent(state) : null;

  return (
    <Dialog open={open} onOpenChange={(o) => (!running || !o ? onOpenChange(o) : undefined)}>
      {/* グリッドの列幅を固定する(長いログの 1 行で、モーダルやバーが広がらないように) */}
      <DialogContent className="grid-cols-[minmax(0,1fr)] sm:max-w-xl" showCloseButton={!running}>
        <DialogHeader>
          <DialogTitle>{addMode ? t("engine.install.add.title") : t("engine.install.title")}</DialogTitle>
          <DialogDescription>{t(`engine.install.screen.${screen}` as MessageKey)}</DialogDescription>
        </DialogHeader>

        {screen === "check" && (
          <div className="flex max-h-[60vh] min-w-0 flex-col gap-3 overflow-y-auto text-sm">
            <div className="rounded-lg border p-3">
              <p className="font-medium">{t("engine.install.check.title")}</p>
              {preflightLoading && !preflight && <p className="text-muted-foreground">…</p>}
              {preflight && (
                <ul className="mt-1 flex flex-col gap-1 text-muted-foreground">
                  <li>
                    {preflight.supported ? "✓" : "✗"} {t("engine.install.check.os")}
                    {preflight.platform ? `(${preflight.platform})` : ""}
                    {!preflight.supported && preflight.unsupportedReason ? `: ${preflight.unsupportedReason}` : ""}
                  </li>
                  <li>
                    {preflight.enoughSpace ? "✓" : "✗"} {t("engine.install.check.space")}:{" "}
                    {preflight.freeBytes === null ? "?" : formatBytes(preflight.freeBytes)} /{" "}
                    {t("engine.install.check.required")} {formatBytes(preflight.requiredBytes)}
                  </li>
                  {preflight.gpu !== "notApplicable" && (
                    <li>
                      {preflight.gpu === "found" ? "✓" : "!"} {t("engine.install.check.gpu")}:{" "}
                      {preflight.gpu === "found"
                        ? (preflight.gpuName ?? "NVIDIA")
                        : t("engine.install.check.gpuMissing")}
                    </li>
                  )}
                  <li className="break-all">
                    {t("engine.install.check.dir")}: <span className="font-mono text-xs">{preflight.installDir}</span>
                  </li>
                </ul>
              )}
              {preflight && needsGpuConfirm(preflight.gpu) && (
                <label className="mt-2 flex items-center gap-2 text-amber-600 dark:text-amber-400">
                  <Switch checked={gpuOk} onCheckedChange={setGpuOk} />
                  {t("engine.install.check.gpuConfirm")}
                </label>
              )}
            </div>

            <div className="rounded-lg border p-3">
              <p className="font-medium">{t("engine.install.components")}</p>
              {!addMode && (
              <p className="mt-1 text-muted-foreground">
                {t("engine.install.base")} (Python {view.base.pythonVersion} · ComfyUI {view.base.comfyuiTag} · PyTorch{" "}
                {view.base.torchVersion}) — {formatMb(view.base.downloadMb)} / {formatMb(view.base.diskMb)}
              </p>
              )}
              {components.map((c) => (
                <label key={c.id} className="mt-2 flex items-start gap-2">
                  <Switch
                    checked={ids.includes(c.id)}
                    disabled={addMode && installedIds.includes(c.id)}
                    onCheckedChange={(on) => toggle(c.id, on)}
                  />
                  <span>
                    {t(`engine.install.component.${c.id}` as MessageKey)}
                    {addMode && installedIds.includes(c.id) && (
                      <span className="ml-1 text-xs text-muted-foreground">({t("engine.install.installedMark")})</span>
                    )}
                    <span className="block text-xs text-muted-foreground">
                      {c.name} — {formatMb(c.downloadMb)} / {formatMb(c.diskMb)}
                    </span>
                  </span>
                </label>
              ))}
              <p className="mt-2 text-xs text-muted-foreground">
                {t("engine.install.total")
                  .replace("{dl}", formatMb(totals.downloadMb))
                  .replace("{disk}", formatMb(totals.diskMb))}
              </p>
            </div>
          </div>
        )}

        {screen === "consent" && (
          <div className="flex max-h-[60vh] min-w-0 flex-col gap-3 overflow-y-auto text-sm">
            <div className="rounded-lg border p-3">
              <p className="font-medium">{t("engine.install.consent.hosts")}</p>
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{view.hosts.join(", ")}</p>
              <p className="mt-1 text-xs text-muted-foreground">{t("engine.install.consent.hostsNote")}</p>
            </div>
            <div className="rounded-lg border p-3">
              <p className="font-medium">{t("engine.install.consent.licenses")}</p>
              <ul className="mt-1 flex flex-col gap-1 text-muted-foreground">
                {!addMode && (
                  <>
                    <li>ComfyUI {view.base.comfyuiTag} — {view.base.comfyuiLicense}</li>
                    <li>PyTorch {view.base.torchVersion} — {view.base.torchLicense}</li>
                    <li>Python {view.base.pythonVersion} — PSF</li>
                  </>
                )}
                {components
                  .filter((c) => ids.includes(c.id) && !(addMode && installedIds.includes(c.id)))
                  .map((c) => (
                    <li key={c.id}>
                      {c.name} — {c.license}
                    </li>
                  ))}
              </ul>
              <p className="mt-2 text-xs text-muted-foreground">{t("engine.install.consent.note")}</p>
            </div>
            <label className="flex items-start gap-2">
              <Switch checked={accepted} onCheckedChange={setAccepted} />
              <span>{t("engine.install.consent.accept")}</span>
            </label>
          </div>
        )}

        {screen === "progress" && (
          <div className="flex min-w-0 flex-col gap-3 text-sm">
            <div className="min-w-0">
              <p className="font-medium">
                {status === "done"
                  ? t("engine.install.done")
                  : status === "failed"
                    ? t("engine.install.failed")
                    : status === "cancelled"
                      ? t("engine.install.cancelled")
                      : `${t("engine.install.step")} ${state?.step ?? 0}/${state?.totalSteps ?? 0}: ${stepLabel}`}
              </p>
              {running && (
                <Progress value={pct} className="mt-2">
                  <span className="text-xs text-muted-foreground">
                    {pct}%{inStep !== null ? `(${inStep}%)` : ""}
                  </span>
                </Progress>
              )}
              {status === "failed" && state?.error && (
                <p role="alert" className="mt-1 text-destructive">
                  {errorMessage({ code: state.error.code, message: state.error.message })}
                </p>
              )}
              {status === "done" && <p className="mt-1 text-muted-foreground">{t("engine.install.doneHint")}</p>}
            </div>
            <pre
              aria-label={t("engine.install.log")}
              className="h-48 min-w-0 max-w-full overflow-auto whitespace-pre-wrap break-all rounded-lg border bg-muted/40 p-2 font-mono text-[11px] leading-snug"
            >
              {logs.join("\n")}
            </pre>
            <div className="flex gap-2">
              <Button
                type="button"
                size="xs"
                variant="outline"
                onClick={() => void navigator.clipboard.writeText(logs.join("\n"))}
              >
                {t("engine.install.copyLog")}
              </Button>
            </div>
          </div>
        )}

        <DialogFooter>
          {screen === "check" && (
            <>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                {t("generate.reset.all.cancel")}
              </Button>
              <Button type="button" disabled={!startable} onClick={() => setScreen("consent")}>
                {t("engine.install.next")}
              </Button>
            </>
          )}
          {screen === "consent" && (
            <>
              <Button type="button" variant="outline" onClick={() => setScreen("check")}>
                {t("engine.install.back")}
              </Button>
              <Button type="button" disabled={!accepted || starting} onClick={() => void start()}>
                {t("engine.install.begin")}
              </Button>
            </>
          )}
          {screen === "progress" && (
            <>
              {running && (
                <Button type="button" variant="outline" onClick={() => void useEngineInstallStore.getState().cancel()}>
                  {t("engine.install.cancel")}
                </Button>
              )}
              {(status === "failed" || status === "cancelled") && (
                <>
                  <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                    {t("engine.install.close")}
                  </Button>
                  <Button type="button" onClick={() => void start()} disabled={starting}>
                    {t("engine.install.retry")}
                  </Button>
                </>
              )}
              {status === "done" && (
                <>
                  <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                    {t("engine.install.close")}
                  </Button>
                  <Button type="button" onClick={() => void launch()}>
                    {t("engine.install.launch")}
                  </Button>
                </>
              )}
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
