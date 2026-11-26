"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { type Language, type Settings, type SettingsPatch, type SettingsScope, settingsGet, type Theme } from "@/lib/tauri/settings";
import { useSettingsStore } from "@/stores/settings-store";
import { useUiStore } from "@/stores/ui-store";

import { Overridable } from "./overridable";

const THEMES: { value: Theme; labelKey: MessageKey }[] = [
  { value: "system", labelKey: "settings.ui.theme.system" },
  { value: "light", labelKey: "settings.ui.theme.light" },
  { value: "dark", labelKey: "settings.ui.theme.dark" },
];

function Choice<T extends string>({
  label,
  value,
  items,
  onChange,
}: {
  label: string;
  value: T;
  items: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <Select value={value} items={items} onValueChange={(v) => v && onChange(v as T)}>
      <SelectTrigger className="w-full" aria-label={label}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {items.map((i) => (
          <SelectItem key={i.value} value={i.value}>
            {i.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/** テーマか言語の 1 項目。そのスコープの値を読み、選んだらすぐ保存する。 */
function DisplayValue({ scope, kind }: { scope: SettingsScope; kind: "theme" | "language" }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const effective = useSettingsStore((s) => s.settings);
  const [global, setGlobal] = useState<Settings | null>(null);

  useEffect(() => {
    if (scope === "global") void settingsGet("global").then(setGlobal).catch(() => {});
  }, [scope]);

  const settings = scope === "global" ? global : effective;
  if (!settings) return null;

  async function save(patch: SettingsPatch) {
    try {
      await useSettingsStore.getState().update({ ...patch, scope });
      if (scope === "global") setGlobal(await settingsGet("global"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  if (kind === "theme") {
    return (
      <Choice
        label={t("settings.ui.theme")}
        value={settings.theme}
        items={THEMES.map((x) => ({ value: x.value, label: t(x.labelKey) }))}
        onChange={(theme) => void save({ theme })}
      />
    );
  }
  return (
    <Choice<Language | "auto">
      label={t("settings.ui.language")}
      value={settings.language ?? "auto"}
      items={[
        { value: "auto", label: t("settings.ui.language.auto") },
        { value: "ja", label: "日本語" },
        { value: "en", label: "English" },
      ]}
      onChange={(v) => void save({ language: v === "auto" ? null : v })}
    />
  );
}

export function DisplaySection({ scope }: { scope: SettingsScope }) {
  const t = useT();
  const density = useUiStore((s) => s.density);

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.ui.title")}</h2>
      <Overridable group="theme" scope={scope} label={t("settings.ui.theme")}>
        <DisplayValue scope={scope} kind="theme" />
      </Overridable>
      <Overridable group="language" scope={scope} label={t("settings.ui.language")}>
        <DisplayValue scope={scope} kind="language" />
      </Overridable>
      {scope === "user" && (
        <div className="flex flex-col gap-2 rounded-lg border p-3">
          <Label>{t("settings.ui.density")}</Label>
          {/* 画面の状態なので、すぐ反映・自動保存する */}
          <Choice<"compact" | "comfortable">
            label={t("settings.ui.density")}
            value={density}
            items={[
              { value: "compact", label: t("settings.ui.density.compact") },
              { value: "comfortable", label: t("settings.ui.density.comfortable") },
            ]}
            onChange={(v) => useUiStore.getState().setDensity(v)}
          />
        </div>
      )}
    </section>
  );
}
