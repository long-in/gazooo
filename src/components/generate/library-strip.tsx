"use client";

import { ChevronDown, ChevronUp, MoreHorizontal, Pause, Play, Search, Star, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { ImageDetailSheet } from "@/components/gallery/image-detail-sheet";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Switch } from "@/components/ui/switch";
import { useImageActions } from "@/lib/gallery/use-image-as";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { imageSrc } from "@/lib/image-src";
import { stripScope } from "@/lib/projects/scope";
import { matchesQuery } from "@/lib/queue";
import { toAppError } from "@/lib/tauri/errors";
import { type ImageDetail, imageGet, imageSetFavorite } from "@/lib/tauri/images";
import type { ImageItem, Job } from "@/lib/tauri/jobs";
import { cn } from "@/lib/utils";
import { useFormStore } from "@/stores/form-store";
import { useJobStore } from "@/stores/job-store";
import { useProjectStore } from "@/stores/project-store";
import { type StripFilter, useStripStore } from "@/stores/strip-store";
import { useUiStore } from "@/stores/ui-store";

import { JobDetailSheet } from "./job-detail-sheet";
import { QueueCard } from "./queue-card";
import { QueueSummary } from "./queue-summary";

const SEARCH_DEBOUNCE_MS = 300;
const FILTERS: StripFilter[] = ["all", "queue", "images"];
const detailCache = new Map<string, ImageDetail>();

async function getDetail(id: string): Promise<ImageDetail | null> {
  const hit = detailCache.get(id);
  if (hit) return hit;
  try {
    const d = await imageGet(id);
    detailCache.set(id, d);
    return d;
  } catch {
    return null;
  }
}

type Confirm = "cancelQueued" | "retryFailed" | "removeFailed" | null;

/**
 * 生成ページ下部の帯(docs/28)。左がキューのカード(実行中 → 待機中 → 失敗・キャンセル)、右が生成済みの画像。
 * 選択したものに応じて、ヘッダに操作を出す。横スクロール。
 */
export function LibraryStrip() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const actions = useImageActions(true);

  const items = useStripStore((s) => s.items);
  const loading = useStripStore((s) => s.loading);
  const loaded = useStripStore((s) => s.loaded);
  const hasMore = useStripStore((s) => s.nextCursor !== null);
  const favoritesOnly = useStripStore((s) => s.favoritesOnly);
  const collapsed = useStripStore((s) => s.collapsed);
  const selectedId = useStripStore((s) => s.selectedId);
  const selectedJobId = useStripStore((s) => s.selectedJobId);
  const filter = useStripStore((s) => s.filter);
  const scope = useStripStore((s) => s.scope);
  // 帯の範囲: 既定は現在のプロジェクトの画像。[このプロジェクト / すべて] で切り替える(docs/40 §4.3)
  const allProjects = useUiStore((s) => s.panels["strip.allProjects"] === true);
  const currentProjectId = useProjectStore((s) => s.currentId);
  const projectsLoaded = useProjectStore((s) => s.loaded);
  const uiHydrated = useUiStore((s) => s.hydrated);
  const wantedScope = stripScope(allProjects, currentProjectId);
  const wantedKey = typeof wantedScope === "string" ? wantedScope : wantedScope.projectId;
  const scopeReady = uiHydrated && projectsLoaded;
  const { reload, setQuery, setFavoritesOnly, setCollapsed, toggleSelect, select, toggleSelectJob, selectJob, setFilter } =
    useStripStore.getState();

  const jobs = useJobStore((s) => s.jobs);
  const paused = useJobStore((s) => s.paused);
  const { cancel, reorder, retry, remove, enqueue, setPaused } = useJobStore.getState();

  const [text, setText] = useState(useStripStore.getState().query);
  const [detailId, setDetailId] = useState<string | null>(null);
  const [jobDetailId, setJobDetailId] = useState<string | null>(null);
  const [selDetail, setSelDetail] = useState<ImageDetail | null>(null);
  const [confirm, setConfirm] = useState<Confirm>(null);
  const empty = items.length === 0;
  const scrollRef = useRef<HTMLDivElement>(null);
  const lastFocused = useRef<HTMLElement | null>(null);

  // キューのカード: 実行中 → 待機中(実行順)→ 失敗・キャンセル。検索語があればプロンプトで絞る
  const queued = useMemo(() => jobs.filter((j) => j.status === "queued"), [jobs]);
  const failed = useMemo(() => jobs.filter((j) => j.status === "failed" || j.status === "cancelled"), [jobs]);
  const cards = useMemo(() => {
    const running = jobs.filter((j) => j.status === "running");
    return [...running, ...queued, ...failed].filter((j) => matchesQuery(j.params, text));
  }, [jobs, queued, failed, text]);
  const showCards = filter !== "images";
  const showImages = filter !== "queue";

  const run = async (action: () => Promise<unknown>, success?: string) => {
    try {
      await action();
      if (success) toast.success(success);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  };

  // 範囲(現在のプロジェクト / すべて)をストアへ反映する(画面状態・プロジェクト一覧が揃ってから)
  useEffect(() => {
    if (scopeReady) useStripStore.getState().setScope(wantedScope);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scopeReady, wantedKey]);
  const scopeApplied = scopeReady && (typeof scope === "string" ? scope : scope.projectId) === wantedKey;

  // 検索語・お気に入り・範囲の変更を、入力が止まってから反映して読み込み直す(初回は即時)
  const first = useRef(true);
  useEffect(() => {
    if (!scopeApplied) return;
    const delay = first.current ? 0 : SEARCH_DEBOUNCE_MS;
    first.current = false;
    const timer = setTimeout(() => {
      setQuery(text);
      void reload();
    }, delay);
    return () => clearTimeout(timer);
  }, [scopeApplied, text, favoritesOnly, wantedKey, setQuery, reload]);

  // 選択中の画像の詳細(プロンプト・設定)
  useEffect(() => {
    let cancelled = false;
    if (!selectedId) return;
    void getDetail(selectedId).then((d) => {
      if (!cancelled) setSelDetail(d);
    });
    return () => {
      cancelled = true;
    };
  }, [selectedId]);
  const detail = selectedId && selDetail?.image.id === selectedId ? selDetail : null;
  const selectedImage = detail ? (items.find((i) => i.id === detail.image.id) ?? detail.image) : null;
  const params = detail?.job.params;
  // 選択中のカード(完了して消えたら、選択も無くなる)
  const selectedJob = selectedJobId ? (jobs.find((j) => j.id === selectedJobId && j.status !== "completed") ?? null) : null;
  const jobDetail = jobDetailId ? (jobs.find((j) => j.id === jobDetailId) ?? null) : null;

  // 選択したものが帯の中にあれば、見える位置までスクロールする
  const selKey = selectedId ?? selectedJobId;
  useEffect(() => {
    if (!selKey) return;
    scrollRef.current
      ?.querySelector<HTMLElement>(`[data-image-id="${selKey}"], [data-job-id="${selKey}"]`)
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [selKey]);

  // 右端に近づいたら続きを読み込む
  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (hasMore && !loading && showImages && el.scrollWidth - el.scrollLeft - el.clientWidth < 400) {
      void useStripStore.getState().loadMore();
    }
  }, [hasMore, loading, showImages]);
  // 縦のホイール回転は横スクロールに変換する
  const hasBody = showCards ? cards.length > 0 || !empty : !empty;
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
      el.scrollLeft += e.deltaY;
      e.preventDefault();
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [collapsed, hasBody]);
  // 画面幅より少ない件数しか読み込めていない場合に続きを取得する
  useEffect(() => {
    onScroll();
  }, [items.length, onScroll]);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-image-id], [data-job-id]");
    let next = e.key === "ArrowRight" ? el?.nextElementSibling : el?.previousElementSibling;
    if (next && !(next instanceof HTMLElement && (next.dataset.imageId || next.dataset.jobId))) {
      // 区切り線をまたぐ
      next = e.key === "ArrowRight" ? next.nextElementSibling : next.previousElementSibling;
    }
    if (next instanceof HTMLElement) {
      e.preventDefault();
      next.focus();
      if (next.dataset.jobId) selectJob(next.dataset.jobId);
      else select(next.dataset.imageId ?? null);
    }
  }

  async function toggleFavorite(image: ImageItem) {
    try {
      await imageSetFavorite(image.id, !image.isFavorite);
      useStripStore.getState().applyFavorite(image.id, !image.isFavorite);
      setSelDetail((d) => (d ? { ...d, image: { ...d.image, isFavorite: !image.isFavorite } } : d));
      detailCache.delete(image.id);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  async function onConfirm() {
    const which = confirm;
    setConfirm(null);
    if (which === "cancelQueued") {
      await run(async () => {
        for (const j of queued) await cancel(j.id);
      }, t("queue.bulk.cancelled"));
    } else if (which === "retryFailed") {
      await run(async () => {
        for (const j of failed) await retry(j.id);
      }, t("queue.bulk.retried"));
    } else if (which === "removeFailed") {
      await run(async () => {
        await remove(failed.map((j) => j.id));
        selectJob(null);
      }, t("queue.bulk.removed"));
    }
  }

  const confirmText: Record<Exclude<Confirm, null>, { title: MessageKey; desc: MessageKey; ok: MessageKey }> = {
    cancelQueued: { title: "queue.confirm.cancelQueued.title", desc: "queue.confirm.cancelQueued.desc", ok: "queue.cancelQueued" },
    retryFailed: { title: "queue.confirm.retryFailed.title", desc: "queue.confirm.retryFailed.desc", ok: "queue.retryFailed" },
    removeFailed: { title: "library.removeFailed.title", desc: "library.removeFailed.desc", ok: "library.removeFailed" },
  };
  const ct = confirm ? confirmText[confirm] : null;

  const filtered = text.trim() !== "" || favoritesOnly;
  const menuItem = "justify-start";
  const nothing = (!showCards || cards.length === 0) && (!showImages || (loaded && empty));

  return (
    <section aria-label={t("strip.title")} className="flex shrink-0 flex-col gap-1.5 border-t bg-muted/20 px-4 py-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <h2 className="text-sm font-medium">{t("strip.title")}</h2>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-expanded={!collapsed}
          aria-label={collapsed ? t("strip.expand") : t("strip.collapse")}
          title={collapsed ? t("strip.expand") : t("strip.collapse")}
          onClick={() => setCollapsed(!collapsed)}
        >
          {collapsed ? <ChevronUp /> : <ChevronDown />}
        </Button>
        <QueueSummary />
        {paused && (
          <span role="status" className="flex items-center gap-1 text-xs font-medium text-amber-600 dark:text-amber-400">
            <Pause className="size-3" />
            {t("jobbar.paused")}
          </span>
        )}
        <div className="ml-auto flex items-center gap-1">
          <Button type="button" size="xs" variant={paused ? "default" : "outline"} onClick={() => void run(() => setPaused(!paused))}>
            {paused ? <Play /> : <Pause />}
            {paused ? t("queue.resume") : t("queue.pause")}
          </Button>
          <Popover>
            <PopoverTrigger
              render={<Button type="button" size="icon-xs" variant="outline" aria-label={t("queue.menu")} title={t("queue.menu")} />}
            >
              <MoreHorizontal />
            </PopoverTrigger>
            <PopoverContent align="end" className="flex w-64 flex-col gap-1 p-2">
              <Button type="button" size="sm" variant="ghost" className={menuItem} disabled={queued.length === 0} onClick={() => setConfirm("cancelQueued")}>
                {t("queue.cancelQueued")}
              </Button>
              <Button type="button" size="sm" variant="ghost" className={menuItem} disabled={failed.length === 0} onClick={() => setConfirm("retryFailed")}>
                {t("queue.retryFailed")}
              </Button>
              <Button type="button" size="sm" variant="ghost" className={menuItem} disabled={failed.length === 0} onClick={() => setConfirm("removeFailed")}>
                {t("library.removeFailed")}
              </Button>
            </PopoverContent>
          </Popover>
        </div>
      </div>

      {!collapsed && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <div role="group" aria-label={t("strip.title")} className="flex gap-1">
            {FILTERS.map((f) => (
              <Button key={f} type="button" size="xs" variant={filter === f ? "default" : "outline"} aria-pressed={filter === f} onClick={() => setFilter(f)}>
                {t(`library.filter.${f}` as MessageKey)}
                {f === "queue" && <span className="ml-1 opacity-70">{cards.length}</span>}
              </Button>
            ))}
          </div>
          <div role="group" aria-label={t("strip.scope")} className="flex gap-1">
            {([false, true] as const).map((all) => (
              <Button
                key={String(all)}
                type="button"
                size="xs"
                variant={allProjects === all ? "default" : "outline"}
                aria-pressed={allProjects === all}
                onClick={() => useUiStore.getState().setPanelOpen("strip.allProjects", all)}
              >
                {all ? t("strip.scopeAll") : t("strip.scopeThis")}
              </Button>
            ))}
          </div>
          <div className="relative w-44">
            <Search aria-hidden className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              type="search"
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder={t("gallery.search")}
              aria-label={t("gallery.search")}
              className="h-7 pl-7 text-xs"
            />
          </div>
          <label className="flex items-center gap-1.5 text-xs">
            <Switch checked={favoritesOnly} onCheckedChange={setFavoritesOnly} />
            <Star aria-hidden className="size-3.5 text-amber-400" />
            {t("gallery.favoritesOnly")}
          </label>

          {selectedImage && params && (
            <div className="flex min-w-0 flex-1 flex-wrap items-center justify-end gap-1.5">
              <span className="min-w-0 max-w-[20rem] flex-1 truncate text-xs text-muted-foreground" title={params.prompt}>
                {t("strip.selected")}: {params.prompt}
              </span>
              <Button type="button" size="xs" onClick={() => actions.loadSettings(selectedImage, params)}>
                {t("strip.loadSettings")}
              </Button>
              <Popover>
                <PopoverTrigger render={<Button type="button" size="xs" variant="outline" />}>
                  {t("strip.promptOnly")}
                  <ChevronDown />
                </PopoverTrigger>
                <PopoverContent align="end" className="flex w-40 flex-col gap-1 p-2">
                  <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => actions.loadPrompt(params, "replace")}>
                    {t("strip.promptReplace")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => actions.loadPrompt(params, "append")}>
                    {t("strip.promptAppend")}
                  </Button>
                </PopoverContent>
              </Popover>
              <Popover>
                <PopoverTrigger render={<Button type="button" size="xs" variant="outline" />}>
                  {t("strip.base")}
                  <ChevronDown />
                </PopoverTrigger>
                <PopoverContent align="end" className="flex w-56 flex-col gap-1 p-2">
                  <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => void actions.setAsImg2img(selectedImage)}>
                    {t("gallery.useAsImg2img")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => void actions.setAsReference(selectedImage)}>
                    {t("gallery.useAsReference")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => void actions.setAsInpaint(selectedImage)}>
                    {t("gallery.useAsInpaint")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => void actions.setAsQwenReference(selectedImage)}>
                    {t("gallery.useAsQwenReference")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => void actions.setAsFaceSwapTarget(selectedImage)}>
                    {t("gallery.useAsFaceSwapTarget")}
                  </Button>
                  <Button type="button" size="sm" variant="ghost" className={menuItem} onClick={() => void actions.setAsFaceSwapFace(selectedImage)}>
                    {t("gallery.useAsFaceSwapFace")}
                  </Button>
                </PopoverContent>
              </Popover>
              <Button
                type="button"
                size="xs"
                variant="outline"
                onClick={() => void run(() => enqueue({ ...params, seedRandomize: true }, 1), t("queue.again.done"))}
              >
                {t("queue.again")}
              </Button>
              <Button
                type="button"
                size="icon-xs"
                variant="outline"
                aria-pressed={selectedImage.isFavorite}
                aria-label={selectedImage.isFavorite ? t("gallery.unfavorite") : t("gallery.favorite")}
                title={selectedImage.isFavorite ? t("gallery.unfavorite") : t("gallery.favorite")}
                onClick={() => void toggleFavorite(selectedImage)}
              >
                <Star className={selectedImage.isFavorite ? "fill-amber-400 text-amber-400" : undefined} />
              </Button>
              <Button type="button" size="xs" variant="outline" onClick={() => setDetailId(selectedImage.id)}>
                {t("strip.details")}
              </Button>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={t("strip.deselect")} title={t("strip.deselect")} onClick={() => select(null)}>
                <X />
              </Button>
            </div>
          )}

          {selectedJob && (
            <JobActions
              job={selectedJob}
              queued={queued}
              onClear={() => selectJob(null)}
              onDetails={() => setJobDetailId(selectedJob.id)}
              run={run}
              ops={{ cancel, reorder, retry, remove, enqueue }}
            />
          )}
        </div>
      )}

      {!collapsed &&
        (nothing ? (
          <p className="flex h-28 items-center justify-center text-xs text-muted-foreground">
            {filtered ? t("strip.noResults") : t("strip.empty")}
          </p>
        ) : (
          // 選択の枠は要素の外側に描くため、四辺に余白を取る(スクロール領域は、はみ出しを切り取る)
          <div ref={scrollRef} onScroll={onScroll} onKeyDown={onKeyDown} className="-mx-1 flex scroll-px-1 gap-2 overflow-x-auto p-1" data-testid="strip-scroll">
            {showCards &&
              cards.map((job) => (
                <QueueCard
                  key={job.id}
                  job={job}
                  waitingIndex={queued.indexOf(job) + 1}
                  selected={job.id === selectedJobId}
                  onSelect={() => toggleSelectJob(job.id)}
                  onOpen={(el) => {
                    lastFocused.current = el;
                    setJobDetailId(job.id);
                  }}
                />
              ))}
            {showCards && showImages && cards.length > 0 && items.length > 0 && (
              <div aria-hidden className="mx-1 w-px shrink-0 self-stretch bg-border" />
            )}
            {showImages &&
              items.map((img) => (
                <Thumb
                  key={img.id}
                  image={img}
                  selected={img.id === selectedId}
                  onSelect={() => toggleSelect(img.id)}
                  onOpen={(el) => {
                    lastFocused.current = el;
                    setDetailId(img.id);
                  }}
                />
              ))}
            {showImages && loading && <span className="flex w-16 shrink-0 items-center text-xs text-muted-foreground">{t("strip.loading")}</span>}
          </div>
        ))}

      <ImageDetailSheet
        id={detailId}
        stay
        onClose={() => {
          setDetailId(null);
          // 閉じたら帯のサムネイルへフォーカスを戻す
          setTimeout(() => lastFocused.current?.focus(), 0);
        }}
      />
      <JobDetailSheet
        job={jobDetail}
        onClose={() => {
          setJobDetailId(null);
          setTimeout(() => lastFocused.current?.focus(), 0);
        }}
      />
      {ct && (
        <ConfirmDialog
          open
          title={t(ct.title)}
          description={t(ct.desc)}
          confirmLabel={t(ct.ok)}
          cancelLabel={t("common.cancel")}
          destructive={confirm !== "retryFailed"}
          onOpenChange={(o) => !o && setConfirm(null)}
          onConfirm={() => void onConfirm()}
        />
      )}
    </section>
  );
}

/** 選択中のキューのカードの操作(docs/28 §3.3)。 */
function JobActions({
  job,
  queued,
  onClear,
  onDetails,
  run,
  ops,
}: {
  job: Job;
  queued: Job[];
  onClear: () => void;
  onDetails: () => void;
  run: (action: () => Promise<unknown>, success?: string) => Promise<void>;
  ops: {
    cancel: (id: string) => Promise<void>;
    reorder: (id: string, pos: number) => Promise<void>;
    retry: (id: string) => Promise<Job>;
    remove: (ids: string[]) => Promise<number>;
    enqueue: (params: Job["params"], count: number) => Promise<Job[]>;
  };
}) {
  const t = useT();
  const summary = job.params.faceSwap?.enabled ? t("gallery.params.faceSwap") : job.params.prompt;
  const idx = queued.findIndex((j) => j.id === job.id);
  const last = queued.length - 1;
  const load = () => useFormStore.getState().loadParams({ ...job.params, seedRandomize: false });
  const again = () => run(() => ops.enqueue({ ...job.params, seedRandomize: true }, 1), t("queue.again.done"));

  return (
    <div className="flex min-w-0 flex-1 flex-wrap items-center justify-end gap-1.5">
      <span className="min-w-0 max-w-[20rem] flex-1 truncate text-xs text-muted-foreground" title={summary}>
        {t("strip.selected")}: {summary}
      </span>
      {job.status === "queued" && (
        <>
          <Button type="button" size="xs" variant="outline" disabled={idx <= 0} onClick={() => void run(() => ops.reorder(job.id, 0))}>
            {t("queue.moveTop")}
          </Button>
          <Button type="button" size="xs" variant="outline" disabled={idx <= 0} onClick={() => void run(() => ops.reorder(job.id, idx - 1))}>
            ◀ {t("library.prev")}
          </Button>
          <Button type="button" size="xs" variant="outline" disabled={idx < 0 || idx >= last} onClick={() => void run(() => ops.reorder(job.id, idx + 1))}>
            {t("library.next")} ▶
          </Button>
          <Button type="button" size="xs" variant="outline" disabled={idx < 0 || idx >= last} onClick={() => void run(() => ops.reorder(job.id, last))}>
            {t("queue.moveBottom")}
          </Button>
        </>
      )}
      {(job.status === "queued" || job.status === "running") && (
        <Button type="button" size="xs" variant="outline" onClick={() => void run(() => ops.cancel(job.id))}>
          {t("queue.cancel")}
        </Button>
      )}
      {(job.status === "failed" || job.status === "cancelled") && (
        <>
          <Button type="button" size="xs" onClick={() => void run(() => ops.retry(job.id), t("queue.retried"))}>
            {t("queue.retry")}
          </Button>
          <Button type="button" size="xs" variant="outline" onClick={() => void again()}>
            {t("queue.again")}
          </Button>
        </>
      )}
      {job.status !== "running" && (
        <Button type="button" size="xs" variant="outline" onClick={load}>
          {t("queue.load")}
        </Button>
      )}
      {(job.status === "failed" || job.status === "cancelled") && (
        <Button
          type="button"
          size="xs"
          variant="outline"
          onClick={() =>
            void run(async () => {
              await ops.remove([job.id]);
              onClear();
            })
          }
        >
          {t("queue.remove")}
        </Button>
      )}
      <Button type="button" size="xs" variant="outline" onClick={onDetails}>
        {t("strip.details")}
      </Button>
      <Button type="button" size="icon-xs" variant="ghost" aria-label={t("strip.deselect")} title={t("strip.deselect")} onClick={onClear}>
        <X />
      </Button>
    </div>
  );
}

function Thumb({
  image,
  selected,
  onSelect,
  onOpen,
}: {
  image: ImageItem;
  selected: boolean;
  onSelect: () => void;
  onOpen: (el: HTMLElement) => void;
}) {
  const [tip, setTip] = useState(`${image.width}×${image.height} · seed ${image.seed}`);
  const [label, setLabel] = useState(`seed ${image.seed}`);

  function loadTip() {
    void getDetail(image.id).then((d) => {
      if (!d) return;
      const p = d.job.params;
      const head = p.prompt.length > 120 ? `${p.prompt.slice(0, 120)}…` : p.prompt;
      setTip(`${head}\n${p.model.name} · ${image.width}×${image.height}`);
      setLabel(head);
    });
  }

  return (
    <button
      type="button"
      data-image-id={image.id}
      title={tip}
      aria-label={label}
      aria-pressed={selected}
      onMouseEnter={loadTip}
      onFocus={loadTip}
      onClick={onSelect}
      onDoubleClick={(e) => onOpen(e.currentTarget)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onOpen(e.currentTarget);
        }
      }}
      className={cn(
        "relative size-28 shrink-0 overflow-hidden rounded-md border bg-muted/30 outline-none transition focus-visible:ring-2 focus-visible:ring-ring",
        selected && "ring-2 ring-primary",
      )}
    >
      <img src={imageSrc(image.thumbPath)} alt="" className="size-full object-cover" loading="lazy" draggable={false} />
      {image.isFavorite && <Star aria-hidden className="absolute right-1 top-1 size-3.5 fill-amber-400 text-amber-400" />}
    </button>
  );
}
