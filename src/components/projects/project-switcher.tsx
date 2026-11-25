"use client";

import { FolderPlus } from "lucide-react";
import Link from "next/link";
import { toast } from "sonner";

import { Button, buttonVariants } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { cn } from "@/lib/utils";
import { useProjectStore } from "@/stores/project-store";

import { ProjectDot } from "./project-dot";
import { useProjectDialogs } from "./use-project-dialogs";

const NONE = "__none__";

/**
 * 生成ページ上部のプロジェクト選択(docs/40 §4.1)。切り替えると、そのプロジェクトの前回の設定をフォームへ適用する
 * (確認ダイアログは出さず、「元に戻す」つきのトーストで知らせる)。生成した画像は選んでいるプロジェクトへ入る。
 */
export function ProjectSwitcher() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const projects = useProjectStore((s) => s.projects);
  const currentId = useProjectStore((s) => s.currentId);
  const dialogs = useProjectDialogs();

  async function switchTo(id: string | null) {
    try {
      const r = await useProjectStore.getState().switchTo(id);
      if (r.restored && r.undo) {
        const undo = r.undo;
        toast(t("project.restored"), { action: { label: t("project.undo"), onClick: undo } });
      }
    } catch (e) {
      toast.error(`${t("project.switchFailed")}: ${errorMessage(toAppError(e))}`);
    }
  }

  const current = projects.find((p) => p.id === currentId);

  return (
    <div className="flex items-center gap-2" title={t("project.hint")}>
      <span className="shrink-0 text-sm text-muted-foreground">{t("project.label")}</span>
      <div className="min-w-0 max-w-72 flex-1">
        <Select
          value={currentId ?? NONE}
          items={[
            { value: NONE, label: t("project.none") },
            ...projects.map((p) => ({ value: p.id, label: p.name })),
          ]}
          onValueChange={(v) => v !== null && void switchTo(v === NONE ? null : v)}
        >
          <SelectTrigger className="w-full" aria-label={t("project.switcher")}>
            {current && <ProjectDot color={current.color} />}
            <SelectValue placeholder={t("project.none")} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NONE}>{t("project.none")}</SelectItem>
            {projects.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                <ProjectDot color={p.color} />
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <Button
        type="button"
        variant="outline"
        size="icon"
        aria-label={t("project.new")}
        title={t("project.new")}
        onClick={() => dialogs.open({ kind: "create", onCreated: (p) => void switchTo(p.id) })}
      >
        <FolderPlus />
      </Button>
      <Link href="/gallery" className={cn(buttonVariants({ variant: "ghost", size: "sm" }))}>
        {t("project.manage")}
      </Link>
      {dialogs.element}
    </div>
  );
}
