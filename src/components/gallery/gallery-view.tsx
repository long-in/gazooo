"use client";

import { Download, LayoutGrid, Maximize2, Search, Star, X } from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { MoveToProjectMenu } from "@/components/projects/move-to-project";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useErrorMessage, useLanguage, useT } from "@/lib/i18n/use-t";
import { formatCount } from "@/lib/projects/scope-nav";
import { activeEntry, decodeEntry, encodeEntry, type ScopeEntry, selectEntry, validEntry } from "@/lib/projects/scope";
import { toAppError } from "@/lib/tauri/errors";
import { useGalleryStore } from "@/stores/gallery-store";
import { useProjectStore } from "@/stores/project-store";
import { useUiStore } from "@/stores/ui-store";

import { DeleteConfirm } from "./delete-confirm";
import { DeleteNonFavorites } from "./delete-non-favorites";
import { GalleryGrid } from "./gallery-grid";
import { GalleryScopePanel } from "./gallery-scope-panel";
import { GalleryViewer } from "./gallery-viewer";
import { ImageDetailSheet } from "./image-detail-sheet";
import { useBulkExport } from "./use-bulk-export";

const SEARCH_DEBOUNCE_MS = 300;

export function GalleryView() {
  const t = useT();
  const lang = useLanguage();
  const errorMessage = useErrorMessage();
  const router = useRouter();
  const openId = useSearchParams().get("id");

  const items = useGalleryStore((s) => s.items);
  const loading = useGalleryStore((s) => s.loading);
  const loaded = useGalleryStore((s) => s.loaded);
  const error = useGalleryStore((s) => s.error);
  const hasMore = useGalleryStore((s) => s.nextCursor !== null);
  const favoritesOnly = useGalleryStore((s) => s.favoritesOnly);
  const scope = useGalleryStore((s) => s.scope);
  const selected = useGalleryStore((s) => s.selected);
  const viewerId = useGalleryStore((s) => s.viewerId);
  // 表示方法はユーザーごとの画面状態(settings.uiState)に保存する(docs/37 §2)
  const viewerMode = useUiStore((s) => s.panels["gallery.viewer"] === true);
  const projects = useProjectStore((s) => s.projects);
  const projectsLoaded = useProjectStore((s) => s.loaded);
  const uiHydrated = useUiStore((s) => s.hydrated);
  const { reload, loadMore, setQuery, setFavoritesOnly, toggleSelect, selectRange, setViewerId, setFavorite, clearSelection, selectAll, setFavoriteMany, remove } =
    useGalleryStore.getState();

  const bulkExport = useBulkExport();
  const [text, setText] = useState(useGalleryStore.getState().query);
  const [confirmBulkDelete, setConfirmBulkDelete] = useState(false);
  const [viewerDeleteId, setViewerDeleteId] = useState<string | null>(null);

  // 保存されたスコープを 1 回だけ復元する(画面状態・プロジェクト一覧が揃ってから。消えたプロジェクトは「すべて」へ)
  const scopeReady = useGalleryStore((s) => s.scopeRestored);
  useEffect(() => {
    if (scopeReady || !uiHydrated || !projectsLoaded) return;
    const saved = validEntry(decodeEntry(useUiStore.getState().galleryScope), new Set(projects.map((p) => p.id)));
    const state = selectEntry(saved);
    useGalleryStore.setState({ scope: state.scope, favoritesOnly: state.favoritesOnly, scopeRestored: true });
  }, [scopeReady, uiHydrated, projectsLoaded, projects]);

  // 表示中のプロジェクトが削除されたら「すべて」へ戻す
  useEffect(() => {
    if (!scopeReady || typeof scope !== "object") return;
    if (!projects.some((p) => p.id === scope.projectId)) onSelectEntry("all");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scopeReady, scope, projects]);

  // スコープ・お気に入りの変更をユーザーごとの画面状態へ保存する(お気に入りのスコープはスイッチと常に同期)
  useEffect(() => {
    if (scopeReady) useUiStore.getState().setGalleryScope(encodeEntry(activeEntry({ scope, favoritesOnly })));
  }, [scopeReady, scope, favoritesOnly]);

  function onSelectEntry(entry: ScopeEntry) {
    const state = selectEntry(entry);
    useGalleryStore.setState({ scope: state.scope, favoritesOnly: state.favoritesOnly });
    clearSelection();
  }

  // 検索語・お気に入りフィルタ・スコープの変更を(入力が止まってから)反映して読み込み直す。初回は即時。
  const first = useRef(true);
  useEffect(() => {
    if (!scopeReady) return;
    const delay = first.current ? 0 : SEARCH_DEBOUNCE_MS;
    first.current = false;
    const timer = setTimeout(() => {
      setQuery(text);
      void reload();
    }, delay);
    return () => clearTimeout(timer);
  }, [scopeReady, text, favoritesOnly, scope, setQuery, reload]);

  const open = useCallback((id: string) => router.push(`/gallery/?id=${encodeURIComponent(id)}`), [router]);
  const close = useCallback(() => router.replace("/gallery/"), [router]);

  async function bulk(action: () => Promise<unknown>, okKey?: Parameters<typeof t>[0]) {
    try {
      await action();
      if (okKey) toast.success(t(okKey));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const ids = [...selected];
  const filtered = text.trim() !== "" || favoritesOnly;
  // 全プロジェクトを横断して見ているときだけ、画像の所属をチップで示す(docs/40 §5.2)
  const showProject = scope === "all";
  const openProject = useCallback((id: string) => onSelectEntry({ projectId: id }), []); // eslint-disable-line react-hooks/exhaustive-deps
  const totals = useProjectStore((s) => s.totals);
  const activeScope = activeEntry({ scope, favoritesOnly });
  const heading = (() => {
    if (activeScope === "all") return { name: t("scope.all"), count: totals.total };
    if (activeScope === "favorites") return { name: t("scope.favorites"), count: totals.favorites };
    if (activeScope === "unassigned") return { name: t("scope.unassigned"), count: totals.unassigned };
    const p = projects.find((x) => x.id === activeScope.projectId);
    return { name: p?.name ?? "", count: p?.imageCount ?? 0 };
  })();
  const selectedImages = items.filter((i) => selected.has(i.id));

  return (
    <div className="flex h-full min-h-0">
      <GalleryScopePanel active={activeEntry({ scope, favoritesOnly })} onSelect={onSelectEntry} />
      <div className="flex min-w-0 flex-1 flex-col gap-4 p-6">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="sr-only">{t("gallery.title")}</h1>
        <p className="flex min-w-0 max-w-full items-baseline gap-1.5 text-sm font-medium" aria-live="polite">
          <span className="truncate">{heading.name}</span>
          <span className="shrink-0 text-xs font-normal tabular-nums text-muted-foreground">
            {formatCount(heading.count, lang)} {t("scope.unit")}
            {filtered && ` (${t("scope.narrowed")})`}
          </span>
        </p>
        <div className="relative min-w-56 flex-1 sm:max-w-sm">
          <Search aria-hidden className="pointer-events-none absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
          <Input
            type="search"
            className="pl-8"
            value={text}
            placeholder={t("gallery.search")}
            aria-label={t("gallery.search")}
            onChange={(e) => setText(e.target.value)}
          />
        </div>
        <label className="flex items-center gap-2 text-sm">
          <Switch checked={favoritesOnly} onCheckedChange={setFavoritesOnly} />
          <Star aria-hidden className="size-4 text-amber-400" />
          {t("gallery.favoritesOnly")}
        </label>
        <div role="radiogroup" aria-label={t("gallery.viewMode")} className="ml-auto flex rounded-lg border p-0.5">
          {([
            { viewer: false, label: t("gallery.viewGrid"), Icon: LayoutGrid },
            { viewer: true, label: t("gallery.viewViewer"), Icon: Maximize2 },
          ] as const).map(({ viewer, label, Icon }) => (
            <Button
              key={label}
              type="button"
              role="radio"
              aria-checked={viewerMode === viewer}
              aria-label={label}
              title={label}
              variant={viewerMode === viewer ? "secondary" : "ghost"}
              size="icon-sm"
              onClick={() => useUiStore.getState().setPanelOpen("gallery.viewer", viewer)}
            >
              <Icon />
            </Button>
          ))}
        </div>
        <DeleteNonFavorites scopeName={heading.name} searching={text.trim() !== ""} favoritesScope={activeScope === "favorites"} />
      </div>

      {!viewerMode && selected.size > 0 && (
        <div role="toolbar" className="flex flex-wrap items-center gap-2 rounded-lg border bg-muted/40 px-3 py-2">
          <span className="text-sm font-medium">
            {selected.size} {t("gallery.selected")}
          </span>
          <Button type="button" variant="ghost" size="sm" onClick={selectAll}>
            {t("gallery.selectAll")}
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => void bulk(() => setFavoriteMany(ids, true))}>
            <Star />
            {t("gallery.favoriteMany")}
          </Button>
          <MoveToProjectMenu images={selectedImages} align="start" />
          {bulkExport.state ? (
            <>
              <span className="text-sm tabular-nums" aria-live="polite">
                {t("gallery.export.running")} {bulkExport.state.done} / {bulkExport.state.total}
              </span>
              <Button type="button" variant="outline" size="sm" onClick={bulkExport.cancel}>
                {t("gallery.export.cancel")}
              </Button>
            </>
          ) : (
            <Button type="button" variant="outline" size="sm" onClick={() => void bulkExport.start(ids)}>
              <Download />
              {t("gallery.exportMany")}
            </Button>
          )}
          <Button type="button" variant="outline" size="sm" className="text-destructive" onClick={() => setConfirmBulkDelete(true)}>
            {t("gallery.delete")}
          </Button>
          <Button type="button" variant="ghost" size="icon-sm" className="ml-auto" aria-label={t("gallery.clearSelection")} title={t("gallery.clearSelection")} onClick={clearSelection}>
            <X />
          </Button>
        </div>
      )}

      {error && items.length === 0 ? (
        <p role="alert" className="text-sm text-destructive">
          {t("gallery.loadFailed")}: {errorMessage(error)}
        </p>
      ) : loaded && items.length === 0 ? (
        <p className="rounded-xl border border-dashed p-12 text-center text-sm text-muted-foreground">
          {filtered ? t("gallery.noResults") : t("gallery.empty")}
        </p>
      ) : viewerMode ? (
        <GalleryViewer
          items={items}
          selectedId={viewerId}
          hasMore={hasMore}
          loading={loading}
          onSelect={setViewerId}
          onLoadMore={() => void loadMore()}
          onOpen={open}
          onToggleFavorite={(img) => void bulk(() => setFavorite(img.id, !img.isFavorite))}
          onSetFavorite={(img, value) => void bulk(() => setFavorite(img.id, value))}
          onDelete={setViewerDeleteId}
          showProject={showProject}
          onOpenProject={openProject}
        />
      ) : (
        <GalleryGrid
          items={items}
          hasMore={hasMore}
          loading={loading}
          selected={selected}
          onLoadMore={() => void loadMore()}
          onOpen={open}
          onToggleSelect={toggleSelect}
          onSelectRange={selectRange}
          showProject={showProject}
          onOpenProject={openProject}
        />
      )}
      {loading && items.length > 0 && <p className="text-center text-xs text-muted-foreground">{t("gallery.loadingMore")}</p>}

      <ImageDetailSheet id={openId} onClose={close} />

      <DeleteConfirm
        open={viewerDeleteId !== null}
        count={1}
        onOpenChange={(v) => !v && setViewerDeleteId(null)}
        onConfirm={() => {
          const id = viewerDeleteId;
          setViewerDeleteId(null);
          if (id) void bulk(() => remove([id], true), "gallery.deleted");
        }}
      />

      <DeleteConfirm
        open={confirmBulkDelete}
        count={ids.length}
        onOpenChange={setConfirmBulkDelete}
        onConfirm={() => {
          setConfirmBulkDelete(false);
          void bulk(() => remove(ids, true), "gallery.deleted");
        }}
      />
      </div>
    </div>
  );
}
