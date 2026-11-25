"use client";

import { ACCENTS, type UserColor } from "@/lib/users/logic";
import { useT } from "@/lib/i18n/use-t";
import type { Project, ProjectColor } from "@/lib/tauri/projects";
import { useProjectStore } from "@/stores/project-store";
import { cn } from "@/lib/utils";

/** プロジェクトの色の丸(色なしは枠のみ)。 */
export function ProjectDot({ color, className }: { color: ProjectColor | null; className?: string }) {
  const bg = color ? ACCENTS[color as UserColor]?.light.bg : undefined;
  return (
    <span
      aria-hidden
      className={cn("inline-block size-2.5 shrink-0 rounded-full border", !bg && "border-muted-foreground/60", className)}
      style={bg ? { backgroundColor: bg, borderColor: bg } : undefined}
    />
  );
}

/**
 * 画像の所属プロジェクトを示す小さなチップ(docs/40 §5.2)。未分類・不明なプロジェクトは何も出さない。
 * `onOpen` を渡すと、クリックでそのプロジェクトの表示へ移る。
 */
export function ProjectChip({
  projectId,
  onOpen,
  className,
}: {
  projectId: string | null;
  onOpen?: (project: Project) => void;
  className?: string;
}) {
  const t = useT();
  const project = useProjectStore((s) => (projectId ? s.projects.find((p) => p.id === projectId) : undefined));
  if (!project) return null;
  const base = cn(
    "inline-flex max-w-full items-center gap-1 rounded-full border bg-background/85 px-1.5 py-0.5 text-[11px] leading-none text-foreground shadow-sm",
    className,
  );
  const body = (
    <>
      <ProjectDot color={project.color} className="size-2" />
      <span className="truncate">{project.name}</span>
    </>
  );
  if (!onOpen) return <span className={base}>{body}</span>;
  return (
    <button
      type="button"
      className={cn(base, "outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/60")}
      title={t("gallery.projectOpen")}
      aria-label={`${t("gallery.project")}: ${project.name}`}
      onClick={(e) => {
        e.stopPropagation();
        onOpen(project);
      }}
    >
      {body}
    </button>
  );
}
