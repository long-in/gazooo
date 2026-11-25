"use client";

import { BookmarkPlus } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";

import { Button, buttonVariants } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { BUILTIN_PRESETS } from "@/lib/generation/builtin-presets";
import { useErrorMessage, useLanguage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { cn } from "@/lib/utils";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";
import { usePresetStore } from "@/stores/preset-store";

import { PresetNameDialog } from "./preset-name-dialog";

/** 生成フォーム上部のプリセット操作(適用・保存・管理画面へのリンク)。 */
export function PresetBar() {
  const t = useT();
  const lang = useLanguage();
  const errorMessage = useErrorMessage();
  const presets = usePresetStore((s) => s.presets);
  const save = usePresetStore((s) => s.save);
  const params = useFormStore((s) => s.params);

  const [saveOpen, setSaveOpen] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function apply(id: string) {
    // 組み込み(コードの定数)。プロンプト・モデルの選択は保つ(docs/16 §5)
    const builtin = BUILTIN_PRESETS.find((p) => p.id === id);
    if (builtin) {
      useFormStore.getState().applyBuiltinPreset(builtin, useEngineStore.getState().models);
      toast.success(t("preset.applied"));
      return;
    }
    const preset = presets.find((p) => p.id === id);
    if (!preset) return;
    useFormStore.getState().applyPreset(preset.params);
    toast.success(t("preset.applied"));
  }

  async function submit(name: string, overwriteId?: string) {
    setBusy(true);
    setSaveError(null);
    try {
      await save({ id: overwriteId, name, params });
      setSaveOpen(false);
      toast.success(t("preset.saved"));
    } catch (e) {
      const err = toAppError(e);
      setSaveError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex items-center gap-2">
      <div className="min-w-0 flex-1">
        <Select
          // 常に未選択のままにして、同じプリセットを続けて選んでも再適用できるようにする
          value={null}
          items={[
            ...BUILTIN_PRESETS.map((p) => ({ value: p.id, label: `${p.name[lang]} ${t("preset.builtin.suffix")}` })),
            ...presets.map((p) => ({ value: p.id, label: p.name })),
          ]}
          onValueChange={(v) => v !== null && apply(v)}
        >
          <SelectTrigger className="w-full" aria-label={t("preset.apply")}>
            <SelectValue placeholder={t("preset.apply")} />
          </SelectTrigger>
          <SelectContent>
            {BUILTIN_PRESETS.map((p) => (
              <SelectItem key={p.id} value={p.id} title={p.description[lang]}>
                {p.name[lang]} {t("preset.builtin.suffix")}
              </SelectItem>
            ))}
            {presets.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <Button
        type="button"
        variant="outline"
        size="icon"
        aria-label={t("preset.save")}
        title={t("preset.save")}
        onClick={() => {
          setSaveError(null);
          setSaveOpen(true);
        }}
      >
        <BookmarkPlus />
      </Button>
      <Link href="/presets" className={cn(buttonVariants({ variant: "ghost", size: "sm" }))}>
        {t("preset.manage")}
      </Link>

      {/* 開くたびに入力欄を空の状態から始めるため、閉じている間はマウントしない */}
      {saveOpen && (
        <PresetNameDialog
          open
          mode="save"
          initialName=""
          presets={presets}
          error={saveError}
          busy={busy}
          onOpenChange={setSaveOpen}
          onSubmit={(name, overwriteId) => void submit(name, overwriteId)}
        />
      )}
    </div>
  );
}
