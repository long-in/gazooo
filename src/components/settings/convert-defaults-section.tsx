"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import {
  type ConvertSettings,
  type ConvertSettingsPatch,
  convertPickOutputDir,
  convertSettingsGet,
  convertSettingsUpdate,
  type OutFormat,
  type PngCompression,
} from "@/lib/tauri/convert";
import { toAppError } from "@/lib/tauri/errors";
import type { SettingsScope } from "@/lib/tauri/settings";

import { Overridable } from "./overridable";

const FORMATS: { value: OutFormat; label: string }[] = [
  { value: "png", label: "PNG" },
  { value: "jpeg", label: "JPEG" },
  { value: "webp", label: "WebP" },
];

const PNG_LEVELS: { value: PngCompression; key: MessageKey }[] = [
  { value: "fast", key: "convert.png.fast" },
  { value: "default", key: "convert.png.default" },
  { value: "best", key: "convert.png.best" },
];

function Fields({ scope }: { scope: SettingsScope }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [settings, setSettings] = useState<ConvertSettings | null>(null);

  useEffect(() => {
    void convertSettingsGet(scope === "global" ? "global" : undefined)
      .then(setSettings)
      .catch(() => {});
  }, [scope]);

  if (!settings) return null;

  async function apply(op: () => Promise<ConvertSettings | null>) {
    try {
      const next = await op();
      if (next) setSettings(next);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }
  const save = (patch: ConvertSettingsPatch) => void apply(() => convertSettingsUpdate(patch, scope));

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        {FORMATS.map((f) => (
          <Button
            key={f.value}
            size="sm"
            aria-pressed={settings.format === f.value}
            variant={settings.format === f.value ? "default" : "outline"}
            onClick={() => save({ format: f.value })}
          >
            {f.label}
          </Button>
        ))}
      </div>
      {settings.format === "png" ? (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm">{t("settings.convert.pngCompression")}</span>
          {PNG_LEVELS.map((l) => (
            <Button
              key={l.value}
              size="sm"
              aria-pressed={settings.pngCompression === l.value}
              variant={settings.pngCompression === l.value ? "default" : "outline"}
              onClick={() => save({ pngCompression: l.value })}
            >
              {t(l.key)}
            </Button>
          ))}
        </div>
      ) : (
        <div className="flex items-center gap-2">
          <Label htmlFor="convert-quality">{t("settings.convert.quality")}</Label>
          <Input
            id="convert-quality"
            type="number"
            min={1}
            max={100}
            className="w-24"
            defaultValue={settings.quality}
            onBlur={(e) => {
              const quality = Number(e.target.value);
              if (quality !== settings.quality) save({ quality });
            }}
          />
        </div>
      )}
      {settings.format === "webp" && (
        <label className="flex items-center gap-2 text-sm">
          <Switch checked={settings.webpLossless} onCheckedChange={(v) => save({ webpLossless: v })} />
          {t("settings.convert.lossless")}
        </label>
      )}
      <div className="flex items-center gap-2">
        <Label htmlFor="convert-background">{t("settings.convert.background")}</Label>
        <input
          id="convert-background"
          type="color"
          value={settings.background}
          onChange={(e) => save({ background: e.target.value })}
        />
      </div>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1 text-xs" title={settings.outputDir}>
          {settings.outputDir}
        </code>
        <Button size="sm" variant="outline" onClick={() => void apply(() => convertPickOutputDir(scope))}>
          {t("settings.convert.pickDir")}
        </Button>
      </div>
      <label className="flex items-center gap-2 text-sm">
        <Switch checked={settings.overwrite} onCheckedChange={(v) => save({ overwrite: v })} />
        {t("settings.convert.overwrite")}
      </label>
    </>
  );
}

/** 画像の形式変換の既定の設定(docs/52 §2 B)。変換ページで変えた値は、個別設定になる。 */
export function ConvertDefaultsSection({ scope }: { scope: SettingsScope }) {
  const t = useT();
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.convert.title")}</h2>
      <Overridable group="convert" scope={scope} label={t("settings.convert.title")}>
        <Fields scope={scope} />
      </Overridable>
    </section>
  );
}
