"use client";

import { LockKeyhole, Repeat2 } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { colorClass, USER_COLORS } from "@/lib/users/logic";
import { cn } from "@/lib/utils";
import { useUserStore } from "@/stores/user-store";

import { UserAvatar } from "./user-avatar";

/**
 * ナビ上部のユーザー表示と切り替え(docs/36 §5.2)。アバターをクリックするとメニューが開く。
 * 切り替えは、ログアウトしてウィンドウをリロードし、ユーザーの選択画面へ戻る。
 */
export function UserMenu() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const current = useUserStore((s) => s.current);
  const [open, setOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);

  if (!current) return null;

  async function switchUser(cancelJobs: boolean) {
    try {
      await useUserStore.getState().switchUser(cancelJobs);
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "USER_BUSY" && !cancelJobs) {
        setOpen(false);
        setConfirmOpen(true);
        return;
      }
      toast.error(errorMessage(err));
    }
  }

  async function changeColor(color: string) {
    if (!current || color === current.color) return;
    try {
      await useUserStore.getState().setColor(current.id, color);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  return (
    <>
      <Popover open={open} onOpenChange={setOpen}>
        <Tooltip>
          <TooltipTrigger
            render={
              <PopoverTrigger
                aria-label={`${t("user.menu.label")}: ${current.name}`}
                className="flex size-9 items-center justify-center rounded-md outline-none transition-colors hover:bg-sidebar-accent focus-visible:ring-1 focus-visible:ring-ring"
              />
            }
          >
            <UserAvatar
              name={current.name}
              color={current.color}
              className="size-7 text-sm"
            />
          </TooltipTrigger>
          <TooltipContent side="right">{current.name}</TooltipContent>
        </Tooltip>
        <PopoverContent side="right" align="start" className="w-60 gap-3">
          <div className="flex items-center gap-3">
            <UserAvatar
              name={current.name}
              color={current.color}
              className="size-10 text-lg"
            />
            <div className="min-w-0">
              <div className="truncate font-medium">{current.name}</div>
              <div className="text-xs text-muted-foreground">
                {current.authKind === "none"
                  ? t("user.menu.unlocked")
                  : t("user.menu.locked")}
              </div>
            </div>
          </div>
          <div
            role="radiogroup"
            aria-label={t("user.menu.color")}
            className="flex flex-wrap gap-1.5"
          >
            {USER_COLORS.map((c) => (
              <button
                key={c}
                type="button"
                role="radio"
                aria-checked={current.color === c}
                aria-label={c}
                onClick={() => void changeColor(c)}
                className={cn(
                  "size-6 rounded-full outline-none ring-offset-2 ring-offset-popover focus-visible:ring-2 focus-visible:ring-ring",
                  colorClass(c),
                  current.color === c && "ring-2 ring-foreground",
                )}
              />
            ))}
          </div>
          <Button variant="outline" onClick={() => void switchUser(false)}>
            <Repeat2 />
            {t("user.menu.switch")}
          </Button>
          <Button
            variant="ghost"
            render={<Link href="/settings?scope=user&tab=other" />}
            onClick={() => setOpen(false)}
          >
            <LockKeyhole />
            {t("user.menu.lockSettings")}
          </Button>
        </PopoverContent>
      </Popover>
      <ConfirmDialog
        open={confirmOpen}
        title={t("user.switch.busyTitle")}
        description={t("user.switch.busyDesc")}
        confirmLabel={t("user.switch.busyConfirm")}
        cancelLabel={t("common.cancel")}
        onOpenChange={setConfirmOpen}
        onConfirm={() => void switchUser(true)}
      />
    </>
  );
}
