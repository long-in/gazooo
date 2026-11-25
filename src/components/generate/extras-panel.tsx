"use client";

import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { supportsFaceDetailer } from "@/lib/generation/defaults";
import { availableTabs, canEnableTab, currentTab, type WorkTab } from "@/lib/generation/work-tab";
import type { MessageKey } from "@/lib/i18n";
import { useT } from "@/lib/i18n/use-t";
import { useEngineStore } from "@/stores/engine-store";
import { fieldErrorOf, useFormErrorStore } from "@/stores/form-error-store";
import { useFormStore } from "@/stores/form-store";
import { useUiStore } from "@/stores/ui-store";

import { FaceSwapSection } from "./face-swap-section";
import { ControlnetSection } from "./controlnet-section";
import { FaceDetailerSection } from "./face-detailer-section";
import { Img2imgSection } from "./img2img-section";
import { InpaintSection } from "./inpaint-section";
import { ReferenceSection } from "./reference-section";

/**
 * 拡張機能。排他の機能(画像から生成・Inpaint・参照画像・顔の入れ替え)は、モデルで使えるものだけを
 * 「作業タブ」で選ぶ(docs/45 段階 1)。ControlNet・顔補正は使えるモデルのときだけ下に並べる。
 * 生成フォームの縦を短くするため、右側のワークスペースに置く(畳める。docs/20 §4.2)。
 * かんたんモードでは既定で閉じ、オンの機能の件数をバッジで示す(docs/38 §2.3)。
 */
export function ExtrasPanel() {
  const t = useT();
  const err = fieldErrorOf(useFormErrorStore((s) => s.error));
  const simple = useUiStore((s) => s.generateMode === "simple");
  const id = simple ? "extras.simple" : "extras.advanced";
  const open = useUiStore((s) => s.panels[id] ?? !simple);
  const params = useFormStore((s) => s.params);
  const status = useEngineStore((s) => s.status);
  const controlnetModels = useEngineStore((s) => s.models?.controlnets.length ?? 0);
  const picked = useFormStore((s) => s.workTab);
  const kind = params.model.kind;

  const tabs = availableTabs(kind);
  // 機能がオンならそのタブを示す(設定の復元・プリセット・ジョブの読み込みに追従)。すべてオフなら選んだタブ
  const tab = currentTab(params, picked);
  const cnOn = params.controlnet?.enabled === true;
  const cnOpen = useUiStore((s) => s.panels["extras.cn"] ?? cnOn);
  const postOn = params.faceDetailer?.enabled === true;
  const postOpen = useUiStore((s) => s.panels["extras.post"] ?? postOn);
  const showFace = supportsFaceDetailer(kind);
  if (tabs.length === 0 && !showFace) return null;

  const count = [
    params.faceSwap?.enabled,
    params.reference?.enabled,
    params.img2img?.enabled,
    params.inpaint?.enabled,
    params.controlnet?.enabled,
    params.faceDetailer?.enabled,
  ].filter(Boolean).length;

  function select(next: WorkTab) {
    useFormStore.getState().setWorkTab(next, canEnableTab(next, kind, status, controlnetModels > 0));
  }

  return (
    <details
      open={open}
      className="flex flex-col gap-4 rounded-lg border p-3 [&[open]>summary]:mb-1"
      onToggle={(e) => {
        const next = e.currentTarget.open;
        if (next !== open) useUiStore.getState().setPanelOpen(id, next);
      }}
    >
      <summary className="cursor-pointer text-sm font-medium">
        {t("generate.extras")}
        {count > 0 && (
          <Badge variant="secondary" className="ml-2">
            {t("generate.extras.badge").replace("{n}", String(count))}
          </Badge>
        )}
      </summary>
      {tabs.length > 0 && (
        <div role="tablist" aria-label={t("generate.tabs")} className="flex flex-wrap gap-1">
          {tabs.map((x) => (
            <Button
              key={x}
              type="button"
              role="tab"
              aria-selected={tab === x}
              size="xs"
              variant={tab === x ? "default" : "outline"}
              onClick={() => select(x)}
            >
              {t(`generate.tab.${x}` as MessageKey)}
            </Button>
          ))}
        </div>
      )}
      {tab === "faceSwap" && <FaceSwapSection error={err} />}
      {tab === "reference" && <ReferenceSection error={err} />}
      {tab === "img2img" && <Img2imgSection error={err} />}
      {tab === "inpaint" && <InpaintSection error={err} />}
      {tab === "controlnet" && <ControlnetSection error={err} />}
      {(tab === "img2img" || tab === "inpaint") && (
        <Fold id="extras.cn" open={cnOpen} title={t("generate.cn.combine")} on={cnOn}>
          <ControlnetSection error={err} />
        </Fold>
      )}
      {showFace && (
        <Fold id="extras.post" open={postOpen} title={t("generate.post")} on={postOn}>
          <FaceDetailerSection error={err} />
        </Fold>
      )}
    </details>
  );
}

/** 折りたたみ(開閉は端末の画面状態に保存)。中の機能がオンなら「オン」のバッジを出す。 */
function Fold({
  id,
  open,
  title,
  on,
  children,
}: {
  id: string;
  open: boolean;
  title: string;
  on: boolean;
  children: ReactNode;
}) {
  const t = useT();
  return (
    <details
      open={open}
      className="flex flex-col gap-3 rounded-lg border border-dashed p-3 [&[open]>summary]:mb-1"
      onToggle={(e) => {
        const next = e.currentTarget.open;
        if (next !== open) useUiStore.getState().setPanelOpen(id, next);
      }}
    >
      <summary className="cursor-pointer text-sm font-medium">
        {title}
        {on && (
          <Badge variant="secondary" className="ml-2">
            {t("generate.fold.on")}
          </Badge>
        )}
      </summary>
      {children}
    </details>
  );
}
