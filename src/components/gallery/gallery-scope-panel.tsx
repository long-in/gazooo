"use client";

import { ChevronDown, ChevronRight, ChevronUp, FolderPlus, Layers, MoreHorizontal, PanelLeftClose, PanelLeftOpen, Search, Star, Tag } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";

import { ProjectDot } from "@/components/projects/project-dot";
import { useProjectDialogs } from "@/components/projects/use-project-dialogs";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useLanguage, useT } from "@/lib/i18n/use-t";
import { type ScopeEntry, sameEntry } from "@/lib/projects/scope";
import { filterProjects, formatCount, moveIndex, NARROW_WIDTH, PROJECT_FILTER_THRESHOLD } from "@/lib/projects/scope-nav";
import type { Project } from "@/lib/tauri/projects";
import { cn } from "@/lib/utils";
import { useProjectStore } from "@/stores/project-store";
import { useUiStore } from "@/stores/ui-store";

export const SCOPE_PANEL_ID = "gallery.scope";
export const SCOPE_PROJECTS_ID = "gallery.scope.projects";
const menuItem = "w-full justify-start";

/**
 * ギャラリー左の「表示範囲」パネル(docs/40 §5.1・§11)。
 * 「すべて」「お気に入り」はプロジェクトの垣根を越えたフラット表示、続いて未分類と各プロジェクト。
 * プロジェクトの管理(名前・色・並べ替え・削除)もここで行う(§5.4)。
 */
export function GalleryScopePanel({ active, onSelect }: { active: ScopeEntry; onSelect: (entry: ScopeEntry) => void }) {
  const t = useT();
  const lang = useLanguage();
  const projects = useProjectStore((s) => s.projects);
  const totals = useProjectStore((s) => s.totals);
  const saved = useUiStore((s) => s.panels[SCOPE_PANEL_ID]);
  const projectsOpen = useUiStore((s) => s.panels[SCOPE_PROJECTS_ID] !== false);
  const dialogs = useProjectDialogs();
  const [query, setQuery] = useState("");
  const [menuId, setMenuId] = useState<string | null>(null);
  // 狭いウィンドウでは既定で折りたたむ(保存値があればそちらが優先。§11.4)
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    const check = () => setNarrow(window.innerWidth < NARROW_WIDTH);
    check();
    window.addEventListener("resize", check);
    return () => window.removeEventListener("resize", check);
  }, []);
  const open = saved === undefined ? !narrow : saved !== false;

  const activeProjectId = typeof active === "object" ? active.projectId : null;
  const showFilter = projects.length > PROJECT_FILTER_THRESHOLD;
  const visible = filterProjects(projects, showFilter ? query : "", activeProjectId);
  const fmt = (n: number) => formatCount(n, lang);
  const setOpen = (v: boolean) => useUiStore.getState().setPanelOpen(SCOPE_PANEL_ID, v);

  const fixed: { key: string; entry: ScopeEntry; icon: ReactNode; label: string; count: number }[] = [
    { key: "all", entry: "all", icon: <Layers aria-hidden className="size-4" />, label: t("scope.all"), count: totals.total },
    { key: "favorites", entry: "favorites", icon: <Star aria-hidden className="size-4 text-amber-400" />, label: t("scope.favorites"), count: totals.favorites },
    { key: "unassigned", entry: "unassigned", icon: <Tag aria-hidden className="size-4 text-muted-foreground" />, label: t("scope.unassigned"), count: totals.unassigned },
  ];

  if (!open) {
    // 折りたたみ時のミニレール(色ドット。件数はツールチップ。§11.3)
    const railBtn = (key: string, entry: ScopeEntry, icon: ReactNode, label: string, count: number) => {
      const selected = sameEntry(active, entry);
      return (
        <Button
          key={key}
          type="button"
          variant={selected ? "secondary" : "ghost"}
          size="icon-sm"
          aria-current={selected ? "true" : undefined}
          aria-label={`${label} (${count})`}
          title={`${label} (${count.toLocaleString("en-US")})`}
          onClick={() => onSelect(entry)}
        >
          {icon}
        </Button>
      );
    };
    return (
      <nav aria-label={t("scope.panel")} className="flex w-12 shrink-0 flex-col items-center gap-1 overflow-y-auto border-r p-2">
        <Button type="button" variant="ghost" size="icon-sm" aria-label={t("scope.expand")} title={t("scope.expand")} onClick={() => setOpen(true)}>
          <PanelLeftOpen />
        </Button>
        {fixed.map((f) => railBtn(f.key, f.entry, f.icon, f.label, f.count))}
        <div className="my-1 h-px w-5 bg-border" />
        {projects.map((p) => railBtn(p.id, { projectId: p.id }, <ProjectDot color={p.color} />, p.name, p.imageCount))}
      </nav>
    );
  }

  const row = (entry: ScopeEntry, icon: ReactNode, label: string, count: number) => {
    const selected = sameEntry(active, entry);
    return (
      <Button
        type="button"
        size="sm"
        variant={selected ? "secondary" : "ghost"}
        aria-current={selected ? "true" : undefined}
        data-scope-row=""
        tabIndex={selected ? 0 : -1}
        title={`${label} (${count.toLocaleString("en-US")})`}
        className={cn(
          "relative h-8 min-w-0 flex-1 justify-start gap-2",
          selected && "before:absolute before:inset-y-1 before:left-0 before:w-0.5 before:rounded-full before:bg-primary",
        )}
        onClick={() => onSelect(entry)}
      >
        {icon}
        <span className={cn("min-w-0 flex-1 truncate text-left", count === 0 && "text-muted-foreground")}>{label}</span>
        <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{fmt(count)}</span>
      </Button>
    );
  };

  // ↑↓ / Home / End で項目間を移動(roving tabindex。§11.2)
  function onNavKeyDown(e: React.KeyboardEvent<HTMLElement>) {
    const rows = Array.from(e.currentTarget.querySelectorAll<HTMLElement>("[data-scope-row]"));
    const idx = rows.indexOf(document.activeElement as HTMLElement);
    if (idx < 0) return;
    const next = moveIndex(idx, rows.length, e.key);
    if (next === null) return;
    e.preventDefault();
    rows[next]?.focus();
  }

  return (
    <nav aria-label={t("scope.panel")} className="flex w-56 shrink-0 flex-col border-r" onKeyDown={onNavKeyDown}>
      <div className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto p-3 pb-1">
        <div className="mb-1 flex items-center justify-between">
          <h2 className="px-2 text-xs font-medium text-muted-foreground">{t("scope.title")}</h2>
          <Button type="button" variant="ghost" size="icon-xs" aria-label={t("scope.collapse")} title={t("scope.collapse")} onClick={() => setOpen(false)}>
            <PanelLeftClose />
          </Button>
        </div>
        {fixed.map((f) => (
          <div key={f.key} className="flex">
            {row(f.entry, f.icon, f.label, f.count)}
          </div>
        ))}

        <button
          type="button"
          className="mt-2 flex items-center gap-1 rounded px-2 py-1 text-left text-xs font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
          aria-expanded={projectsOpen}
          onClick={() => useUiStore.getState().setPanelOpen(SCOPE_PROJECTS_ID, !projectsOpen)}
        >
          {projectsOpen ? <ChevronDown aria-hidden className="size-3.5" /> : <ChevronRight aria-hidden className="size-3.5" />}
          {t("scope.projects")}
        </button>
        {projectsOpen && (
          <>
            {showFilter && (
              <div className="relative mb-1">
                <Search aria-hidden className="pointer-events-none absolute left-2 top-2 size-3.5 text-muted-foreground" />
                <Input
                  type="search"
                  className="h-7 pl-7 text-xs"
                  value={query}
                  placeholder={t("scope.filter")}
                  aria-label={t("scope.filter")}
                  onChange={(e) => setQuery(e.target.value)}
                />
              </div>
            )}
            <ul className="flex flex-col gap-0.5">
              {visible.map((p) => {
                const i = projects.findIndex((x) => x.id === p.id);
                return (
                  <li
                    key={p.id}
                    className="flex items-center gap-0.5"
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setMenuId(p.id);
                    }}
                    onKeyDown={(e) => {
                      if (e.target !== e.currentTarget.querySelector("[data-scope-row]")) return;
                      if (e.key === "F2") {
                        e.preventDefault();
                        dialogs.open({ kind: "edit", project: p });
                      } else if (e.key === "Delete") {
                        e.preventDefault();
                        dialogs.open({ kind: "delete", project: p });
                      }
                    }}
                  >
                    {row({ projectId: p.id }, <ProjectDot color={p.color} />, p.name, p.imageCount)}
                    <ProjectMenu
                      project={p}
                      first={i === 0}
                      last={i === projects.length - 1}
                      open={menuId === p.id}
                      onOpenChange={(v) => setMenuId(v ? p.id : null)}
                      onEdit={() => dialogs.open({ kind: "edit", project: p })}
                      onDelete={() => dialogs.open({ kind: "delete", project: p })}
                      onMove={(delta) => {
                        const ids = projects.map((x) => x.id);
                        ids.splice(i + delta, 0, ids.splice(i, 1)[0]);
                        void useProjectStore.getState().reorder(ids).catch(() => {});
                      }}
                    />
                  </li>
                );
              })}
            </ul>
            {showFilter && query.trim() !== "" && visible.length === 0 && <p className="px-2 text-xs text-muted-foreground">{t("scope.filterEmpty")}</p>}
          </>
        )}
      </div>
      <div className="border-t p-3">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="w-full justify-start"
          onClick={() => dialogs.open({ kind: "create", onCreated: (p) => onSelect({ projectId: p.id }) })}
        >
          <FolderPlus />
          {t("project.new")}
        </Button>
      </div>
      {dialogs.element}
    </nav>
  );
}

function ProjectMenu({
  project,
  first,
  last,
  open,
  onOpenChange,
  onEdit,
  onDelete,
  onMove,
}: {
  project: Project;
  first: boolean;
  last: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onEdit: () => void;
  onDelete: () => void;
  onMove: (delta: -1 | 1) => void;
}) {
  const t = useT();
  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            className={cn("shrink-0 text-muted-foreground")}
            aria-label={`${t("project.menu")}: ${project.name}`}
            title={t("project.menu")}
          />
        }
      >
        <MoreHorizontal />
      </PopoverTrigger>
      <PopoverContent align="end" className="flex w-44 flex-col gap-1 p-2">
        <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={onEdit}>
          {t("project.rename")}
        </Button>
        <Button type="button" size="sm" variant="ghost" className={menuItem} disabled={first} onClick={() => onMove(-1)}>
          <ChevronUp />
          {t("project.moveUp")}
        </Button>
        <Button type="button" size="sm" variant="ghost" className={menuItem} disabled={last} onClick={() => onMove(1)}>
          <ChevronDown />
          {t("project.moveDown")}
        </Button>
        <Button type="button" size="sm" variant="ghost" className={cn(menuItem, "text-destructive")} onClick={onDelete}>
          {t("project.delete")}
        </Button>
      </PopoverContent>
    </Popover>
  );
}
