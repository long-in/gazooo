"use client";

import { useRef } from "react";

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogAction,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { useT } from "@/lib/i18n/use-t";

/** 削除の確認(ファイルは OS のゴミ箱へ移動される)。 */
export function DeleteConfirm({
  open,
  count,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  count: number;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
}) {
  const t = useT();
  const actionRef = useRef<HTMLButtonElement>(null);
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      {/* 初期フォーカスを [削除] に置き、Enter で確定できるようにする(docs/37 §10.2)。
          開くキーの押しっぱなし(repeat)で即確定しないよう、リピートの Enter は無視する。 */}
      <AlertDialogContent
        initialFocus={actionRef}
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.repeat) e.preventDefault();
        }}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("gallery.deleteTitle")}
            {count > 1 && ` (${count})`}
          </AlertDialogTitle>
          <AlertDialogDescription>{t("gallery.deleteDesc")}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("gallery.deleteCancel")}</AlertDialogCancel>
          <AlertDialogAction ref={actionRef} variant="destructive" onClick={onConfirm}>
            {t("gallery.deleteConfirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
