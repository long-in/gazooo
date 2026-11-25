"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { type MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import type { Language, Settings, Theme } from "@/lib/tauri/settings";
import { useSettingsStore } from "@/stores/settings-store";
import { useUiStore } from "@/stores/ui-store";

import { AssistantSection } from "./assistant-section";
import { TranslateSection } from "./translate-section";
import { EngineInstallSection } from "./engine-install-section";
import { EngineProcessSection } from "./engine-process-section";
import { OtherSection } from "./other-section";
import { StorageSection } from "./storage-section";

type LanguageChoice = Language | "auto";

const TAB_IDS = ["engine", "storage", "assistant", "display", "other"] as const;
type TabId = (typeof TAB_IDS)[number];

const TAB_LABELS: Record<TabId, MessageKey> = {
  engine: "settings.tabs.engine",
  storage: "settings.tabs.storage",
  assistant: "settings.tabs.assistant",
  display: "settings.tabs.display",
  other: "settings.tabs.other",
};

/** 検証エラーの field がどのタブの項目かを返す。 */
const FIELD_TAB: Record<string, TabId> = {
  comfyuiUrl: "engine",
  theme: "display",
  language: "display",
};

function parseTab(value: string | null): TabId {
  return TAB_IDS.find((id) => id === value) ?? "engine";
}

const THEMES: { value: Theme; labelKey: MessageKey }[] = [
  { value: "system", labelKey: "settings.ui.theme.system" },
  { value: "light", labelKey: "settings.ui.theme.light" },
  { value: "dark", labelKey: "settings.ui.theme.dark" },
];

export function SettingsForm() {
  const loaded = useSettingsStore((s) => s.loaded);
  const settings = useSettingsStore((s) => s.settings);
  const loadError = useSettingsStore((s) => s.loadError);
  const errorMessage = useErrorMessage();
  const t = useT();

  if (!loaded) return null;
  if (loadError) {
    return (
      <div className="p-8 text-sm text-destructive">
        {t("settings.loadFailed")}: {errorMessage(loadError)}
      </div>
    );
  }
  return <SettingsFormInner initial={settings} />;
}

function SettingsFormInner({ initial }: { initial: Settings }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const update = useSettingsStore((s) => s.update);
  const density = useUiStore((s) => s.density);

  const [draft, setDraft] = useState<Settings>(initial);
  const [fieldError, setFieldError] = useState<{
    field?: string;
    message: string;
  } | null>(null);
  const [saving, setSaving] = useState(false);

  const router = useRouter();
  const params = useSearchParams();
  const tab = parseTab(params.get("tab"));
  function selectTab(next: TabId) {
    router.replace(`/settings?tab=${next}`);
  }

  const languageChoice: LanguageChoice = draft.language ?? "auto";

  async function save() {
    setSaving(true);
    setFieldError(null);
    try {
      await update({
        comfyuiUrl: draft.comfyuiUrl,
        theme: draft.theme,
        language: draft.language,
      });
      toast.success(t("settings.saved"));
    } catch (e) {
      const err = toAppError(e);
      const field = err.details?.field;
      setFieldError({ field, message: errorMessage(err) });
      if (field && FIELD_TAB[field]) selectTab(FIELD_TAB[field]);
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-8 p-8">
      <h1 className="sr-only">{t("settings.title")}</h1>

      <Tabs value={tab} onValueChange={(v) => selectTab(parseTab(v as string))}>
        <TabsList>
          {TAB_IDS.map((id) => (
            <TabsTrigger key={id} value={id}>
              {t(TAB_LABELS[id])}
            </TabsTrigger>
          ))}
        </TabsList>

        <TabsContent value="engine" keepMounted>
          <section className="flex flex-col gap-3">
            <h2 className="text-lg font-medium">
              {t("settings.engine.title")}
            </h2>
            <Label htmlFor="comfyui-url">{t("settings.engine.url")}</Label>
            <Input
              id="comfyui-url"
              value={draft.comfyuiUrl}
              aria-invalid={fieldError?.field === "comfyuiUrl"}
              onChange={(e) =>
                setDraft((d) => ({ ...d, comfyuiUrl: e.target.value }))
              }
            />
            <p className="text-xs text-muted-foreground">
              {t("settings.engine.urlHint")}
            </p>
            {fieldError?.field === "comfyuiUrl" && (
              <p role="alert" className="text-sm text-destructive">
                {fieldError.message}
              </p>
            )}
          </section>

          <EngineInstallSection />

          <EngineProcessSection />
        </TabsContent>

        <TabsContent value="storage" keepMounted>
          <StorageSection />
        </TabsContent>

        <TabsContent value="assistant" keepMounted>
          <div className="flex flex-col gap-8">
            <AssistantSection />
            <TranslateSection />
          </div>
        </TabsContent>

        <TabsContent value="display" keepMounted>
          <section className="flex flex-col gap-3">
            <h2 className="text-lg font-medium">{t("settings.ui.title")}</h2>
            <div className="grid grid-cols-2 gap-4">
              <div className="flex flex-col gap-2">
                <Label>{t("settings.ui.theme")}</Label>
                <Select
                  value={draft.theme}
                  items={THEMES.map((x) => ({
                    value: x.value,
                    label: t(x.labelKey),
                  }))}
                  onValueChange={(v) =>
                    v && setDraft((d) => ({ ...d, theme: v as Theme }))
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {THEMES.map((x) => (
                      <SelectItem key={x.value} value={x.value}>
                        {t(x.labelKey)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-2">
                <Label>{t("settings.ui.language")}</Label>
                <Select
                  value={languageChoice}
                  items={[
                    { value: "auto", label: t("settings.ui.language.auto") },
                    { value: "ja", label: "日本語" },
                    { value: "en", label: "English" },
                  ]}
                  onValueChange={(v) =>
                    v &&
                    setDraft((d) => ({
                      ...d,
                      language: v === "auto" ? null : (v as Language),
                    }))
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">
                      {t("settings.ui.language.auto")}
                    </SelectItem>
                    <SelectItem value="ja">日本語</SelectItem>
                    <SelectItem value="en">English</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-2">
                <Label>{t("settings.ui.density")}</Label>
                {/* 画面の状態なので、[保存] を待たずにすぐ反映・自動保存する */}
                <Select
                  value={density}
                  items={[
                    { value: "compact", label: t("settings.ui.density.compact") },
                    { value: "comfortable", label: t("settings.ui.density.comfortable") },
                  ]}
                  onValueChange={(v) => v && useUiStore.getState().setDensity(v as "compact" | "comfortable")}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="compact">{t("settings.ui.density.compact")}</SelectItem>
                    <SelectItem value="comfortable">{t("settings.ui.density.comfortable")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          </section>
        </TabsContent>

        {/* デバッグ・ログの操作は即時反映(下の [保存] の対象外) */}
        <TabsContent value="other" keepMounted>
          <OtherSection />
        </TabsContent>
      </Tabs>

      {tab !== "other" && (
        <div>
          <Button onClick={save} disabled={saving}>
            {t("settings.save")}
          </Button>
        </div>
      )}
    </div>
  );
}
