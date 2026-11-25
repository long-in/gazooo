"use client";

import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, Star } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { ProjectChip } from "@/components/projects/project-dot";
import { imageSrc } from "@/lib/image-src";
import type { ImageItem } from "@/lib/tauri/jobs";
import { cn } from "@/lib/utils";

const GAP = 8;
const MIN_CELL = 176;

/** 仮想化したサムネイルグリッド(行単位で描画し、末尾付近でさらに読み込む)。 */
export function GalleryGrid({
  items,
  hasMore,
  loading,
  selected,
  onLoadMore,
  onOpen,
  onToggleSelect,
  onSelectRange,
  showProject = false,
  onOpenProject,
}: {
  items: ImageItem[];
  hasMore: boolean;
  loading: boolean;
  selected: Set<string>;
  onLoadMore: () => void;
  onOpen: (id: string) => void;
  onToggleSelect: (id: string) => void;
  onSelectRange: (id: string) => void;
  /** 所属プロジェクトのチップを出す(全プロジェクト横断の表示のとき。docs/40 §5.2)。 */
  showProject?: boolean;
  onOpenProject?: (projectId: string) => void;
}) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);

  useEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const cols = Math.max(1, Math.floor((width + GAP) / (MIN_CELL + GAP)));
  const cell = width > 0 ? (width - GAP * (cols - 1)) / cols : MIN_CELL;
  const rowCount = Math.ceil(items.length / cols);

  // useVirtualizer はメモ化できない関数を返す。React Compiler がこのコンポーネントを最適化対象から外すだけで、動作に影響はない
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => cell + GAP,
    overscan: 4,
  });

  // 列数・セルサイズが変わったら行の高さを測り直す
  useEffect(() => {
    virtualizer.measure();
  }, [cell, cols, virtualizer]);

  // 末尾の数行に近づいたら次のページを読み込む
  const virtualRows = virtualizer.getVirtualItems();
  const lastRow = virtualRows.length > 0 ? virtualRows[virtualRows.length - 1].index : -1;
  useEffect(() => {
    if (hasMore && !loading && lastRow >= rowCount - 3) onLoadMore();
  }, [hasMore, loading, lastRow, rowCount, onLoadMore]);

  const selectionActive = selected.size > 0;

  return (
    <div ref={parentRef} className="min-h-0 flex-1 overflow-y-auto" data-testid="gallery-scroll">
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualRows.map((row) => {
          const start = row.index * cols;
          const rowItems = items.slice(start, start + cols);
          return (
            <div
              key={row.key}
              className="absolute left-0 top-0 grid w-full"
              style={{
                transform: `translateY(${row.start}px)`,
                height: cell,
                gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
                gap: GAP,
              }}
            >
              {rowItems.map((img) => {
                const isSelected = selected.has(img.id);
                return (
                  <button
                    key={img.id}
                    type="button"
                    aria-label={`${img.width}×${img.height} seed ${img.seed}`}
                    aria-pressed={selectionActive ? isSelected : undefined}
                    className={cn(
                      "group relative overflow-hidden rounded-lg border bg-muted/30 outline-none focus-visible:ring-3 focus-visible:ring-ring/50",
                      isSelected && "ring-2 ring-primary",
                    )}
                    onMouseDown={(e) => {
                      if (e.shiftKey) e.preventDefault(); // テキスト選択を防ぐ
                    }}
                    onClick={(e) => {
                      if (e.shiftKey) onSelectRange(img.id);
                      else if (selectionActive || e.ctrlKey || e.metaKey) onToggleSelect(img.id);
                      else onOpen(img.id);
                    }}
                  >
                    <img
                      src={imageSrc(img.thumbPath)}
                      alt=""
                      loading="lazy"
                      decoding="async"
                      className="size-full object-cover"
                    />
                    {img.isFavorite && (
                      <Star aria-hidden className="absolute right-1.5 top-1.5 size-4 fill-amber-400 text-amber-400 drop-shadow" />
                    )}
                    {showProject && img.projectId && (
                      <ProjectChip
                        projectId={img.projectId}
                        onOpen={onOpenProject ? (p) => onOpenProject(p.id) : undefined}
                        className="absolute bottom-1.5 left-1.5 max-w-[85%]"
                      />
                    )}
                    <span
                      aria-hidden
                      className={cn(
                        "absolute left-1.5 top-1.5 flex size-5 items-center justify-center rounded-full border bg-background/80 text-primary opacity-0 transition-opacity group-hover:opacity-100",
                        isSelected && "bg-primary text-primary-foreground opacity-100",
                        selectionActive && "opacity-100",
                      )}
                      onClick={(e) => {
                        e.stopPropagation();
                        onToggleSelect(img.id);
                      }}
                    >
                      {isSelected && <Check className="size-3" />}
                    </span>
                  </button>
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}
