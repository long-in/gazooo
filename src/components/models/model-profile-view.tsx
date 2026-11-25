"use client";

import { Check, X } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import type { MessageKey } from "@/lib/i18n";
import { useT } from "@/lib/i18n/use-t";
import type { featuresFor } from "@/lib/models/profile";
import type { ModelProfile } from "@/lib/tauri/models";
import type { Language } from "@/lib/tauri/settings";

const FEATURES = ["negative", "faceDetailer", "controlnet", "inpaint"] as const;

/**
 * モデルの説明の本体(docs/18 §5)。生成画面の説明パネルと、取得ページのカードで共通に使う。
 * 内容はカタログの `profile` から作る(単一の情報源)。
 */
export function ModelProfileView({
  profile,
  lang,
  features,
}: {
  profile: ModelProfile;
  lang: Language;
  /** 対応機能(○ / ×)。種別が決まっているときだけ渡す(種別から計算する値)。 */
  features?: ReturnType<typeof featuresFor>;
}) {
  const t = useT();
  return (
    <div className="flex flex-col gap-2 text-sm">
      <p className="font-medium">{profile.summary[lang]}</p>

      <div className="grid gap-2 sm:grid-cols-2">
        <section aria-label={t("modelProfile.bestFor")}>
          <h4 className="text-xs font-medium text-muted-foreground">{t("modelProfile.bestFor")}</h4>
          <ul className="mt-0.5 list-disc pl-4 text-xs">
            {profile.bestFor[lang].map((s) => (
              <li key={s}>{s}</li>
            ))}
          </ul>
        </section>
        <section aria-label={t("modelProfile.weakAt")}>
          <h4 className="text-xs font-medium text-muted-foreground">{t("modelProfile.weakAt")}</h4>
          <ul className="mt-0.5 list-disc pl-4 text-xs">
            {profile.weakAt[lang].map((s) => (
              <li key={s}>{s}</li>
            ))}
          </ul>
        </section>
      </div>

      <p className="text-xs">
        <span className="font-medium text-muted-foreground">{t("modelProfile.prompt")}: </span>
        {t(`modelProfile.prompt.${profile.promptStyle.kind}` as MessageKey)}。{profile.promptStyle.hint[lang]}
      </p>

      <p className="text-xs">
        <span className="font-medium text-muted-foreground">{t("modelProfile.resources")}: </span>
        VRAM {profile.resources.vramGb}GB {t("modelProfile.vramOrMore")} ·{" "}
        {t(`modelProfile.speed.${profile.resources.speed}` as MessageKey)}
      </p>

      {features && (
        <ul className="flex flex-wrap gap-x-3 gap-y-0.5 text-xs" aria-label={t("modelProfile.features")}>
          {FEATURES.map((f) => (
            <li key={f} className="flex items-center gap-1">
              {features[f] ? (
                <Check className="size-3.5 text-emerald-600" aria-label={t("modelProfile.supported")} />
              ) : (
                <X className="size-3.5 text-muted-foreground" aria-label={t("modelProfile.unsupported")} />
              )}
              <span className={features[f] ? "" : "text-muted-foreground"}>
                {t(`modelProfile.feature.${f}` as MessageKey)}
              </span>
            </li>
          ))}
        </ul>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">
          {t("modelProfile.moderation")}: {t(`modelProfile.moderation.${profile.moderation.level}` as MessageKey)}
        </Badge>
        {profile.moderation.level !== "unknown" && (
          <span className="text-xs text-muted-foreground">{profile.moderation.note[lang]}</span>
        )}
        <span className="text-xs text-muted-foreground">{profile.sourceNote[lang]}</span>
      </div>
    </div>
  );
}
