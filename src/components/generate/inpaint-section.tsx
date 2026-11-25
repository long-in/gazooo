"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { defaultInpaint, sizeForImage } from "@/lib/generation/inpaint";
import { supportsInpaint } from "@/lib/generation/defaults";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { type InputImage, inputImageGet, inputImagePick } from "@/lib/tauri/inputs";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";

import { Field, NumberInput, SliderNumber } from "./fields";
import { MaskEditor } from "./mask-editor";
import { imageSrc } from "@/lib/image-src";

/**
 * Inpaint(F-15。docs/13 §3)。参照画像にマスクを描き、塗った範囲だけを描き直す。
 * 参照画像はファイル選択(Rust 側のダイアログ)か、ギャラリー詳細の「Inpaint に使う」で設定する。
 */
export function InpaintSection({ error }: { error: (field: string) => string | null }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const kind = useFormStore((s) => s.params.model.kind);
  const stored = useFormStore((s) => s.params.inpaint);
  const faceOn = useFormStore((s) => s.params.faceDetailer?.enabled === true);
  const img2imgOn = useFormStore((s) => s.params.img2img?.enabled === true);
  const { setInpaintEnabled, patchInpaint, setInpaintImage } = useFormStore.getState();
  const status = useEngineStore((s) => s.status);

  const ip = stored ?? defaultInpaint();
  const connected = status?.connected === true;
  let reason: MessageKey | null = null;
  if (!supportsInpaint(kind)) reason = "generate.inpaint.unsupported";
  else if (connected && !status.features.inpaint) reason = "generate.inpaint.nodeMissing";
  else if (img2imgOn && !ip.enabled) reason = "generate.inpaint.img2imgOn";
  else if (faceOn && !ip.enabled) reason = "generate.face.inpaint";
  const on = ip.enabled && reason === null;

  // 画像の情報(表示・マスクエディタ用)。復元(プリセット・再現)で画像・マスクが消えていたら、無効にして知らせる
  const [info, setInfo] = useState<InputImage | null>(null);
  const [editing, setEditing] = useState(false);
  const imagePath = ip.image;
  const maskPath = ip.mask;
  useEffect(() => {
    if (!imagePath) return;
    let cancelled = false;
    const check = async () => {
      try {
        const img = await inputImageGet(imagePath);
        if (maskPath) await inputImageGet(maskPath);
        if (!cancelled) setInfo(img);
      } catch (e) {
        if (cancelled) return;
        setInfo(null);
        if (toAppError(e).code === "IMAGE_NOT_FOUND") {
          useFormStore.getState().patchInpaint({ image: "", mask: "", enabled: false });
          toast.warning(t("generate.inpaint.imageMissing"));
        }
      }
    };
    void check();
    return () => {
      cancelled = true;
    };
  }, [imagePath, maskPath, t]);
  const shown = imagePath ? info : null;

  async function pick() {
    try {
      const img = await inputImagePick();
      if (!img) return; // キャンセル
      setInfo(img);
      setInpaintImage(img.path, img.width, img.height);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const sizeInexact = shown !== null && !sizeForImage(shown.width, shown.height).exact;

  return (
    <section className="rounded-lg border p-3" aria-labelledby="inpaint-title">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 id="inpaint-title" className="text-sm font-medium">
            {t("generate.inpaint.title")}
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground">{t("generate.inpaint.desc")}</p>
        </div>
        <Switch
          aria-labelledby="inpaint-title"
          checked={on}
          disabled={reason !== null || !connected}
          onCheckedChange={(v) => setInpaintEnabled(v)}
        />
      </div>

      {reason && (
        <div className="mt-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
          <p>{t(reason)}</p>
        </div>
      )}

      {on && (
        <div className="mt-3 flex flex-col gap-3">
          <p className="rounded-md bg-muted/50 p-2 text-xs text-muted-foreground">{t("generate.inpaint.promptHint")}</p>

          <Field
            label={t("generate.inpaint.image")}
            error={error("inpaint.image")}
            hint={t("generate.inpaint.galleryHint")}
          >
            <div className="flex items-center gap-3">
              {shown ? (
                <img
                  src={imageSrc(shown.filePath)}
                  alt={t("generate.inpaint.image")}
                  className="size-20 shrink-0 rounded-md border bg-muted/30 object-contain"
                />
              ) : (
                <div className="flex size-20 shrink-0 items-center justify-center rounded-md border border-dashed p-1 text-center text-[11px] text-muted-foreground">
                  {t("generate.inpaint.noImage")}
                </div>
              )}
              <div className="flex flex-col items-start gap-1">
                <Button type="button" variant="outline" size="sm" onClick={() => void pick()}>
                  {imagePath ? t("generate.inpaint.change") : t("generate.inpaint.pick")}
                </Button>
                {shown && (
                  <span className="text-xs text-muted-foreground">
                    {shown.width}×{shown.height}
                  </span>
                )}
              </div>
            </div>
            {sizeInexact && <p className="text-xs text-muted-foreground">{t("generate.inpaint.sizeNote")}</p>}
          </Field>

          <Field label={t("generate.inpaint.mask")} error={error("inpaint.mask")}>
            <div className="flex items-center gap-3">
              <Button type="button" variant="outline" size="sm" disabled={!shown} onClick={() => setEditing(true)}>
                {maskPath ? t("generate.inpaint.editMask") : t("generate.inpaint.drawMask")}
              </Button>
              <span className="text-xs text-muted-foreground">
                {maskPath ? t("generate.inpaint.maskSet") : t("generate.inpaint.maskNone")}
              </span>
            </div>
          </Field>

          <Field
            label={t("generate.inpaint.denoise")}
            htmlFor="inpaint-denoise"
            hint={t("generate.inpaint.denoise.hint")}
            help={t("help.inpaint.denoise")}
            error={error("inpaint.denoise")}
          >
            <SliderNumber
              id="inpaint-denoise"
              value={ip.denoise}
              min={0.05}
              max={1}
              step={0.05}
              invalid={!!error("inpaint.denoise")}
              onChange={(n) => patchInpaint({ denoise: Math.round(n * 100) / 100 })}
            />
          </Field>

          <div className="grid grid-cols-2 gap-3">
            <Field label={t("generate.inpaint.grow")} htmlFor="inpaint-grow" error={error("inpaint.growMask")}>
              <NumberInput
                id="inpaint-grow"
                value={ip.growMask}
                min={0}
                max={64}
                step={1}
                invalid={!!error("inpaint.growMask")}
                onChange={(n) => patchInpaint({ growMask: Math.round(n) })}
              />
            </Field>
            <Field label={t("generate.inpaint.feather")} htmlFor="inpaint-feather" error={error("inpaint.feather")}>
              <NumberInput
                id="inpaint-feather"
                value={ip.feather}
                min={0}
                max={31}
                step={1}
                invalid={!!error("inpaint.feather")}
                onChange={(n) => patchInpaint({ feather: Math.round(n) })}
              />
            </Field>
          </div>

          <label className="flex items-start gap-2 text-sm">
            <input
              type="checkbox"
              className="mt-0.5 size-4 accent-primary"
              checked={ip.compositeBack}
              onChange={(e) => patchInpaint({ compositeBack: e.target.checked })}
            />
            <span>
              {t("generate.inpaint.composite")}
              <span className="block text-xs text-muted-foreground">{t("generate.inpaint.composite.hint")}</span>
            </span>
          </label>
        </div>
      )}

      {shown && (
        <MaskEditor
          open={editing}
          onOpenChange={setEditing}
          image={shown}
          existingMask={maskPath}
          onSaved={(path) => patchInpaint({ mask: path })}
        />
      )}
    </section>
  );
}
