"use client";

import { ModelsLink } from "@/components/models/models-link";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { defaultFaceDetailer, supportsFaceDetailer } from "@/lib/generation/defaults";
import { useT } from "@/lib/i18n/use-t";
import type { MessageKey } from "@/lib/i18n";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";

import { Field, NumberInput, SliderNumber } from "./fields";

/**
 * 顔補正(F-11。docs/10-assist-features.md §4・§8)。
 * スイッチで有効にすると設定が開く。使えない理由(非対応の種別・ノード/モデル未導入)は
 * スイッチを無効にして表示する。
 */
export function FaceDetailerSection({ error }: { error: (field: string) => string | null }) {
  const t = useT();
  const kind = useFormStore((s) => s.params.model.kind);
  const face = useFormStore((s) => s.params.faceDetailer) ?? defaultFaceDetailer();
  const patchFace = useFormStore((s) => s.patchFace);
  const status = useEngineStore((s) => s.status);
  const detectors = useEngineStore((s) => s.models?.faceDetectors) ?? [];

  const connected = status?.connected === true;
  let reason: MessageKey | null = null;
  if (!supportsFaceDetailer(kind)) reason = "generate.face.unsupported";
  else if (connected && !status.features.faceDetailer) reason = "generate.face.nodeMissing";
  else if (connected && detectors.length === 0) reason = "generate.face.noDetector";

  const on = face.enabled && reason === null;

  return (
    <section className="rounded-lg border p-3" aria-labelledby="face-title">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 id="face-title" className="text-sm font-medium">
            {t("generate.face.title")}
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground">{t("generate.face.desc")}</p>
        </div>
        <Switch
          aria-labelledby="face-title"
          checked={on}
          disabled={reason !== null || !connected}
          onCheckedChange={(v) => patchFace({ enabled: v })}
        />
      </div>

      {reason && (
        <div className="mt-2 flex flex-col gap-1 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
          <p>{t(reason)}</p>
          {reason === "generate.face.noDetector" && <ModelsLink category="detector" />}
        </div>
      )}

      {on && (
        <div className="mt-3 flex flex-col gap-3">
          <Field label={t("generate.face.detector")} htmlFor="face-detector" error={error("faceDetailer.detector")}>
            <Select
              value={face.detector === "" ? null : face.detector}
              items={detectors.map((d) => ({ value: d, label: d.replace(/^bbox\//, "") }))}
              onValueChange={(v) => v !== null && patchFace({ detector: v })}
            >
              <SelectTrigger id="face-detector" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {detectors.map((d) => (
                  <SelectItem key={d} value={d}>
                    {d.replace(/^bbox\//, "")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>

          <Field
            label={t("generate.face.denoise")}
            htmlFor="face-denoise"
            hint={t("generate.face.denoise.hint")}
            help={t("help.face.denoise")}
            error={error("faceDetailer.denoise")}
          >
            <SliderNumber
              id="face-denoise"
              value={face.denoise}
              min={0.05}
              max={1}
              step={0.05}
              invalid={!!error("faceDetailer.denoise")}
              onChange={(n) => patchFace({ denoise: Math.round(n * 100) / 100 })}
            />
          </Field>

          <Field label={t("generate.face.prompt")} htmlFor="face-prompt" hint={t("generate.face.prompt.hint")}>
            <Textarea
              id="face-prompt"
              rows={2}
              value={face.prompt}
              onChange={(e) => patchFace({ prompt: e.target.value })}
            />
          </Field>

          <details>
            <summary className="cursor-pointer text-sm">{t("generate.advanced")}</summary>
            <div className="mt-3">
              <Field
                label={t("generate.face.confidence")}
                htmlFor="face-confidence"
                error={error("faceDetailer.confidence")}
              >
                <NumberInput
                  id="face-confidence"
                  value={face.confidence}
                  min={0.1}
                  max={1}
                  step={0.05}
                  invalid={!!error("faceDetailer.confidence")}
                  onChange={(n) => patchFace({ confidence: n })}
                />
              </Field>
            </div>
          </details>
        </div>
      )}
    </section>
  );
}
