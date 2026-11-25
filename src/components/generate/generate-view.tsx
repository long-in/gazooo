"use client";

import { useEffect } from "react";

import { EngineBanner } from "@/components/engine/engine-banner";
import { ProjectSwitcher } from "@/components/projects/project-switcher";
import { useMinWidth } from "@/lib/use-min-width";
import { useAssistantStore } from "@/stores/assistant-store";
import { useStripStore } from "@/stores/strip-store";

import { AssistantPanel } from "./assistant-panel";
import { GenerationForm } from "./generation-form";
import { LibraryStrip } from "./library-strip";
import { Workspace } from "./workspace";

/**
 * 生成ページ(docs/27・28)。メイン = 生成フォーム + 補助(2 カラム)、画面下 = キューと画像の帯。
 * lg(1024px)未満は、メインを 1 カラムにする。
 */
export function GenerateView() {
  const panelOpen = useAssistantStore((s) => s.panelOpen);
  const lg = useMinWidth(1024);

  // 他の画面へ移っても、選択は持ち越さない
  useEffect(
    () => () => {
      useStripStore.getState().select(null);
    },
    [],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* スクロールは 1 本だけ(左右で別々にスクロールさせない)。広い画面では右の補助を上に固定する */}
      <div className="min-h-0 flex-1 overflow-y-auto p-4 pb-2">
        <div className="mx-auto flex max-w-[110rem] flex-col gap-3">
          <EngineBanner />
          <ProjectSwitcher />
          {lg ? (
            // lg: フォーム | 補助。xl: フォーム(2 列)の下に補助。2xl: フォーム(2 列)| 補助(docs/50 §2.3)
            <div className="grid grid-cols-[minmax(24rem,36rem)_minmax(0,1fr)] items-start gap-6 xl:grid-cols-1 2xl:grid-cols-[minmax(44rem,60rem)_minmax(0,1fr)]">
              <GenerationForm />
              <div className="sticky top-0 max-h-[calc(100vh-8rem)] overflow-y-auto xl:static xl:max-h-none xl:overflow-visible 2xl:sticky 2xl:max-h-[calc(100vh-8rem)] 2xl:overflow-y-auto">
                <Workspace />
              </div>
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              <GenerationForm />
              <Workspace />
            </div>
          )}
        </div>
      </div>
      <LibraryStrip />
      {/* フォームの外に置く(フォームの Ctrl+Enter で生成が走らないように) */}
      <AssistantPanel open={panelOpen} onOpenChange={(o) => useAssistantStore.getState().setPanelOpen(o)} />
    </div>
  );
}
