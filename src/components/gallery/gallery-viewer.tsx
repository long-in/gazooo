"use client";

import { useVirtualizer } from "@tanstack/react-virtual";
import { Star, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { MoveToProjectMenu } from "@/components/projects/move-to-project";
import { ProjectChip } from "@/components/projects/project-dot";
import { Button } from "@/components/ui/button";
import { useT } from "@/lib/i18n/use-t";
import { imageSrc } from "@/lib/image-src";
import { detectMac, modifierLabel } from "@/lib/shortcuts";
import { imageGet } from "@/lib/tauri/images";
import type { ImageItem } from "@/lib/tauri/jobs";
import { cn } from "@/lib/utils";
import { isTypingTarget, moveViewer, viewerKeyAction } from "@/lib/viewer-nav";

const THUMB = 80;
const GAP = 8;
const FULL_DELAY_MS = 80;

/** ビューア表示: 選択中の画像を大きく、下部に横一列のストリップ(docs/37)。 */
export function GalleryViewer({
  items,
  selectedId,
  hasMore,
  loading,
  onSelect,
  onLoadMore,
  onOpen,
  onToggleFavorite,
  onSetFavorite,
  onDelete,
  showProject = false,
  onOpenProject,
}: {
  items: ImageItem[];
  selectedId: string | null;
  hasMore: boolean;
  loading: boolean;
  onSelect: (id: string) => void;
  onLoadMore: () => void;
  onOpen: (id: string) => void;
  onToggleFavorite: (img: ImageItem) => void;
  /** お気に入りを指定の値にする(⌘G / ⌘C。docs/37 §10)。 */
  onSetFavorite: (img: ImageItem, value: boolean) => void;
  onDelete: (id: string) => void;
  /** 所属プロジェクトのチップを出す(全プロジェクト横断の表示のとき。docs/40 §5.2)。 */
  showProject?: boolean;
  onOpenProject?: (projectId: string) => void;
}) {
  const t = useT();
  const rootRef = useRef<HTMLDivElement>(null);
  const stripRef = useRef<HTMLDivElement>(null);
  const isMac = detectMac();
  const mod = modifierLabel(isMac);

  const index = Math.max(
    0,
    items.findIndex((i) => i.id === selectedId),
  );
  const current: ImageItem | undefined = items[index];
  const currentId = current?.id ?? null;

  // 原寸は、選択が止まってから読み込む(連続移動中はサムネイルだけ。先読みは前後 1 枚)
  const [fullId, setFullId] = useState<string | null>(null);
  useEffect(() => {
    const timer = setTimeout(() => setFullId(currentId), FULL_DELAY_MS);
    return () => clearTimeout(timer);
  }, [currentId]);
  const fullReady = currentId !== null && fullId === currentId;

  const prevPath = items[index - 1]?.filePath;
  const nextPath = items[index + 1]?.filePath;
  useEffect(() => {
    if (!fullReady) return;
    for (const p of [prevPath, nextPath]) if (p) new Image().src = imageSrc(p);
  }, [fullReady, prevPath, nextPath]);

  // 情報行のプロンプトは詳細から取る(一覧の項目には含まれない)
  const [prompt, setPrompt] = useState<{ id: string; text: string } | null>(null);
  useEffect(() => {
    if (!fullReady || !currentId) return;
    let cancelled = false;
    imageGet(currentId)
      .then((d) => {
        if (!cancelled) setPrompt({ id: currentId, text: d.job.params.prompt });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [fullReady, currentId]);
  const promptText = prompt && prompt.id === currentId ? prompt.text : "";

  // useVirtualizer はメモ化できない関数を返す(gallery-grid.tsx と同じ理由)
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    horizontal: true,
    count: items.length,
    getScrollElement: () => stripRef.current,
    estimateSize: () => THUMB + GAP,
    overscan: 8,
  });

  // 選択中のサムネイルを可視範囲へ(なるべく中央)
  const scrollToIndex = virtualizer.scrollToIndex;
  const hasItems = items.length > 0;
  useEffect(() => {
    if (!hasItems) return;
    const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    scrollToIndex(index, { align: "center", behavior: reduce ? "auto" : "smooth" });
  }, [index, hasItems, scrollToIndex]);

  // 縦ホイールを横スクロールにする
  useEffect(() => {
    const el = stripRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        e.preventDefault();
        el.scrollLeft += e.deltaY;
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // 右端に近づいたら続きを読み込む
  const virtualItems = virtualizer.getVirtualItems();
  const lastIdx = virtualItems.length > 0 ? virtualItems[virtualItems.length - 1].index : -1;
  useEffect(() => {
    if (hasMore && !loading && lastIdx >= items.length - 8) onLoadMore();
  }, [hasMore, loading, lastIdx, items.length, onLoadMore]);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const action = viewerKeyAction(e.nativeEvent, isMac);
      if (!action || !current || isTypingTarget(e.target as Element)) return;
      // テキスト選択中の ⌘C / Ctrl+C は OS のコピーに任せる(docs/37 §10.1)
      if (action === "favoriteOff" && (window.getSelection()?.toString() ?? "") !== "") return;
      e.preventDefault();
      if (action === "open") onOpen(current.id);
      else if (action === "favorite") onToggleFavorite(current);
      else if (action === "favoriteOn") {
        if (!current.isFavorite) onSetFavorite(current, true);
      } else if (action === "favoriteOff") {
        if (current.isFavorite) onSetFavorite(current, false);
      }
      else if (action === "delete") onDelete(current.id);
      else {
        const next = moveViewer(
          items.map((i) => i.id),
          current.id,
          action,
        );
        if (next) onSelect(next);
      }
    },
    [current, items, isMac, onOpen, onSelect, onToggleFavorite, onSetFavorite, onDelete],
  );

  // 開いた時点でフォーカスをメイン領域に置く(← → がそのまま効く)
  useEffect(() => {
    rootRef.current?.focus();
  }, []);

  if (!current) return null;

  return (
    <div
      ref={rootRef}
      tabIndex={0}
      aria-label={t("gallery.viewer")}
      className="flex min-h-0 flex-1 flex-col gap-2 outline-none"
      onKeyDown={onKeyDown}
    >
      <div
        className="relative min-h-0 flex-1 overflow-hidden rounded-lg border bg-muted/30"
        onDoubleClick={() => onOpen(current.id)}
      >
        {/* 先にサムネイルを出し、原寸が揃ったら重ねる */}
        <img src={imageSrc(current.thumbPath)} alt="" className="absolute inset-0 size-full object-contain" />
        {fullReady && (
          <img
            key={current.id}
            src={imageSrc(current.filePath)}
            alt={promptText.slice(0, 120)}
            decoding="async"
            className="absolute inset-0 size-full object-contain"
          />
        )}
      </div>

      <div className="flex items-center gap-3 text-sm">
        {showProject && current.projectId && (
          <ProjectChip projectId={current.projectId} onOpen={onOpenProject ? (p) => onOpenProject(p.id) : undefined} />
        )}
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{promptText}</span>
        <span className="shrink-0 tabular-nums text-muted-foreground">
          {current.width}×{current.height} · {index + 1} / {items.length}
          {hasMore ? "+" : ""}
        </span>
        <MoveToProjectMenu images={[current]} />
        <Button type="button" variant="outline" size="sm" onClick={() => onOpen(current.id)}>
          {t("gallery.viewerDetail")}
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={current.isFavorite ? t("gallery.unfavorite") : t("gallery.favorite")}
          title={current.isFavorite ? `${t("gallery.unfavorite")} (${mod}C)` : `${t("gallery.favorite")} (${mod}G)`}
          aria-pressed={current.isFavorite}
          onClick={() => onToggleFavorite(current)}
        >
          <Star className={cn(current.isFavorite && "fill-amber-400 text-amber-400")} />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="text-destructive"
          aria-label={t("gallery.delete")}
          title={`${t("gallery.delete")} (${mod}${isMac ? "⌫" : "Backspace"})`}
          onClick={() => onDelete(current.id)}
        >
          <Trash2 />
        </Button>
      </div>

      <div
        ref={stripRef}
        role="listbox"
        aria-label={t("gallery.viewerStrip")}
        aria-orientation="horizontal"
        aria-activedescendant={`viewer-opt-${current.id}`}
        className="shrink-0 overflow-x-auto overflow-y-hidden"
        style={{ height: THUMB + 12 }}
      >
        <div className="relative h-full" style={{ width: virtualizer.getTotalSize() }}>
          {virtualItems.map((v) => {
            const img = items[v.index];
            if (!img) return null;
            const active = img.id === current.id;
            return (
              <div
                key={img.id}
                id={`viewer-opt-${img.id}`}
                role="option"
                aria-selected={active}
                className={cn(
                  "absolute top-1 cursor-pointer overflow-hidden rounded-md border bg-muted/30",
                  active && "ring-2 ring-primary",
                )}
                style={{ left: v.start, width: THUMB, height: THUMB }}
                onClick={() => {
                  onSelect(img.id);
                  rootRef.current?.focus();
                }}
              >
                <img src={imageSrc(img.thumbPath)} alt="" loading="lazy" decoding="async" className="size-full object-cover" />
                {img.isFavorite && (
                  <Star aria-hidden className="absolute right-0.5 top-0.5 size-3 fill-amber-400 text-amber-400 drop-shadow" />
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
