"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { type MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { parseSettingsLocation, settingsHref, type TabId, tabsFor } from "@/lib/settings/scope-nav";
import { toAppError } from "@/lib/tauri/errors";
import type { Settings, SettingsScope } from "@/lib/tauri/settings";
import { cn } from "@/lib/utils";
import { useSettingsScopeStore } from "@/stores/settings-scope-store";
import { useSettingsStore } from "@/stores/settings-store";

import { AssistantSection } from "./assistant-section";
import { ConvertDefaultsSection } from "./convert-defaults-section";
import { DisplaySection } from "./display-section";
import { EngineInstallSection } from "./engine-install-section";
import { EngineProcessSection } from "./engine-process-section";
import { OtherSection } from "./other-section";
import { StorageSection } from "./storage-section";
import { TranslateSection } from "./translate-section";

const TAB_LABELS: Record<TabId, MessageKey> = {
  engine: "settings.tabs.engine",
  storage: "settings.tabs.storage",
  assistant: "settings.tabs.assistant",
  display: "settings.tabs.display",
  other: "settings.tabs.other",
};

const SCOPES: SettingsScope[] = ["user", "global"];

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

/** ComfyUI の接続先(全体設定)。入力して [保存] で反映する。 */
function EngineUrlSection({ initial }: { initial: Settings }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [url, setUrl] = useState(initial.comfyuiUrl);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await useSettingsStore.getState().update({ comfyuiUrl: url });
      toast.success(t("settings.saved"));
    } catch (e) {
      const message = errorMessage(toAppError(e));
      setError(message);
      toast.error(message);
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.engine.title")}</h2>
      <Label htmlFor="comfyui-url">{t("settings.engine.url")}</Label>
      <div className="flex gap-2">
        <Input id="comfyui-url" value={url} aria-invalid={error !== null} onChange={(e) => setUrl(e.target.value)} />
        <Button onClick={() => void save()} disabled={saving}>
          {t("settings.save")}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{t("settings.engine.urlHint")}</p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}

/**
 * 設定ページ(docs/25・52)。上で [個別設定 | 全体設定] を切り替え、その下をカテゴリのタブにする。
 */
function SettingsFormInner({ initial }: { initial: Settings }) {
  const t = useT();
  const router = useRouter();
  const params = useSearchParams();
  const { scope, tab } = parseSettingsLocation(params.get("scope"), params.get("tab"));

  useEffect(() => {
    void useSettingsScopeStore.getState().load().catch(() => {});
  }, []);

  const go = (nextScope: SettingsScope, nextTab: TabId) => router.replace(settingsHref(nextScope, nextTab));

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-6 p-8">
      <h1 className="sr-only">{t("settings.title")}</h1>

      <div className="flex flex-col gap-2">
        <div role="radiogroup" aria-label={t("settings.title")} className="flex w-fit gap-1 rounded-lg bg-muted p-1">
          {SCOPES.map((s) => (
            <button
              key={s}
              type="button"
              role="radio"
              aria-checked={scope === s}
              onClick={() => go(s, parseSettingsLocation(s, tab).tab)}
              className={cn(
                "rounded-md px-4 py-1.5 text-sm transition-colors",
                scope === s ? "bg-background font-medium shadow-sm" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {t(`settings.scope.${s}`)}
            </button>
          ))}
        </div>
        <p
          className={cn(
            "text-xs",
            scope === "global"
              ? "rounded-md border border-amber-500/40 bg-amber-500/10 p-2"
              : "text-muted-foreground",
          )}
        >
          {t(`settings.scope.${scope}.desc`)}
        </p>
      </div>

      {/* スコープを切り替えたら中身を作り直し、そのスコープの値を読み直す */}
      <Tabs key={scope} value={tab} onValueChange={(v) => go(scope, parseSettingsLocation(scope, v as string).tab)}>
        <TabsList>
          {tabsFor(scope).map((id) => (
            <TabsTrigger key={id} value={id}>
              {t(TAB_LABELS[id])}
            </TabsTrigger>
          ))}
        </TabsList>

        {scope === "global" && (
          <TabsContent value="engine" keepMounted>
            <EngineUrlSection initial={initial} />
            <EngineInstallSection />
            <EngineProcessSection />
          </TabsContent>
        )}

        <TabsContent value="storage" keepMounted>
          <StorageSection scope={scope} />
        </TabsContent>

        <TabsContent value="assistant" keepMounted>
          <div className="flex flex-col gap-8">
            <AssistantSection scope={scope} />
            <TranslateSection scope={scope} />
            {scope === "user" && (
              <Link href={settingsHref("global", "assistant")} className="text-xs underline underline-offset-2">
                {t("settings.scope.installInGlobal")}
              </Link>
            )}
          </div>
        </TabsContent>

        <TabsContent value="display" keepMounted>
          <DisplaySection scope={scope} />
        </TabsContent>

        <TabsContent value="other" keepMounted>
          <div className="flex flex-col gap-8">
            <ConvertDefaultsSection scope={scope} />
            <OtherSection scope={scope} />
          </div>
        </TabsContent>
      </Tabs>
    </div>
  );
}
