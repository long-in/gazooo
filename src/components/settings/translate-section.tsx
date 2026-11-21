"use client";

import { useEffect } from "react";
import { toast } from "sonner";

import {
  MODEL_LABEL,
  PROVIDER_LABEL,
  PROVIDERS,
} from "@/components/settings/assistant-section";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import {
  ASSISTANT_MODELS,
  type AssistantProvider,
} from "@/lib/tauri/assistant";
import { toAppError } from "@/lib/tauri/errors";
import { useTranslateMtStore } from "@/stores/translate-mt-store";
import { useTranslateStore } from "@/stores/translate-store";
import { TranslateMtSection } from "@/components/settings/translate-mt-section";

/**
 * プロンプト翻訳の設定(docs/34 §4)。AI サポートとは別に、送信先とモデルを選ぶ。
 * キー・同意・ローカル LLM は AI サポートと共通。操作した時点で即時に保存する。
 */
export function TranslateSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const status = useTranslateStore((s) => s.status);
  const mt = useTranslateMtStore((s) => s.status);

  useEffect(() => {
    void useTranslateStore.getState().loadStatus();
    void useTranslateMtStore.getState().load();
  }, []);

  if (!status) return null;

  async function update(
    patch: Parameters<
      ReturnType<typeof useTranslateStore.getState>["update"]
    >[0],
  ) {
    try {
      await useTranslateStore.getState().update(patch);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.translate.title")}</h2>
      <p className="text-sm text-muted-foreground">
        {t("settings.translate.desc")}
      </p>

      <div className="flex flex-col gap-2">
        <Label>{t("settings.translate.provider")}</Label>
        <Select
          value={status.provider}
          items={PROVIDERS.map((p) => ({
            value: p,
            label: t(PROVIDER_LABEL[p]),
          }))}
          onValueChange={(v) =>
            v && void update({ provider: v as AssistantProvider })
          }
        >
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {PROVIDERS.map((p) => (
              <SelectItem key={p} value={p}>
                {t(PROVIDER_LABEL[p])}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {status.provider === "anthropic" && (
        <div className="flex flex-col gap-2">
          <Label>{t("settings.translate.model")}</Label>
          <Select
            value={status.model}
            items={ASSISTANT_MODELS.map((m) => ({
              value: m,
              label: t(MODEL_LABEL[m]),
            }))}
            onValueChange={(v) => v && void update({ model: v })}
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ASSISTANT_MODELS.map((m) => (
                <SelectItem key={m} value={m}>
                  {t(MODEL_LABEL[m])}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            {t("settings.translate.modelHint")}
          </p>
        </div>
      )}

      <div className="flex flex-col gap-2">
        <Label>{t("settings.translate.engine")}</Label>
        <div className="flex flex-wrap gap-2" role="radiogroup">
          {(["general", "dedicated"] as const).map((e) => {
            const disabled = e === "dedicated" && !mt?.installed;
            return (
              <button
                key={e}
                type="button"
                role="radio"
                aria-checked={status.mtModel === e}
                disabled={disabled}
                className={`rounded-md border px-2.5 py-1 text-sm disabled:opacity-50 ${status.mtModel === e ? "bg-primary text-primary-foreground" : "bg-background"}`}
                onClick={() => void update({ mtModel: e })}
              >
                {t(`settings.translate.engine.${e}` as const)}
              </button>
            );
          })}
        </div>
        <p className="text-xs text-muted-foreground">
          {t("settings.translate.engine.hint")}
          {!mt?.installed && ` ${t("settings.translate.engine.needInstall")}`}
        </p>
      </div>

      <TranslateMtSection />
    </section>
  );
}
