"use client";

import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { storageChangeImagesDir } from "@/lib/tauri/storage";
import { useGalleryStore } from "@/stores/gallery-store";
import { useSettingsStore } from "@/stores/settings-store";

/**
 * 画像の保存先(docs/04-ipc-api.md「保存先の変更」)。
 * 変更は既存画像の扱いを伴うため、フォームの「保存」とは別に、確認ダイアログを経て即時に実行する。
 */
export function StorageSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const imagesDir = useSettingsStore((s) => s.settings.imagesDir);

  // 選択済みで、既存画像の扱いの決定待ちの保存先(null = 既定に戻す)
  const [pending, setPending] = useState<{ dir: string | null } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function pick() {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === "string") {
        setError(null);
        setPending({ dir: selected });
      }
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  async function run(moveExisting: boolean) {
    if (!pending) return;
    setBusy(true);
    setError(null);
    try {
      const report = await storageChangeImagesDir(pending.dir, moveExisting);
      await useSettingsStore.getState().load();
      // 画像の絶対パスが変わるため、表示中の一覧を取り直す
      void useGalleryStore.getState().reload();
      setPending(null);
      const detail = moveExisting
        ? ` (${report.moved} ${t("settings.storage.movedCount")}${report.missing > 0 ? `、${report.missing} ${t("settings.storage.missingCount")}` : ""})`
        : "";
      toast.success(`${t("settings.storage.changed")}${detail}`);
    } catch (e) {
      setError(errorMessage(toAppError(e)));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.storage.title")}</h2>
      <Label htmlFor="images-dir">{t("settings.storage.imagesDir")}</Label>
      <div className="flex gap-2">
        <Input id="images-dir" readOnly value={imagesDir ?? t("settings.storage.default")} />
        <Button type="button" variant="outline" onClick={() => void pick()}>
          {t("settings.storage.change")}
        </Button>
        <Button
          type="button"
          variant="ghost"
          disabled={imagesDir === null}
          onClick={() => {
            setError(null);
            setPending({ dir: null });
          }}
        >
          {t("settings.storage.reset")}
        </Button>
      </div>

      <Dialog open={pending !== null} onOpenChange={(open) => !open && !busy && setPending(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("settings.storage.moveTitle")}</DialogTitle>
            <DialogDescription>{t("settings.storage.moveDesc")}</DialogDescription>
          </DialogHeader>

          <div className="flex flex-col gap-3 text-sm">
            <div>
              <p className="text-xs text-muted-foreground">{t("settings.storage.newDir")}</p>
              <p className="break-all font-mono text-xs">{pending?.dir ?? t("settings.storage.default")}</p>
            </div>
            <div className="rounded-lg border p-3">
              <p className="font-medium">{t("settings.storage.moveYes")}</p>
              <p className="mt-1 text-xs text-muted-foreground">{t("settings.storage.moveYesNote")}</p>
            </div>
            <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 p-3">
              <p className="font-medium">{t("settings.storage.moveNo")}</p>
              <p className="mt-1 text-xs text-muted-foreground">{t("settings.storage.moveNoNote")}</p>
            </div>
            {busy && <p role="status" className="text-xs text-muted-foreground">{t("settings.storage.moveBusy")}</p>}
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" disabled={busy} onClick={() => setPending(null)}>
              {t("preset.cancel")}
            </Button>
            <Button type="button" variant="outline" disabled={busy} onClick={() => void run(false)}>
              {t("settings.storage.moveNo")}
            </Button>
            <Button type="button" disabled={busy} onClick={() => void run(true)}>
              {t("settings.storage.moveYes")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
