"use client";

import { useCallback } from "react";
import { toast } from "sonner";

import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import type { ImageItem } from "@/lib/tauri/jobs";
import { useGalleryStore } from "@/stores/gallery-store";

/** 直前の所属ごとに画像をまとめる(「元に戻す」で、元の所属へ戻すため)。 */
export function groupByProject(images: Pick<ImageItem, "id" | "projectId">[]): Map<string | null, string[]> {
  const groups = new Map<string | null, string[]>();
  for (const img of images) {
    const list = groups.get(img.projectId) ?? [];
    list.push(img.id);
    groups.set(img.projectId, list);
  }
  return groups;
}

/**
 * 画像を別のプロジェクトへ移す操作(docs/40 §5.3)。成功したら「元に戻す」つきのトーストを出す。
 * 移動前の所属は、`images`(呼び出し側が持つ現在の項目)から取る。
 */
export function useMoveImages() {
  const t = useT();
  const errorMessage = useErrorMessage();

  return useCallback(
    async (images: Pick<ImageItem, "id" | "projectId">[], projectId: string | null) => {
      const targets = images.filter((i) => i.projectId !== projectId);
      if (targets.length === 0) return;
      const before = groupByProject(targets);
      try {
        await useGalleryStore.getState().moveToProject(
          targets.map((i) => i.id),
          projectId,
        );
      } catch (e) {
        toast.error(errorMessage(toAppError(e)));
        return;
      }
      const undo = async () => {
        try {
          for (const [prev, ids] of before) await useGalleryStore.getState().moveToProject(ids, prev);
        } catch (e) {
          toast.error(errorMessage(toAppError(e)));
        }
      };
      toast.success(t("gallery.moved"), { action: { label: t("project.undo"), onClick: () => void undo() } });
    },
    [t, errorMessage],
  );
}
