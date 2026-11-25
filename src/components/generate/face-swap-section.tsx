"use client";

import { Plus, X } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  defaultFaceSwap,
  FACE_SWAP_STEPS,
  type FaceSwapQuality,
  estimateMinutes,
  hasFaceSwapPrompt,
  isSmallFaceImage,
  MAX_FACE_SWAP_TARGETS,
  qualityOf,
  supportsFaceSwap,
} from "@/lib/generation/face-swap";
import { REFERENCE_RESOLUTIONS, type ReferenceSize, sizeOf } from "@/lib/generation/reference";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { imageSrc } from "@/lib/image-src";
import { toAppError } from "@/lib/tauri/errors";
import { type InputImage, inputImageGet, inputImagePick, inputImagePickMany } from "@/lib/tauri/inputs";
import type { FaceSwapRegion } from "@/lib/tauri/jobs";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";

import { Field } from "./fields";

const SIZES = Object.keys(REFERENCE_RESOLUTIONS) as ReferenceSize[];
const QUALITIES = Object.keys(FACE_SWAP_STEPS) as FaceSwapQuality[];
const REGIONS: FaceSwapRegion[] = ["head", "face"];

/** 画像の情報を、パスから引く(表示用)。復元で画像が消えていたら `onMissing` を呼ぶ。 */
function useInputInfos(paths: string[], onMissing: (path: string) => void) {
  const [infos, setInfos] = useState<Record<string, InputImage>>({});
  const key = paths.join("\n");
  useEffect(() => {
    let cancelled = false;
    for (const path of key === "" ? [] : key.split("\n")) {
      inputImageGet(path)
        .then((img) => !cancelled && setInfos((m) => (m[path] ? m : { ...m, [path]: img })))
        .catch((e) => {
          if (!cancelled && toAppError(e).code === "IMAGE_NOT_FOUND") onMissing(path);
        });
    }
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return [infos, setInfos] as const;
}

/**
 * 顔の入れ替え(F-21。docs/42 §3・§6.2)。Qwen-Image 2.1 で、顔画像 1 枚と対象画像(複数枚)を指定し、
 * 対象画像の顔(頭)だけを指定の顔に書き換える。顔以外は元の画像のまま保てる(後段合成)。
 * 対象画像ごとに 1 ジョブがキューに入る(生成は生成ボタン。投入は generation-form)。
 */
export function FaceSwapSection({ error }: { error: (field: string) => string | null }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const kind = useFormStore((s) => s.params.model.kind);
  const stored = useFormStore((s) => s.params.faceSwap);
  const steps = useFormStore((s) => s.params.steps);
  const targets = useFormStore((s) => s.faceSwapTargets);
  const count = useFormStore((s) => s.count);
  const status = useEngineStore((s) => s.status);
  const form = useFormStore.getState();

  const fs = stored ?? defaultFaceSwap();
  const connected = status?.connected === true;
  const features = connected ? status.features : null;
  let reason: MessageKey | null = null;
  if (!supportsFaceSwap(kind)) reason = "generate.faceSwap.unsupported";
  else if (features && !features.referenceEdit) reason = "generate.faceSwap.nodeMissing";
  else if (features && !features.faceSwap) reason = "generate.faceSwap.modelMissing";
  const on = fs.enabled && reason === null;
  // 後段合成は、検出のノードがそろっているときだけ(無ければスイッチを無効にして案内する。docs/42 Q-331)
  const compositeAvailable = features?.faceSwapComposite === true;
  const keepOutside = fs.keepOutside && compositeAvailable;

  const [faceInfos, setFaceInfos] = useInputInfos(fs.face === "" ? [] : [fs.face], (path) => {
    if (useFormStore.getState().params.faceSwap?.face === path) useFormStore.getState().patchFaceSwap({ face: "" });
    toast.warning(t("generate.faceSwap.imageMissing"));
  });
  const [targetInfos, setTargetInfos] = useInputInfos(targets, (path) => {
    const s = useFormStore.getState();
    const at = s.faceSwapTargets.indexOf(path);
    if (at >= 0) s.removeFaceSwapTarget(at);
    toast.warning(t("generate.faceSwap.imageMissing"));
  });

  async function pickFace() {
    try {
      const img = await inputImagePick();
      if (!img) return; // キャンセル
      setFaceInfos((m) => ({ ...m, [img.path]: img }));
      useFormStore.getState().patchFaceSwap({ face: img.path });
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  async function pickTargets() {
    try {
      const imgs = await inputImagePickMany();
      if (imgs.length === 0) return;
      setTargetInfos((m) => ({ ...m, ...Object.fromEntries(imgs.map((i) => [i.path, i])) }));
      useFormStore.getState().addFaceSwapTargets(imgs.map((i) => i.path));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  /** 入れると、プロンプト・ステップ等が雛形に替わる。直前の値はトーストから戻せる(docs/21 §2.2 と同じ作り)。 */
  function toggle(enabled: boolean) {
    const before = useFormStore.getState().params;
    form.setFaceSwapEnabled(enabled);
    if (!enabled || hasFaceSwapPrompt(before.prompt)) return;
    toast(t("generate.faceSwap.templateSet"), {
      duration: 8000,
      action: {
        label: t("generate.reset.undo"),
        onClick: () =>
          useFormStore.getState().patch({
            prompt: before.prompt,
            steps: before.steps,
            cfg: before.cfg,
            batchSize: before.batchSize,
          }),
      },
    });
  }

  const size = sizeOf(fs.resolution);
  const quality = qualityOf(steps);

  return (
    <section className="rounded-lg border p-3" aria-labelledby="face-swap-title">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 id="face-swap-title" className="text-sm font-medium">
            {t("generate.faceSwap.title")}
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground">{t("generate.faceSwap.desc")}</p>
        </div>
        <Switch
          aria-labelledby="face-swap-title"
          checked={on}
          disabled={reason !== null || !connected}
          onCheckedChange={toggle}
        />
      </div>

      {reason && (
        <div className="mt-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
          <p>{t(reason)}</p>
        </div>
      )}

      {on && (
        <div className="mt-3 flex flex-col gap-3">
          <p className="rounded-md bg-muted/50 p-2 text-xs text-muted-foreground">{t("generate.faceSwap.consent")}</p>

          <Field label={t("generate.faceSwap.face")} error={error("faceSwap.face")} hint={t("generate.faceSwap.face.hint")}>
            <div className="flex items-end gap-3">
              {fs.face !== "" && (
                <div className="relative">
                  {faceInfos[fs.face] ? (
                    <img
                      src={imageSrc(faceInfos[fs.face].filePath)}
                      alt={t("generate.faceSwap.face")}
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
                    aria-label={t("generate.faceSwap.remove")}
                    title={t("generate.faceSwap.remove")}
                    onClick={() => form.patchFaceSwap({ face: "" })}
                  >
                    <X />
                  </Button>
                </div>
              )}
              <Button type="button" variant="outline" size="sm" onClick={() => void pickFace()}>
                {fs.face === "" ? t("generate.faceSwap.face.pick") : t("generate.faceSwap.face.change")}
              </Button>
            </div>
            {faceInfos[fs.face] && isSmallFaceImage(faceInfos[fs.face]) && (
              <p className="text-xs text-amber-600 dark:text-amber-400">
                {t("generate.faceSwap.face.small")
                  .replace("{w}", String(faceInfos[fs.face].width))
                  .replace("{h}", String(faceInfos[fs.face].height))}
              </p>
            )}
            <p className="text-xs text-muted-foreground">{t("generate.faceSwap.galleryHintFace")}</p>
          </Field>

          <Field
            label={`${t("generate.faceSwap.targets")}(${targets.length})`}
            error={error("faceSwap.target")}
            hint={t("generate.faceSwap.targets.hint").replace("{max}", String(MAX_FACE_SWAP_TARGETS))}
          >
            <ul className="flex flex-wrap gap-3">
              {targets.map((path, i) => {
                const info = targetInfos[path];
                return (
                  <li key={path} className="relative">
                    {info ? (
                      <img
                        src={imageSrc(info.filePath)}
                        alt={`${t("generate.faceSwap.targets")} ${i + 1}`}
                        className="size-20 rounded-md border bg-muted/30 object-contain"
                      />
                    ) : (
                      <div className="size-20 rounded-md border border-dashed" />
                    )}
                    <Button
                      type="button"
                      size="icon-xs"
                      variant="secondary"
                      className="absolute -right-2 -top-2 rounded-full"
                      aria-label={t("generate.faceSwap.remove")}
                      title={t("generate.faceSwap.remove")}
                      onClick={() => form.removeFaceSwapTarget(i)}
                    >
                      <X />
                    </Button>
                  </li>
                );
              })}
              {targets.length < MAX_FACE_SWAP_TARGETS && (
                <li>
                  <Button type="button" variant="outline" className="size-20 flex-col gap-1" onClick={() => void pickTargets()}>
                    <Plus />
                    <span className="text-xs">{t("generate.faceSwap.targets.add")}</span>
                  </Button>
                </li>
              )}
            </ul>
            {targets.length > 0 && (
              <Button type="button" variant="ghost" size="xs" className="self-start" onClick={() => form.clearFaceSwapTargets()}>
                {t("generate.faceSwap.targets.clear")}
              </Button>
            )}
            <p className="text-xs text-muted-foreground">{t("generate.faceSwap.galleryHintTarget")}</p>
          </Field>

          <Field label={t("generate.faceSwap.keepOutside")} hint={t("generate.faceSwap.keepOutside.hint")}>
            <div className="flex items-center gap-3">
              <Switch
                aria-label={t("generate.faceSwap.keepOutside")}
                checked={keepOutside}
                disabled={!compositeAvailable}
                onCheckedChange={(v) => form.patchFaceSwap({ keepOutside: v })}
              />
              {!compositeAvailable && (
                <p className="text-xs text-amber-600 dark:text-amber-400">{t("generate.faceSwap.compositeMissing")}</p>
              )}
            </div>
            {keepOutside && (
              <div className="flex flex-col gap-1.5">
                <div className="flex flex-wrap gap-1.5" role="group" aria-label={t("generate.faceSwap.region")}>
                  {REGIONS.map((r) => (
                    <Button
                      key={r}
                      type="button"
                      size="sm"
                      variant={fs.region === r ? "default" : "outline"}
                      aria-pressed={fs.region === r}
                      onClick={() => form.patchFaceSwap({ region: r })}
                    >
                      {t(`generate.faceSwap.region.${r}` as MessageKey)}
                    </Button>
                  ))}
                </div>
                <p className="text-xs text-muted-foreground">{t(`generate.faceSwap.region.${fs.region}.hint` as MessageKey)}</p>
              </div>
            )}
            {!keepOutside && compositeAvailable && (
              <p className="text-xs text-amber-600 dark:text-amber-400">{t("generate.faceSwap.outsideMayChange")}</p>
            )}
          </Field>

          <Field label={t("generate.faceSwap.quality")} hint={t("generate.faceSwap.quality.hint")}>
            <div className="flex flex-wrap gap-1.5" role="group" aria-label={t("generate.faceSwap.quality")}>
              {QUALITIES.map((q) => (
                <Button
                  key={q}
                  type="button"
                  size="sm"
                  variant={quality === q ? "default" : "outline"}
                  aria-pressed={quality === q}
                  onClick={() => form.patch({ steps: FACE_SWAP_STEPS[q] })}
                >
                  {t(`generate.faceSwap.quality.${q}` as MessageKey)} ({FACE_SWAP_STEPS[q]})
                </Button>
              ))}
            </div>
            {targets.length > 0 && (
              <p className="text-xs text-muted-foreground">
                {t("generate.faceSwap.estimate")
                  .replace("{targets}", String(targets.length))
                  .replace("{count}", String(count))
                  .replace("{min}", String(estimateMinutes(targets.length * count, steps)))}
              </p>
            )}
          </Field>

          <Field label={t("generate.reference.size")} hint={t("generate.reference.size.hint")} error={error("faceSwap.resolution")}>
            <div className="flex flex-wrap gap-1.5" role="group" aria-label={t("generate.reference.size")}>
              {SIZES.map((k) => (
                <Button
                  key={k}
                  type="button"
                  size="sm"
                  variant={size === k ? "default" : "outline"}
                  aria-pressed={size === k}
                  onClick={() => form.patchFaceSwap({ resolution: REFERENCE_RESOLUTIONS[k] })}
                >
                  {t(`generate.reference.size.${k}` as MessageKey)} {REFERENCE_RESOLUTIONS[k]}
                </Button>
              ))}
            </div>
          </Field>
        </div>
      )}
    </section>
  );
}
