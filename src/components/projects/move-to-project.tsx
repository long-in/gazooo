"use client";

import { Check, FolderInput, FolderPlus } from "lucide-react";
import { type ReactNode, useState } from "react";

import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useT } from "@/lib/i18n/use-t";
import { useMoveImages } from "@/lib/projects/use-move-images";
import type { ImageItem } from "@/lib/tauri/jobs";
import { useProjectStore } from "@/stores/project-store";

import { ProjectDot } from "./project-dot";
import { useProjectDialogs } from "./use-project-dialogs";

const menuItem = "w-full justify-start";

/**
 * 画像の移動先を選ぶメニュー(docs/40 §5.3)。移動先は「未分類」・各プロジェクト・「新規プロジェクト…」。
 * `images` は移動する画像(複数可)。1 枚で所属が分かるときは、現在の所属にチェックを付ける。
 */
export function MoveToProjectMenu({
  images,
  trigger,
  align = "end",
}: {
  images: Pick<ImageItem, "id" | "projectId">[];
  /** トリガーの中身(省略時はアイコン + 「プロジェクトへ移動」)。 */
  trigger?: ReactNode;
  align?: "start" | "center" | "end";
}) {
  const t = useT();
  const move = useMoveImages();
  const projects = useProjectStore((s) => s.projects);
  const dialogs = useProjectDialogs();
  const [open, setOpen] = useState(false);

  const only = images.length === 1 ? images[0] : null;

  const run = (projectId: string | null) => {
    setOpen(false);
    void move(images, projectId);
  };

  return (
    <>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger
          render={<Button type="button" variant="outline" size="sm" disabled={images.length === 0} title={t("gallery.move")} />}
        >
          {trigger ?? (
            <>
              <FolderInput />
              {t("gallery.move")}
            </>
          )}
        </PopoverTrigger>
        <PopoverContent align={align} className="flex max-h-80 w-60 flex-col gap-1 overflow-y-auto p-2">
          <p className="px-2 py-1 text-xs text-muted-foreground">{t("gallery.moveTo")}</p>
          <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => run(null)}>
            <ProjectDot color={null} />
            <span className="min-w-0 flex-1 truncate text-left">{t("project.none")}</span>
            {only && only.projectId === null && <Check aria-hidden />}
          </Button>
          {projects.map((p) => (
            <Button key={p.id} type="button" size="sm" variant="ghost" className={menuItem} onClick={() => run(p.id)}>
              <ProjectDot color={p.color} />
              <span className="min-w-0 flex-1 truncate text-left">{p.name}</span>
              {only && only.projectId === p.id && <Check aria-hidden />}
            </Button>
          ))}
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className={menuItem}
            onClick={() => {
              setOpen(false);
              dialogs.open({ kind: "create", onCreated: (p) => void move(images, p.id) });
            }}
          >
            <FolderPlus />
            {t("project.new")}
          </Button>
        </PopoverContent>
      </Popover>
      {dialogs.element}
    </>
  );
}
