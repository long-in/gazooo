"use client";

import { useState } from "react";

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
import { useT } from "@/lib/i18n/use-t";
import type { Preset } from "@/lib/tauri/presets";

const MAX_NAME_CHARS = 60;

/**
 * プリセット名の入力。新規保存と名前変更の両方に使う。
 * 同名のプリセットがある新規保存は、上書きであることを明示して確認する。
 */
export function PresetNameDialog({
  open,
  mode,
  initialName,
  presets,
  editingId,
  error,
  busy,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  mode: "save" | "rename";
  initialName: string;
  presets: Preset[];
  /** 名前変更の対象 ID(自分自身の名前との重複は衝突とみなさない)。 */
  editingId?: string;
  error?: string | null;
  busy?: boolean;
  onOpenChange: (open: boolean) => void;
  /** `overwriteId` は、同名の既存プリセットを上書きする場合の ID。 */
  onSubmit: (name: string, overwriteId?: string) => void;
}) {
  const t = useT();
  const [name, setName] = useState(initialName);

  const trimmed = name.trim();
  const same = presets.find((p) => p.name === trimmed && p.id !== editingId);
  const overwrite = mode === "save" && same !== undefined;
  const valid = trimmed !== "" && [...trimmed].length <= MAX_NAME_CHARS && (mode === "save" || !same);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (valid && !busy) onSubmit(trimmed, overwrite ? same?.id : undefined);
          }}
        >
          <DialogHeader>
            <DialogTitle>{mode === "save" ? t("preset.save") : t("preset.rename")}</DialogTitle>
            <DialogDescription className="sr-only">{t("preset.nameLabel")}</DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2">
            <Label htmlFor="preset-name">{t("preset.nameLabel")}</Label>
            <Input
              id="preset-name"
              autoFocus
              value={name}
              maxLength={MAX_NAME_CHARS}
              placeholder={t("preset.namePlaceholder")}
              aria-invalid={error ? true : undefined}
              onChange={(e) => setName(e.target.value)}
            />
            {overwrite && <p className="text-sm text-amber-600 dark:text-amber-400">{t("preset.overwriteNote")}</p>}
            {!overwrite && same && <p className="text-sm text-destructive">{t("preset.overwriteNote")}</p>}
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t("preset.cancel")}
            </Button>
            <Button type="submit" disabled={!valid || busy}>
              {mode === "rename" ? t("preset.renameButton") : overwrite ? t("preset.overwriteButton") : t("preset.saveButton")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
