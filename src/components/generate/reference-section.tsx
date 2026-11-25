"use client";

import { ArrowLeft, ArrowRight, Plus, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  defaultReference,
  estimateOutputSize,
  imageTag,
  insertImageTag,
  MAX_REFERENCE_IMAGES,
  REFERENCE_RESOLUTIONS,
  type ReferenceSize,
  sizeOf,
  supportsReference,
} from "@/lib/generation/reference";
import { imageSrc } from "@/lib/image-src";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { type InputImage, inputImageGet, inputImagePick } from "@/lib/tauri/inputs";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";

import { Field } from "./fields";

const SIZES = Object.keys(REFERENCE_RESOLUTIONS) as ReferenceSize[];

/**
 * 参照画像(F-19。docs/24 §3・§6.2)。Qwen-Image 2.1 で、参照画像を 1〜4 枚与えて編集・合成する。
 * 1 枚目が編集される画像で、出力サイズも 1 枚目で決まる。プロンプトでは `<image1>` のように番号で指す。
 */
export function ReferenceSection({ error }: { error: (field: string) => string | null }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const kind = useFormStore((s) => s.params.model.kind);
  const stored = useFormStore((s) => s.params.reference);
  const status = useEngineStore((s) => s.status);
  const form = useFormStore.getState();

  const ref = stored ?? defaultReference();
  const connected = status?.connected === true;
  let reason: MessageKey | null = null;
  if (!supportsReference(kind)) reason = "generate.reference.unsupported";
  else if (connected && !status.features.referenceEdit) reason = "generate.reference.nodeMissing";
  const on = ref.enabled && reason === null;

  // 画像の情報(表示用)。復元(プリセット・再現)で画像が消えていたら、その画像を外して知らせる
  const [infos, setInfos] = useState<Record<string, InputImage>>({});
  const imagesKey = ref.images.join("\n");
  useEffect(() => {
    let cancelled = false;
    for (const path of imagesKey === "" ? [] : imagesKey.split("\n")) {
      inputImageGet(path)
        .then((img) => !cancelled && setInfos((m) => (m[path] ? m : { ...m, [path]: img })))
        .catch((e) => {
          if (cancelled || toAppError(e).code !== "IMAGE_NOT_FOUND") return;
          const s = useFormStore.getState();
          const at = (s.params.reference?.images ?? []).indexOf(path);
          if (at >= 0) s.removeReferenceImage(at);
          if ((useFormStore.getState().params.reference?.images ?? []).length === 0) s.setReferenceEnabled(false);
          toast.warning(t("generate.reference.imageMissing"));
        });
    }
    return () => {
      cancelled = true;
    };
  }, [imagesKey, t]);

  const first = ref.images[0] ? infos[ref.images[0]] : undefined;
  const estimate = useMemo(
    () => (first ? estimateOutputSize(first.width, first.height, ref.resolution) : null),
    [first, ref.resolution],
  );

  async function pick(): Promise<InputImage | null> {
    try {
      const img = await inputImagePick();
      if (!img) return null; // キャンセル
      setInfos((m) => ({ ...m, [img.path]: img }));
      return img;
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
      return null;
    }
  }

  async function add() {
    const img = await pick();
    if (img) useFormStore.getState().addReferenceImage(img.path, img.width, img.height);
  }

  /** `<imageN>` をプロンプトのカーソル位置へ入れる。 */
  function insertTag(n: number) {
    const el = document.getElementById("prompt");
    const area = el instanceof HTMLTextAreaElement ? el : null;
    const s = useFormStore.getState();
    const next = insertImageTag(
      s.params.prompt,
      n,
      area ? { start: area.selectionStart, end: area.selectionEnd } : null,
    );
    s.patch({ prompt: next.prompt });
    requestAnimationFrame(() => {
      area?.focus();
      area?.setSelectionRange(next.caret, next.caret);
    });
  }

  const size = sizeOf(ref.resolution);

  return (
    <section className="rounded-lg border p-3" aria-labelledby="reference-title">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 id="reference-title" className="text-sm font-medium">
            {t("generate.reference.title")}
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground">{t("generate.reference.desc")}</p>
        </div>
        <Switch
          aria-labelledby="reference-title"
          checked={on}
          disabled={reason !== null || !connected}
          onCheckedChange={(v) => form.setReferenceEnabled(v)}
        />
      </div>

      {reason && (
        <div className="mt-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
          <p>{t(reason)}</p>
        </div>
      )}

      {on && (
        <div className="mt-3 flex flex-col gap-3">
          <p className="rounded-md bg-muted/50 p-2 text-xs text-muted-foreground">{t("generate.reference.promptHint")}</p>

          <Field
            label={t("generate.reference.images")}
            error={error("reference.images")}
            hint={t("generate.reference.images.hint").replace("{max}", String(MAX_REFERENCE_IMAGES))}
          >
            <ul className="flex flex-wrap gap-3">
              {ref.images.map((path, i) => {
                const info = infos[path];
                return (
                  <li key={path} className="flex w-24 flex-col items-center gap-1">
                    <div className="relative">
                      {info ? (
                        <img
                          src={imageSrc(info.filePath)}
                          alt={imageTag(i + 1)}
                          className="size-24 rounded-md border bg-muted/30 object-contain"
                        />
                      ) : (
                        <div className="size-24 rounded-md border border-dashed" />
                      )}
                      <Button
                        type="button"
                        size="icon-xs"
                        variant="secondary"
                        className="absolute -right-2 -top-2 rounded-full"
                        aria-label={t("generate.reference.remove")}
                        title={t("generate.reference.remove")}
                        onClick={() => form.removeReferenceImage(i)}
                      >
                        <X />
                      </Button>
                    </div>
                    <Button
                      type="button"
                      size="xs"
                      variant="outline"
                      title={t("generate.reference.insertTag")}
                      onClick={() => insertTag(i + 1)}
                    >
                      {imageTag(i + 1)}
                    </Button>
                    <div className="flex items-center gap-1">
                      <Button
                        type="button"
                        size="icon-xs"
                        variant="ghost"
                        disabled={i === 0}
                        aria-label={t("generate.reference.moveEarlier")}
                        onClick={() => form.moveReferenceImage(i, -1)}
                      >
                        <ArrowLeft />
                      </Button>
                      <Button
                        type="button"
                        size="icon-xs"
                        variant="ghost"
                        disabled={i === ref.images.length - 1}
                        aria-label={t("generate.reference.moveLater")}
                        onClick={() => form.moveReferenceImage(i, 1)}
                      >
                        <ArrowRight />
                      </Button>
                    </div>
                    {i === 0 && <span className="text-center text-[11px] text-muted-foreground">{t("generate.reference.first")}</span>}
                  </li>
                );
              })}
              {ref.images.length < MAX_REFERENCE_IMAGES && (
                <li className="flex w-24 flex-col items-center">
                  <Button type="button" variant="outline" className="size-24 flex-col gap-1" onClick={() => void add()}>
                    <Plus />
                    <span className="text-xs">{t("generate.reference.add")}</span>
                  </Button>
                </li>
              )}
            </ul>
            <p className="text-xs text-muted-foreground">{t("generate.reference.galleryHint")}</p>
          </Field>

          <Field
            label={t("generate.reference.size")}
            hint={t("generate.reference.size.hint")}
            error={error("reference.resolution")}
          >
            <div className="flex flex-wrap gap-1.5" role="group" aria-label={t("generate.reference.size")}>
              {SIZES.map((k) => (
                <Button
                  key={k}
                  type="button"
                  size="sm"
                  variant={size === k ? "default" : "outline"}
                  aria-pressed={size === k}
                  onClick={() => form.setReferenceResolution(REFERENCE_RESOLUTIONS[k])}
                >
                  {t(`generate.reference.size.${k}` as MessageKey)} {REFERENCE_RESOLUTIONS[k]}
                </Button>
              ))}
            </div>
            {estimate && (
              <p className="text-xs text-muted-foreground">
                {t("generate.reference.estimate").replace("{w}", String(estimate.width)).replace("{h}", String(estimate.height))}
              </p>
            )}
          </Field>
        </div>
      )}
    </section>
  );
}
