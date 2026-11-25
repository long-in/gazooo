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
      <div className="flex min-h-0 flex-1 flex-col gap-3 p-4 pb-2">
        <EngineBanner />
        <ProjectSwitcher />
        {lg ? (
          <div className="grid min-h-0 flex-1 grid-cols-[minmax(22rem,28rem)_1fr] gap-6">
            <div className="min-h-0 overflow-y-auto pr-2">
              <GenerationForm />
            </div>
            <div className="min-h-0 overflow-y-auto">
              <Workspace />
            </div>
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
            <GenerationForm />
            <Workspace />
          </div>
        )}
      </div>
      <LibraryStrip />
      {/* フォームの外に置く(フォームの Ctrl+Enter で生成が走らないように) */}
      <AssistantPanel open={panelOpen} onOpenChange={(o) => useAssistantStore.getState().setPanelOpen(o)} />
    </div>
  );
}
