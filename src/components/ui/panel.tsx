"use client";

import { ChevronDown, ChevronRight } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/ui-store";

/**
 * 見出しバーつきのパネル(docs/29 §3.1)。見出しのクリックで折りたたみ・展開し、状態は `id` ごとに保存する。
 */
export function Panel({
  id,
  title,
  defaultOpen = true,
  actions,
  children,
  className,
  bodyClassName,
}: {
  id: string;
  title: ReactNode;
  defaultOpen?: boolean;
  /** 見出しバーの右側に置く操作(クリックしても開閉しない)。 */
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  const open = useUiStore((s) => s.panels[id] ?? defaultOpen);
  const bodyId = `panel-${id}`;
  return (
    <section className={cn("overflow-hidden rounded-md border bg-card", className)}>
      <div className="flex items-center bg-panel-header">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={bodyId}
          onClick={() => useUiStore.getState().setPanelOpen(id, !open)}
          className="flex min-w-0 flex-1 items-center gap-1 px-2 py-1 text-left text-[11px] font-semibold uppercase tracking-wide text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring"
        >
          {open ? <ChevronDown className="size-3.5 shrink-0" /> : <ChevronRight className="size-3.5 shrink-0" />}
          <span className="truncate">{title}</span>
        </button>
        {actions && <div className="flex shrink-0 items-center gap-1 pr-1">{actions}</div>}
      </div>
      {open && (
        <div id={bodyId} className={cn("border-t p-2.5", bodyClassName)}>
          {children}
        </div>
      )}
    </section>
  );
}
