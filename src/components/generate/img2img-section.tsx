"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { ModelsLink } from "@/components/models/models-link";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { supportsImg2img } from "@/lib/generation/defaults";
import {
  DENOISE_LEVELS,
  type DenoiseLevel,
  defaultImg2img,
  isCompositionLocked,
  levelOf,
} from "@/lib/generation/img2img";
import { sizeForImage } from "@/lib/generation/inpaint";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { type InputImage, inputImageGet, inputImagePick } from "@/lib/tauri/inputs";
import type { Preprocessor } from "@/lib/tauri/jobs";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";

import { Field, SliderNumber } from "./fields";
import { imageSrc } from "@/lib/image-src";

const NO_PREPROCESSORS: Preprocessor[] = [];
const NO_NAMES: string[] = [];
const LEVELS = Object.keys(DENOISE_LEVELS) as DenoiseLevel[];

/**
 * 画像から生成(img2img。F-18。docs/19 §3・§7.2)。元画像を出発点に、強さの分だけ描き直す。
 * 「構図を固定する」は、ControlNet(Canny)を元画像で自動設定する。
 */
export function Img2imgSection({ error }: { error: (field: string) => string | null }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const kind = useFormStore((s) => s.params.model.kind);
  const stored = useFormStore((s) => s.params.img2img);
  const locked = useFormStore((s) => isCompositionLocked(s.params));
  const inpaintOn = useFormStore((s) => s.params.inpaint?.enabled === true);
  const { setImg2imgEnabled, patchImg2img, setImg2imgImage, setCompositionLock } = useFormStore.getState();
  const status = useEngineStore((s) => s.status);
  const controlnets = useEngineStore((s) => s.models?.controlnets) ?? NO_NAMES;

  const i2i = stored ?? defaultImg2img();
  const connected = status?.connected === true;
  const canny = (status?.features.controlnetPreprocessors ?? NO_PREPROCESSORS).includes("canny");
  let reason: MessageKey | null = null;
  if (!supportsImg2img(kind)) reason = "generate.img2img.unsupported";
  else if (connected && !status.features.img2img) reason = "generate.img2img.nodeMissing";
  else if (inpaintOn && !i2i.enabled) reason = "generate.img2img.inpaintOn";
  const on = i2i.enabled && reason === null;

  // 元画像の情報(表示用)。復元(プリセット・再現)で画像が消えていたら、無効にして知らせる
  const [info, setInfo] = useState<InputImage | null>(null);
  const imagePath = i2i.image;
  useEffect(() => {
    if (!imagePath) return;
    let cancelled = false;
    inputImageGet(imagePath)
      .then((img) => !cancelled && setInfo(img))
      .catch((e) => {
        if (cancelled) return;
        setInfo(null);
        if (toAppError(e).code === "IMAGE_NOT_FOUND") {
          useFormStore.getState().patchImg2img({ image: "", enabled: false });
          toast.warning(t("generate.img2img.imageMissing"));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [imagePath, t]);
  const shown = imagePath ? info : null;

  async function pick(): Promise<InputImage | null> {
    try {
      const img = await inputImagePick();
      if (!img) return null; // キャンセル
      setInfo(img);
      setImg2imgImage(img.path, img.width, img.height);
      return img;
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
      return null;
    }
  }

  const lockUnavailable = connected && (!canny || controlnets.length === 0);
  const sizeInexact = shown !== null && !sizeForImage(shown.width, shown.height).exact;
  const level = levelOf(i2i.denoise);

  return (
    <section className="rounded-lg border p-3" aria-labelledby="img2img-title">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 id="img2img-title" className="text-sm font-medium">
            {t("generate.img2img.title")}
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground">{t("generate.img2img.desc")}</p>
        </div>
        <Switch
          aria-labelledby="img2img-title"
          checked={on}
          disabled={reason !== null || !connected}
          onCheckedChange={(v) => setImg2imgEnabled(v)}
        />
      </div>

      {reason && (
        <div className="mt-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
          <p>{t(reason)}</p>
        </div>
      )}

      {on && (
        <div className="mt-3 flex flex-col gap-3">
          <p className="rounded-md bg-muted/50 p-2 text-xs text-muted-foreground">{t("generate.img2img.promptHint")}</p>

          <Field label={t("generate.img2img.image")} error={error("img2img.image")} hint={t("generate.img2img.galleryHint")}>
            <div className="flex items-center gap-3">
              {shown ? (
                <img
                  src={imageSrc(shown.filePath)}
                  alt={t("generate.img2img.image")}
                  className="size-20 shrink-0 rounded-md border bg-muted/30 object-contain"
                />
              ) : (
                <div className="flex size-20 shrink-0 items-center justify-center rounded-md border border-dashed p-1 text-center text-[11px] text-muted-foreground">
                  {t("generate.img2img.noImage")}
                </div>
              )}
              <div className="flex flex-col items-start gap-1">
                <Button type="button" variant="outline" size="sm" onClick={() => void pick()}>
                  {imagePath ? t("generate.img2img.change") : t("generate.img2img.pick")}
                </Button>
                {shown && (
                  <span className="text-xs text-muted-foreground">
                    {shown.width}×{shown.height}
                  </span>
                )}
              </div>
            </div>
            {sizeInexact && <p className="text-xs text-muted-foreground">{t("generate.img2img.sizeNote")}</p>}
          </Field>

          <Field
            label={t("generate.img2img.denoise")}
            htmlFor="img2img-denoise"
            hint={t("generate.img2img.denoise.hint")}
            help={t("help.img2img.denoise")}
            error={error("img2img.denoise")}
          >
            <div className="flex flex-wrap gap-1.5" role="group" aria-label={t("generate.img2img.denoise")}>
              {LEVELS.map((l) => (
                <Button
                  key={l}
                  type="button"
                  size="sm"
                  variant={level === l ? "default" : "outline"}
                  aria-pressed={level === l}
                  onClick={() => patchImg2img({ denoise: DENOISE_LEVELS[l] })}
                >
                  {t(`generate.img2img.level.${l}` as MessageKey)}
                </Button>
              ))}
            </div>
            <SliderNumber
              id="img2img-denoise"
              value={i2i.denoise}
              min={0.05}
              max={1}
              step={0.05}
              invalid={!!error("img2img.denoise")}
              onChange={(n) => patchImg2img({ denoise: Math.round(n * 100) / 100 })}
            />
          </Field>

          <div>
            <label className="flex items-start gap-2 text-sm">
              <input
                type="checkbox"
                className="mt-0.5 size-4 accent-primary"
                checked={locked}
                disabled={lockUnavailable || imagePath === ""}
                onChange={(e) => setCompositionLock(e.target.checked, controlnets)}
              />
              <span>
                {t("generate.img2img.lock")}
                <span className="block text-xs text-muted-foreground">{t("generate.img2img.lock.hint")}</span>
              </span>
            </label>
            {lockUnavailable && (
              <p className="mt-1 text-xs text-muted-foreground">
                {!canny ? t("generate.img2img.lock.unavailable") : t("generate.img2img.lock.noModel")}{" "}
                {canny && controlnets.length === 0 && <ModelsLink category="controlnet" kind={kind} />}
              </p>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
