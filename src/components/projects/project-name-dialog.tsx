"use client";

import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useT } from "@/lib/i18n/use-t";
import { PROJECT_COLORS, type ProjectColor } from "@/lib/tauri/projects";
import { ACCENTS, type UserColor } from "@/lib/users/logic";
import { cn } from "@/lib/utils";

const MAX_NAME_CHARS = 60;

/**
 * プロジェクトの作成・名前と色の変更(docs/40 §5.4)。開くたびに入力を空(または現在値)から始めるため、
 * 呼び出し側は閉じている間はマウントしない。
 */
export function ProjectNameDialog({
  mode,
  initialName = "",
  initialColor = null,
  error,
  busy,
  onOpenChange,
  onSubmit,
}: {
  mode: "create" | "edit";
  initialName?: string;
  initialColor?: ProjectColor | null;
  error?: string | null;
  busy?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (name: string, color: ProjectColor | null) => void;
}) {
  const t = useT();
  const [name, setName] = useState(initialName);
  const [color, setColor] = useState<ProjectColor | null>(initialColor);
  const trimmed = name.trim();
  const valid = trimmed !== "" && [...trimmed].length <= MAX_NAME_CHARS;

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (valid && !busy) onSubmit(trimmed, color);
          }}
        >
          <DialogHeader>
            <DialogTitle>{mode === "create" ? t("project.createTitle") : t("project.renameTitle")}</DialogTitle>
            <DialogDescription className="sr-only">{t("project.name")}</DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2">
            <Label htmlFor="project-name">{t("project.name")}</Label>
            <Input
              id="project-name"
              autoFocus
              value={name}
              maxLength={MAX_NAME_CHARS}
              placeholder={t("project.namePlaceholder")}
              aria-invalid={error ? true : undefined}
              onChange={(e) => setName(e.target.value)}
            />
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
          </div>
          <div className="flex flex-col gap-2">
            <span id="project-color-label" className="text-sm font-medium">
              {t("project.color")}
            </span>
            <div role="radiogroup" aria-labelledby="project-color-label" className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                role="radio"
                aria-checked={color === null}
                size="xs"
                variant={color === null ? "secondary" : "outline"}
                onClick={() => setColor(null)}
              >
                {t("project.colorNone")}
              </Button>
              {PROJECT_COLORS.map((c) => (
                <button
                  key={c}
                  type="button"
                  role="radio"
                  aria-checked={color === c}
                  aria-label={c}
                  title={c}
                  className={cn(
                    "size-6 rounded-full border-2 border-transparent outline-none focus-visible:ring-2 focus-visible:ring-ring/60",
                    color === c && "border-foreground",
                  )}
                  style={{ backgroundColor: ACCENTS[c as UserColor].light.bg }}
                  onClick={() => setColor(c)}
                />
              ))}
            </div>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t("project.cancel")}
            </Button>
            <Button type="submit" disabled={!valid || busy}>
              {mode === "create" ? t("project.create") : t("project.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
