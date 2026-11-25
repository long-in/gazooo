"use client";

import { Pencil, Play, Trash2 } from "lucide-react";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import type { Preset } from "@/lib/tauri/presets";
import { useFormStore } from "@/stores/form-store";
import { usePresetStore } from "@/stores/preset-store";

import { PresetNameDialog } from "./preset-name-dialog";

export function PresetsView() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const router = useRouter();
  const presets = usePresetStore((s) => s.presets);
  const loaded = usePresetStore((s) => s.loaded);
  const { save, remove } = usePresetStore.getState();

  const [renaming, setRenaming] = useState<Preset | null>(null);
  const [renameError, setRenameError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<Preset | null>(null);

  function use(p: Preset) {
    useFormStore.getState().applyPreset(p.params);
    toast.success(t("preset.applied"));
    router.push("/");
  }

  async function rename(name: string) {
    if (!renaming) return;
    setRenameError(null);
    try {
      await save({ id: renaming.id, name, params: renaming.params });
      setRenaming(null);
      toast.success(t("preset.renamed"));
    } catch (e) {
      setRenameError(errorMessage(toAppError(e)));
    }
  }

  async function confirmDelete() {
    const target = deleting;
    setDeleting(null);
    if (!target) return;
    try {
      await remove(target.id);
      toast.success(t("preset.deleted"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6 p-8">
      <h1 className="sr-only">{t("preset.title")}</h1>

      {loaded && presets.length === 0 ? (
        <p className="rounded-xl border border-dashed p-10 text-center text-sm text-muted-foreground">{t("preset.empty")}</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {presets.map((p) => (
            <li key={p.id} className="flex items-center gap-3 rounded-xl border p-3">
              <div className="min-w-0 flex-1">
                <p className="truncate font-medium">{p.name}</p>
                <p className="truncate text-xs text-muted-foreground">
                  {t(`generate.kind.${p.params.model.kind}` as MessageKey)} · {p.params.width}×{p.params.height} · {p.params.steps} steps
                  {p.params.prompt && ` · ${p.params.prompt}`}
                </p>
              </div>
              <Button type="button" size="sm" onClick={() => use(p)}>
                <Play />
                {t("preset.use")}
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={t("preset.rename")}
                title={t("preset.rename")}
                onClick={() => {
                  setRenameError(null);
                  setRenaming(p);
                }}
              >
                <Pencil />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="text-destructive"
                aria-label={t("preset.delete")}
                title={t("preset.delete")}
                onClick={() => setDeleting(p)}
              >
                <Trash2 />
              </Button>
            </li>
          ))}
        </ul>
      )}

      {renaming && (
        <PresetNameDialog
          open
          mode="rename"
          initialName={renaming.name}
          presets={presets}
          editingId={renaming.id}
          error={renameError}
          onOpenChange={(open) => !open && setRenaming(null)}
          onSubmit={(name) => void rename(name)}
        />
      )}

      <ConfirmDialog
        open={deleting !== null}
        title={t("preset.deleteTitle")}
        description={t("preset.deleteDesc")}
        confirmLabel={t("preset.delete")}
        cancelLabel={t("preset.cancel")}
        onOpenChange={(open) => !open && setDeleting(null)}
        onConfirm={() => void confirmDelete()}
      />
    </div>
  );
}
