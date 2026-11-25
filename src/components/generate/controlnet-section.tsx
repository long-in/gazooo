"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { ModelsLink } from "@/components/models/models-link";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { defaultControlnetUnit, supportsControlnet } from "@/lib/generation/defaults";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { type InputImage, inputImageGet, inputImagePick } from "@/lib/tauri/inputs";
import type { Preprocessor } from "@/lib/tauri/jobs";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";

import { Field, NumberInput, SliderNumber } from "./fields";
import { imageSrc } from "@/lib/image-src";

const ASPECT_TOLERANCE = 0.02;
const NO_NAMES: string[] = [];
const NO_PREPROCESSORS: Preprocessor[] = [];

/**
 * ControlNet(F-12。docs/10-assist-features.md §5・§8)。
 * 参照画像はファイル選択(Rust 側のダイアログ)か、ギャラリー詳細の「参照画像に使う」で設定する。
 */
export function ControlnetSection({ error }: { error: (field: string) => string | null }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const params = useFormStore((s) => s.params);
  const { setControlnetEnabled, patchControlnetUnit } = useFormStore.getState();
  const status = useEngineStore((s) => s.status);
  const controlnets = useEngineStore((s) => s.models?.controlnets) ?? NO_NAMES;

  const kind = params.model.kind;
  const cn = params.controlnet;
  const unit = cn?.units[0] ?? defaultControlnetUnit();
  const available: Preprocessor[] = status?.features.controlnetPreprocessors ?? NO_PREPROCESSORS;

  const connected = status?.connected === true;
  let reason: MessageKey | null = null;
  if (!supportsControlnet(kind)) reason = "generate.cn.unsupported";
  else if (connected && available.length === 0) reason = "generate.cn.nodeMissing";
  else if (connected && controlnets.length === 0) reason = "generate.cn.noModel";
  const on = cn?.enabled === true && reason === null;

  // 参照画像の情報(表示用)。復元(プリセット・再現)で参照画像が消えていたら、無効にして知らせる
  const [info, setInfo] = useState<InputImage | null>(null);
  const imagePath = unit.image;
  useEffect(() => {
    if (!imagePath) return;
    let cancelled = false;
    inputImageGet(imagePath)
      .then((img) => !cancelled && setInfo(img))
      .catch((e) => {
        if (cancelled) return;
        setInfo(null);
        if (toAppError(e).code === "IMAGE_NOT_FOUND") {
          useFormStore.getState().patchControlnetUnit({ image: "" });
          useFormStore.getState().setControlnetEnabled(false);
          toast.warning(t("generate.cn.imageMissing"));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [imagePath, t]);
  const shown = imagePath ? info : null;

  // 使えない前処理(controlnet_aux 未導入など)が選ばれていたら、使えるものに替える
  useEffect(() => {
    if (!connected || available.length === 0) return;
    if (unit.preprocessor !== "none" && !available.includes(unit.preprocessor)) {
      useFormStore.getState().patchControlnetUnit({ preprocessor: available[0] }, controlnets);
    }
  }, [connected, available, unit.preprocessor, controlnets]);

  // 使えるモデルが取れたら、未選択のモデルを埋める
  useEffect(() => {
    if (cn && unit.model === "" && controlnets.length > 0 && cn.units.length > 0) {
      useFormStore.getState().patchControlnetUnit({}, controlnets);
    }
  }, [cn, unit.model, controlnets]);

  async function pick() {
    try {
      const img = await inputImagePick();
      if (!img) return; // キャンセル
      setInfo(img);
      patchControlnetUnit({ image: img.path }, controlnets);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const preprocessorOptions: Preprocessor[] = [...available, "none"];
  const aspectDiffers =
    shown !== null &&
    Math.abs(shown.width / shown.height - params.width / params.height) / (params.width / params.height) >
      ASPECT_TOLERANCE;

  return (
    <section className="rounded-lg border p-3" aria-labelledby="cn-title">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 id="cn-title" className="text-sm font-medium">
            {t("generate.cn.title")}
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground">{t("generate.cn.desc")}</p>
        </div>
        <Switch
          aria-labelledby="cn-title"
          checked={on}
          disabled={reason !== null || !connected}
          onCheckedChange={(v) => setControlnetEnabled(v)}
        />
      </div>

      {reason && (
        <div className="mt-2 flex flex-col gap-1 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
          <p>{t(reason)}</p>
          {reason === "generate.cn.noModel" && <ModelsLink category="controlnet" kind={kind} />}
        </div>
      )}

      {on && (
        <div className="mt-3 flex flex-col gap-3">
          <Field
            label={t("generate.cn.image")}
            error={error("controlnet.units[0].image") ?? error("controlnet.units")}
            hint={t("generate.cn.galleryHint")}
          >
            <div className="flex items-center gap-3">
              {shown ? (
                <img
                  src={imageSrc(shown.filePath)}
                  alt={t("generate.cn.image")}
                  className="size-20 shrink-0 rounded-md border bg-muted/30 object-contain"
                />
              ) : (
                <div className="flex size-20 shrink-0 items-center justify-center rounded-md border border-dashed p-1 text-center text-[11px] text-muted-foreground">
                  {t("generate.cn.noImage")}
                </div>
              )}
              <div className="flex flex-col items-start gap-1">
                <Button type="button" variant="outline" size="sm" onClick={() => void pick()}>
                  {imagePath ? t("generate.cn.change") : t("generate.cn.pick")}
                </Button>
                {shown && (
                  <span className="text-xs text-muted-foreground">
                    {shown.width}×{shown.height}
                  </span>
                )}
              </div>
            </div>
            {aspectDiffers && <p className="text-xs text-muted-foreground">{t("generate.cn.crop")}</p>}
          </Field>

          <Field label={t("generate.cn.preprocessor")} htmlFor="cn-preprocessor">
            <Select
              value={unit.preprocessor}
              items={preprocessorOptions.map((p) => ({
                value: p,
                label: t(`generate.cn.preprocessor.${p}` as MessageKey),
              }))}
              onValueChange={(v) => v !== null && patchControlnetUnit({ preprocessor: v as Preprocessor }, controlnets)}
            >
              <SelectTrigger id="cn-preprocessor" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {preprocessorOptions.map((p) => (
                  <SelectItem key={p} value={p}>
                    {t(`generate.cn.preprocessor.${p}` as MessageKey)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {(!available.includes("depth") || !available.includes("openpose")) && connected && (
              <p className="text-xs text-muted-foreground">{t("generate.cn.auxHint")}</p>
            )}
          </Field>

          <Field
            label={t("generate.cn.model")}
            htmlFor="cn-model"
            hint={t("generate.cn.model.hint")}
            error={error("controlnet.units[0].model")}
          >
            <Select
              value={unit.model === "" ? null : unit.model}
              items={controlnets.map((m) => ({ value: m, label: m }))}
              onValueChange={(v) => v !== null && patchControlnetUnit({ model: v })}
            >
              <SelectTrigger id="cn-model" className="w-full" aria-invalid={error("controlnet.units[0].model") ? true : undefined}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {controlnets.map((m) => (
                  <SelectItem key={m} value={m}>
                    {m}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>

          <Field label={t("generate.cn.strength")} htmlFor="cn-strength" error={error("controlnet.units[0].strength")} help={t("help.cn.strength")}>
            <SliderNumber
              id="cn-strength"
              value={unit.strength}
              min={0}
              max={2}
              step={0.05}
              invalid={!!error("controlnet.units[0].strength")}
              onChange={(n) => patchControlnetUnit({ strength: Math.round(n * 100) / 100 })}
            />
          </Field>

          <div className="grid grid-cols-2 gap-3">
            <Field label={t("generate.cn.start")} htmlFor="cn-start">
              <NumberInput
                id="cn-start"
                value={unit.startPercent}
                min={0}
                max={1}
                step={0.05}
                onChange={(n) => patchControlnetUnit({ startPercent: n })}
              />
            </Field>
            <Field label={t("generate.cn.end")} htmlFor="cn-end" error={error("controlnet.units[0].endPercent")}>
              <NumberInput
                id="cn-end"
                value={unit.endPercent}
                min={0}
                max={1}
                step={0.05}
                invalid={!!error("controlnet.units[0].endPercent")}
                onChange={(n) => patchControlnetUnit({ endPercent: n })}
              />
            </Field>
          </div>
        </div>
      )}
    </section>
  );
}
