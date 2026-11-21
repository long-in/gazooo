"use client";

import { useEffect, useState } from "react";
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
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import type { MessageKey } from "@/lib/i18n/ja";
import { formatBytes } from "@/lib/models/catalog";
import {
  type BackupInclude,
  type BackupManifest,
  type BackupOrphans,
  backupRestart,
} from "@/lib/tauri/backup";
import { toAppError } from "@/lib/tauri/errors";
import { onBackupProgress } from "@/lib/tauri/events";
import {
  BACKUP_CANCELLED,
  canExport,
  useBackupStore,
} from "@/stores/backup-store";

const fill = (text: string, values: Record<string, string | number>) =>
  Object.entries(values).reduce(
    (s, [k, v]) => s.replaceAll(`{${k}}`, String(v)),
    text,
  );

/** 設定 > その他: バックアップの取得と復元(docs/35 §4)。操作ボタンのみで、[保存] ボタンの対象にしない。 */
export function BackupSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const include = useBackupStore((s) => s.include);
  const includeApiKey = useBackupStore((s) => s.includeApiKey);
  const estimate = useBackupStore((s) => s.estimate);
  const busy = useBackupStore((s) => s.busy);
  const progress = useBackupStore((s) => s.progress);
  const [manifest, setManifest] = useState<BackupManifest | null>(null);
  const [restartOpen, setRestartOpen] = useState(false);

  // タブを開いたとき(マウント時)に見込みを取り直す。進捗イベントを購読する
  useEffect(() => {
    void useBackupStore.getState().loadEstimate();
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void onBackupProgress((p) => useBackupStore.getState().setProgress(p)).then(
      (u) => {
        if (cancelled) u();
        else unlisten = u;
      },
    );
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const st = () => useBackupStore.getState();

  function fail(e: unknown) {
    const err = toAppError(e);
    if (err.code === BACKUP_CANCELLED) return;
    toast.error(errorMessage(err));
  }

  async function onExport() {
    try {
      const r = await st().exportBackup();
      if (!r) return;
      toast.success(
        fill(t("settings.backup.exported"), {
          count: r.images,
          size: formatBytes(r.bytes),
        }),
      );
      if (r.skipped > 0)
        toast.warning(fill(t("settings.backup.skipped"), { count: r.skipped }));
    } catch (e) {
      fail(e);
    }
  }

  async function onPick() {
    try {
      const m = await st().inspect();
      if (m) setManifest(m);
    } catch (e) {
      fail(e);
    }
  }

  const gallerySize =
    estimate && include.gallery ? formatBytes(estimate.bytes) : null;
  const percent =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.done / progress.total) * 100))
      : 0;

  return (
    <section className="flex flex-col gap-3" data-testid="backup-section">
      <h2 className="text-lg font-medium">{t("settings.backup.title")}</h2>

      <div className="flex flex-col gap-3 rounded-md border border-border p-3">
        <h3 className="text-sm font-medium">
          {t("settings.backup.create.title")}
        </h3>
        <p className="text-xs text-muted-foreground">
          {t("settings.backup.create.desc")}
        </p>

        <SectionToggle
          id="backup-settings"
          checked={include.settings}
          disabled={busy !== null}
          label={t("settings.backup.include.settings")}
          desc={t("settings.backup.include.settingsDesc")}
          onChange={(v) => st().setInclude("settings", v)}
        >
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              data-testid="backup-api-key"
              checked={includeApiKey}
              disabled={!include.settings || busy !== null}
              onChange={(e) => st().setIncludeApiKey(e.target.checked)}
            />
            {t("settings.backup.apiKey")}
          </label>
          {includeApiKey && (
            <p className="text-xs text-destructive" role="alert">
              {t("settings.backup.apiKeyWarn")}
            </p>
          )}
        </SectionToggle>

        <SectionToggle
          id="backup-gallery"
          checked={include.gallery}
          disabled={busy !== null}
          label={t("settings.backup.include.gallery")}
          desc={t("settings.backup.include.galleryDesc")}
          onChange={(v) => st().setInclude("gallery", v)}
          meta={
            estimate
              ? fill(t("settings.backup.include.galleryCount"), {
                  count: estimate.images,
                  size: gallerySize ?? "—",
                })
              : undefined
          }
        />

        <SectionToggle
          id="backup-presets"
          checked={include.presets}
          disabled={busy !== null}
          label={t("settings.backup.include.presets")}
          desc={t("settings.backup.include.presetsDesc")}
          onChange={(v) => st().setInclude("presets", v)}
          meta={
            estimate
              ? fill(t("settings.backup.include.presetsCount"), {
                  count: estimate.presets,
                })
              : undefined
          }
        >
          {include.presets && !include.gallery && (
            <p className="text-xs text-muted-foreground">
              {t("settings.backup.presetsNote")}
            </p>
          )}
        </SectionToggle>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            disabled={busy !== null || !canExport(include)}
            onClick={() => void onExport()}
          >
            {t("settings.backup.export")}
          </Button>
          {busy === "export" && (
            <Button variant="outline" onClick={() => void st().cancel()}>
              {t("settings.backup.cancel")}
            </Button>
          )}
        </div>
        {busy === "export" && (
          <ProgressBar
            label={t("settings.backup.exporting")}
            percent={percent}
          />
        )}
      </div>

      <div className="flex flex-col gap-2 rounded-md border border-border p-3">
        <h3 className="text-sm font-medium">
          {t("settings.backup.restore.title")}
        </h3>
        <p className="text-xs text-muted-foreground">
          {t("settings.backup.restore.desc")}
        </p>
        <div>
          <Button
            variant="outline"
            disabled={busy !== null}
            onClick={() => void onPick()}
          >
            {t("settings.backup.restore.pick")}
          </Button>
        </div>
        {busy === "restore" && (
          <div className="flex flex-col gap-2">
            <ProgressBar
              label={t(
                `settings.backup.phase.${progress?.phase ?? "files"}` as MessageKey,
              )}
              percent={percent}
            />
            {progress?.phase !== "swap" && (
              <div>
                <Button variant="outline" onClick={() => void st().cancel()}>
                  {t("settings.backup.cancel")}
                </Button>
              </div>
            )}
          </div>
        )}
      </div>

      {manifest && (
        <RestoreDialog
          manifest={manifest}
          onClose={() => setManifest(null)}
          onDone={() => {
            setManifest(null);
            setRestartOpen(true);
          }}
          onError={fail}
        />
      )}

      <ConfirmDialog
        open={restartOpen}
        title={t("settings.backup.restart.title")}
        description={t("settings.backup.restart.desc")}
        confirmLabel={t("settings.backup.restart.confirm")}
        cancelLabel={t("settings.backup.restart.later")}
        destructive={false}
        onOpenChange={setRestartOpen}
        onConfirm={() => {
          setRestartOpen(false);
          void backupRestart().catch(fail);
        }}
      />
    </section>
  );
}

function ProgressBar({ label, percent }: { label: string; percent: number }) {
  return (
    <div className="flex flex-col gap-1" data-testid="backup-progress">
      <span className="text-xs text-muted-foreground">
        {label} {percent}%
      </span>
      <Progress value={percent} />
    </div>
  );
}

function SectionToggle({
  id,
  checked,
  disabled,
  label,
  desc,
  meta,
  onChange,
  children,
}: {
  id: string;
  checked: boolean;
  disabled?: boolean;
  label: string;
  desc: string;
  meta?: string;
  onChange: (v: boolean) => void;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-3">
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onChange}
      />
      <div className="flex flex-col gap-1">
        <Label htmlFor={id}>
          {label}
          {meta && (
            <span className="ml-2 text-xs font-normal text-muted-foreground">
              {meta}
            </span>
          )}
        </Label>
        <p className="text-xs text-muted-foreground">{desc}</p>
        {children}
      </div>
    </div>
  );
}

/** 復元する区分を選ぶ確認ダイアログ(docs/35 §4.2)。 */
function RestoreDialog({
  manifest,
  onClose,
  onDone,
  onError,
}: {
  manifest: BackupManifest;
  onClose: () => void;
  onDone: () => void;
  onError: (e: unknown) => void;
}) {
  const t = useT();
  const busy = useBackupStore((s) => s.busy);
  const [sel, setSel] = useState<BackupInclude>({
    settings: manifest.include.settings,
    gallery: manifest.include.gallery,
    presets: manifest.include.presets,
  });
  const [restoreApiKey, setRestoreApiKey] = useState(manifest.include.apiKey);
  const [deleteOrphans, setDeleteOrphans] = useState(false);
  const [orphans, setOrphans] = useState<BackupOrphans | null>(null);

  const any = sel.settings || sel.gallery || sel.presets;

  // 「バックアップに無い画像ファイルを削除する」の対象の件数・容量
  useEffect(() => {
    if (!sel.gallery) return;
    let live = true;
    void useBackupStore
      .getState()
      .orphans(sel)
      .then((o) => live && setOrphans(o))
      .catch(() => live && setOrphans(null));
    return () => {
      live = false;
    };
  }, [sel]);

  async function run() {
    try {
      await useBackupStore.getState().restore({
        include: sel,
        restoreApiKey: sel.settings && manifest.include.apiKey && restoreApiKey,
        // 実行中・待機中のジョブは、確認のうえ取り消してから復元する(ダイアログの注意書きで確認済み)
        stopJobs: true,
        deleteOrphans: sel.gallery && deleteOrphans,
      });
      onDone();
    } catch (e) {
      onError(e);
    }
  }

  const c = manifest.counts;
  const rows: { key: keyof BackupInclude; label: string; detail: string }[] = [
    {
      key: "settings",
      label: t("settings.backup.include.settings"),
      detail: fill(t("settings.backup.restore.counts.settings"), {
        settings: c.settings,
        conversations: c.conversations,
      }),
    },
    {
      key: "gallery",
      label: t("settings.backup.include.gallery"),
      detail: fill(t("settings.backup.restore.counts.gallery"), {
        images: c.images,
        jobs: c.jobs,
        size: formatBytes(manifest.bytes.images),
      }),
    },
    {
      key: "presets",
      label: t("settings.backup.include.presets"),
      detail: fill(t("settings.backup.include.presetsCount"), {
        count: c.presets,
      }),
    },
  ];

  return (
    <Dialog open onOpenChange={(o) => !o && busy === null && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("settings.backup.restore.dialogTitle")}</DialogTitle>
          <DialogDescription>
            {fill(t("settings.backup.restore.info"), {
              date: new Date(manifest.createdAt).toLocaleString(),
              version: manifest.appVersion,
            })}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          {rows.map((r) => {
            const available = manifest.include[r.key];
            return (
              <div key={r.key} className="flex items-start gap-3">
                <Switch
                  id={`restore-${r.key}`}
                  checked={available && sel[r.key]}
                  disabled={!available || busy !== null}
                  onCheckedChange={(v) => setSel({ ...sel, [r.key]: v })}
                />
                <div className="flex flex-col gap-0.5">
                  <Label htmlFor={`restore-${r.key}`}>{r.label}</Label>
                  <p className="text-xs text-muted-foreground">
                    {available
                      ? r.detail
                      : t("settings.backup.restore.notIncluded")}
                  </p>
                </div>
              </div>
            );
          })}

          {manifest.include.apiKey && sel.settings && (
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={restoreApiKey}
                disabled={busy !== null}
                onChange={(e) => setRestoreApiKey(e.target.checked)}
              />
              {t("settings.backup.restore.apiKey")}
            </label>
          )}

          {sel.gallery && (
            <div className="flex flex-col gap-1">
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  data-testid="restore-delete-orphans"
                  checked={deleteOrphans}
                  disabled={busy !== null}
                  onChange={(e) => setDeleteOrphans(e.target.checked)}
                />
                {t("settings.backup.restore.deleteOrphans")}
              </label>
              {orphans && (
                <p
                  className="text-xs text-muted-foreground"
                  data-testid="restore-orphans"
                >
                  {fill(t("settings.backup.restore.orphans"), {
                    count: orphans.count,
                    size: formatBytes(orphans.bytes),
                  })}
                </p>
              )}
            </div>
          )}

          {sel.settings && !sel.gallery && (
            <p
              className="text-xs text-destructive"
              role="alert"
              data-testid="restore-path-warn"
            >
              {t("settings.backup.restore.pathWarn")}
            </p>
          )}
          <p className="text-xs text-destructive" role="alert">
            {t("settings.backup.restore.warn")}
          </p>
        </div>

        <DialogFooter>
          <Button variant="outline" disabled={busy !== null} onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="destructive"
            disabled={!any || busy !== null}
            onClick={() => void run()}
          >
            {busy === "restore"
              ? t("settings.backup.restoring")
              : t("settings.backup.restore.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
