"use client";

import { ChevronDown, ChevronRight } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { ModelProfileView } from "@/components/models/model-profile-view";
import { Button } from "@/components/ui/button";
import { useLanguage, useT } from "@/lib/i18n/use-t";
import { featuresFor, profileFor, recommendedPatch } from "@/lib/models/profile";
import { useAssistantStore } from "@/stores/assistant-store";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";
import { useModelsStore } from "@/stores/models-store";

const OPEN_KEY = "gazooo.modelProfile.open";

// 開閉は端末に記憶する(読めない環境では、展開したまま)
function readOpen(): boolean {
  try {
    return window.localStorage.getItem(OPEN_KEY) !== "0";
  } catch {
    return true;
  }
}

function writeOpen(open: boolean) {
  try {
    window.localStorage.setItem(OPEN_KEY, open ? "1" : "0");
  } catch {
    /* 記憶できなくても、表示には影響しない */
  }
}

/**
 * 選択中のモデルの説明パネル(docs/18 §5)。用途・得意/不得意・プロンプトの書き方・必要メモリ・対応機能と、
 * [推奨設定を適用] / [AI に相談] を出す。カタログ外のモデルには、種別の一般説明と、その旨を出す。
 */
export function ModelProfilePanel() {
  const t = useT();
  const lang = useLanguage();
  const catalog = useModelsStore((s) => s.view?.catalog);
  const kind = useFormStore((s) => s.params.model.kind);
  const name = useFormStore((s) => s.params.model.name);
  const [open, setOpen] = useState(readOpen);

  const match = profileFor(catalog, kind, name);
  if (!match) return null;
  const { family, profile, exact } = match;

  function toggle() {
    setOpen((o) => {
      writeOpen(!o);
      return !o;
    });
  }

  function applyRecommended() {
    useFormStore.getState().applyRecommended(recommendedPatch(profile), useEngineStore.getState().models);
    toast.success(t("modelProfile.applied"));
  }

  function ask() {
    const subject = exact ? family.name[lang] : t("modelProfile.thisModel");
    useAssistantStore.getState().openWithDraft(`${subject}: ${t("modelProfile.askDraft")}`);
  }

  return (
    <section className="rounded-lg border p-3" aria-labelledby="model-profile-title">
      <button
        type="button"
        onClick={toggle}
        aria-expanded={open}
        className="flex w-full items-center gap-1 text-left text-sm font-medium"
      >
        {open ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
        <span id="model-profile-title">
          {t("modelProfile.title")}
          {exact ? `: ${family.name[lang]}` : ""}
        </span>
      </button>

      {open && (
        <div className="mt-2 flex flex-col gap-3">
          {!exact && (
            <p role="note" className="rounded-md bg-muted px-2 py-1 text-xs text-muted-foreground">
              {t("modelProfile.uncatalogued")}
            </p>
          )}
          <ModelProfileView profile={profile} lang={lang} features={featuresFor(kind)} />
          <div className="flex flex-wrap gap-2">
            <Button type="button" size="sm" variant="outline" onClick={applyRecommended}>
              {t("modelProfile.apply")}
            </Button>
            <Button type="button" size="sm" variant="ghost" onClick={ask}>
              {t("modelProfile.ask")}
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}
