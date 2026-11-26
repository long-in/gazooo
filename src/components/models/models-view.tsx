"use client";

import { FolderOpen } from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { RestartEngineButton } from "@/components/engine/restart-engine-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  type BaseFilter,
  type BundleItem,
  bundleFor,
  CATEGORIES,
  formatBytes,
  installedMap,
  isBase,
  pickVariant,
  PURPOSES,
  type PurposeFilter,
  isCategory,
  isPurpose,
  variantKey,
  visibleFamilies,
} from "@/lib/models/catalog";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useLanguage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { type ModelFamily, type ModelVariant, pickModelsDir } from "@/lib/tauri/models";
import { useEngineProcessStore } from "@/stores/engine-process-store";
import { useModelsStore } from "@/stores/models-store";

import { ConsentDialog } from "./consent-dialog";
import { DownloadsPanel } from "./downloads-panel";
import { FamilyCard } from "./family-card";

const BASE_FILTERS: BaseFilter[] = ["all", "sd15", "sdxl"];

/** モデル取得ページ(docs/12-model-downloader.md §5)。 */
export function ModelsView() {
  const t = useT();
  const lang = useLanguage();
  const errorMessage = useErrorMessage();
  const params = useSearchParams();

  const view = useModelsStore((s) => s.view);
  const loading = useModelsStore((s) => s.loading);
  const error = useModelsStore((s) => s.error);
  const downloads = useModelsStore((s) => s.downloads);
  const category = useModelsStore((s) => s.category);
  const base = useModelsStore((s) => s.base);
  const purpose = useModelsStore((s) => s.purpose);
  const chosen = useModelsStore((s) => s.chosen);

  // 同意が必要で、確認待ちのモデル
  const [consent, setConsent] = useState<{ family: ModelFamily; variant: ModelVariant } | null>(null);
  // 「まとめて取得」で、同意の確認のあとに続けて取得する残り
  const [queue, setQueue] = useState<BundleItem[]>([]);

  // 表示のたびにカタログと導入状況を取り直す(取得イベントの購読はアプリ全体で行う。providers.tsx)
  useEffect(() => {
    void useModelsStore.getState().load();
    void useModelsStore.getState().loadDownloads();
    void useModelsStore.getState().loadEngineDir();
  }, []);

  // 他の画面からの遷移(?category=&base=)を反映する
  const qCategory = params.get("category");
  const qBase = params.get("base");
  const qPurpose = params.get("purpose");
  const qFamily = params.get("family");
  useEffect(() => {
    const st = useModelsStore.getState();
    if (isCategory(qCategory)) st.setCategory(qCategory);
    if (isBase(qBase)) st.setBase(qBase);
    if (isPurpose(qPurpose)) st.setPurpose(qPurpose);
  }, [qCategory, qBase, qPurpose]);

  // AI のモデル提案から開いたとき(?family=)は、そのファミリーのタブへ移り、絞り込みを外して、カードへ移動する
  // (同じ ?family= では 1 回だけ。あとからユーザーがタブや絞り込みを変えても、戻さない)
  const movedTo = useRef<string | null>(null);
  useEffect(() => {
    const family = view?.catalog.families.find((f) => f.id === qFamily);
    if (!family || movedTo.current === family.id) return;
    movedTo.current = family.id;
    const st = useModelsStore.getState();
    st.setCategory(family.category);
    st.setBase("all");
    st.setPurpose("all");
    const timer = setTimeout(() => document.getElementById(`fam-${family.id}`)?.scrollIntoView({ block: "start" }), 50);
    return () => clearTimeout(timer);
  }, [view, qFamily]);

  const installed = useMemo(() => installedMap(view), [view]);
  const families = useMemo(
    () => visibleFamilies(view?.catalog.families ?? [], category, base, purpose),
    [view, category, base, purpose],
  );
  const dirKnown = view?.modelsDir != null;
  const engineDir = useModelsStore((s) => s.engineDir);
  // 起動管理が有効で ComfyUI が稼働中なら、反映のあとにその場で再起動できる(docs/15 §9)
  const managedRunning = useEngineProcessStore((s) => s.view?.config.enabled === true && s.state?.status === "running");

  /** 取得を始める。同意が要るときは確認ダイアログを出し(`needs-consent`)、失敗したら理由を表示する(`failed`)。 */
  async function startDownload(
    family: ModelFamily,
    variant: ModelVariant,
    agreed = false,
  ): Promise<"started" | "needs-consent" | "failed"> {
    try {
      await useModelsStore.getState().start(family.id, variant.id, agreed);
      setConsent(null);
      return "started";
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "MODELS_NO_CONSENT") {
        setConsent({ family, variant });
        return "needs-consent";
      }
      setConsent(null);
      setQueue([]);
      toast.error(errorMessage(err));
      return "failed";
    }
  }

  /**
   * 必要なファイルをまとめて取得する(docs/17 §5)。導入済みは飛ばして順に始める。同意が要るものがあれば、
   * そこで確認ダイアログを出し、同意したあと残りを続ける(取得そのものは、バックエンドが 1 件ずつ実行する)。
   */
  async function startAll(items: BundleItem[]) {
    const todo = items.filter((i) => !i.installed);
    for (let i = 0; i < todo.length; i++) {
      const result = await startDownload(todo[i].family, todo[i].variant);
      if (result === "needs-consent") setQueue(todo.slice(i + 1)); // 残りは、同意のあとに続ける
      if (result !== "started") return;
    }
  }

  async function agreeAndContinue() {
    if (!consent) return;
    const result = await startDownload(consent.family, consent.variant, true);
    if (result !== "started") return;
    const rest = queue;
    setQueue([]);
    if (rest.length > 0) await startAll(rest);
  }

  async function changeDir(path: string | null) {
    try {
      await useModelsStore.getState().setDir(path);
      toast.success(t("models.dir.changed"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  async function applyToEngine(clear = false) {
    try {
      await useModelsStore.getState().applyToEngine(clear);
      toast.success(t(clear ? "models.engine.cleared" : "models.engine.applied"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  async function pickDir() {
    try {
      const picked = await pickModelsDir();
      if (picked) await changeDir(picked);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-4 p-6">
      <header>
        <h1 className="sr-only">{t("models.title")}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{t("models.desc")}</p>
        <p className="mt-1 text-xs text-muted-foreground">{t("models.net.notice")}</p>
      </header>

      {/* 保存先 */}
      <section className="rounded-lg border p-3" aria-labelledby="models-dir-title">
        <h2 id="models-dir-title" className="text-sm font-medium">
          {t("models.dir.label")}
        </h2>
        {dirKnown ? (
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1 text-xs" title={view?.modelsDir ?? ""}>
              {view?.modelsDir}
            </code>
            <Badge variant="outline">
              {t(`models.dir.source.${view?.modelsDirSource === "setting" ? "setting" : "detected"}` as MessageKey)}
            </Badge>
            {view?.freeBytes != null && (
              <span className="text-xs text-muted-foreground">
                {t("models.dir.free")}: {formatBytes(view.freeBytes)}
              </span>
            )}
            <Button variant="outline" size="sm" onClick={() => void pickDir()}>
              <FolderOpen className="size-3.5" />
              {t("models.dir.change")}
            </Button>
            <Link href="/settings?scope=global&tab=storage" className="text-xs font-medium underline underline-offset-2">
              {t("settings.storage.migrate")}
            </Link>
            {view?.modelsDirSource === "setting" && (
              <Button variant="ghost" size="sm" onClick={() => void changeDir(null)}>
                {t("models.dir.auto")}
              </Button>
            )}
          </div>
        ) : (
          view && (
            <div
              role="alert"
              className="mt-2 flex flex-col gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm"
            >
              <p className="font-medium">{t("models.dir.unknown.title")}</p>
              <p className="text-xs">{t("models.dir.unknown.body")}</p>
              <div>
                <Button size="sm" onClick={() => void pickDir()}>
                  <FolderOpen className="size-3.5" />
                  {t("models.dir.change")}
                </Button>
              </div>
            </div>
          )
        )}
        {dirKnown && engineDir && (engineDir.mismatch || engineDir.applied || engineDir.hasCustomConfig) && (
          <div className="mt-3 flex flex-col gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
            {engineDir.applied ? (
              <>
                <p className="text-xs">{t("models.engine.applied.note")}</p>
                <div className="flex flex-wrap gap-2">
                  <Button variant="outline" size="sm" onClick={() => void applyToEngine(true)}>
                    {t("models.engine.clear")}
                  </Button>
                  {managedRunning && <RestartEngineButton variant="default" label={t("engine.proc.restartNow")} />}
                </div>
              </>
            ) : engineDir.mismatch ? (
              <>
                <p className="font-medium">{t("models.engine.mismatch.title")}</p>
                <p className="text-xs">{t("models.engine.mismatch.body")}</p>
                {engineDir.engineModelsDir && (
                  <p className="text-xs text-muted-foreground">
                    {t("models.engine.refers")}: <code>{engineDir.engineModelsDir}</code>
                  </p>
                )}
                {engineDir.canApply ? (
                  <div>
                    <Button size="sm" onClick={() => void applyToEngine()}>
                      {t("models.engine.apply")}
                    </Button>
                  </div>
                ) : (
                  <p className="text-xs">
                    {t(`models.engine.blocked.${engineDir.reason ?? "engine_unknown"}` as MessageKey)}
                  </p>
                )}
              </>
            ) : (
              <p className="text-xs">{t("models.engine.custom")}</p>
            )}
          </div>
        )}
      </section>

      <DownloadsPanel />

      {/* カテゴリとベースモデル */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div role="tablist" aria-label={t("models.title")} className="flex flex-wrap gap-1 rounded-lg bg-muted p-1">
          {CATEGORIES.map((c) => (
            <button
              key={c}
              role="tab"
              type="button"
              aria-selected={category === c}
              onClick={() => useModelsStore.getState().setCategory(c)}
              className={
                "rounded-md px-3 py-1 text-sm transition-colors " +
                (category === c ? "bg-background font-medium shadow-sm" : "text-muted-foreground hover:text-foreground")
              }
            >
              {t(`models.tab.${c}` as MessageKey)}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-3">
          {category === "checkpoint" && (
            <div className="flex items-center gap-2">
              <label htmlFor="models-purpose" className="text-sm text-muted-foreground">
                {t("models.filter.purpose")}
              </label>
              <Select
                value={purpose}
                items={PURPOSES.map((p) => ({ value: p, label: t(`models.purpose.${p}` as MessageKey) }))}
                onValueChange={(v) => v !== null && useModelsStore.getState().setPurpose(v as PurposeFilter)}
              >
                <SelectTrigger id="models-purpose" className="w-44">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {PURPOSES.map((p) => (
                    <SelectItem key={p} value={p}>
                      {t(`models.purpose.${p}` as MessageKey)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          <div className="flex items-center gap-2">
            <label htmlFor="models-base" className="text-sm text-muted-foreground">
              {t("models.filter.base")}
            </label>
            <Select
              value={base}
              items={BASE_FILTERS.map((b) => ({
                value: b,
                label: t((b === "all" ? "models.filter.all" : `models.base.${b}`) as MessageKey),
              }))}
              onValueChange={(v) => v !== null && useModelsStore.getState().setBase(v as BaseFilter)}
            >
              <SelectTrigger id="models-base" className="w-36">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {BASE_FILTERS.map((b) => (
                  <SelectItem key={b} value={b}>
                    {t((b === "all" ? "models.filter.all" : `models.base.${b}`) as MessageKey)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </div>

      {loading && !view && <p className="text-sm text-muted-foreground">{t("models.loading")}</p>}
      {error && !view && (
        <p role="alert" className="text-sm text-destructive">
          {t("models.loadFailed")}: {errorMessage(error)}
        </p>
      )}
      {view && families.length === 0 && <p className="text-sm text-muted-foreground">{t("models.empty")}</p>}

      <div className="flex flex-col gap-3">
        {families.map(({ family, variants }) => {
          const selected = pickVariant(variants, chosen[family.id]);
          const bundle = bundleFor(
            view?.catalog.families ?? [],
            family,
            selected,
            (fid, vid) => installed.get(variantKey(fid, vid))?.installed === true,
          );
          return (
          <FamilyCard
            key={family.id}
            family={family}
            variants={variants}
            bundle={bundle}
            onDownloadAll={() => void startAll(bundle)}
            lang={lang}
            chosenId={chosen[family.id]}
            installed={(v) => installed.get(variantKey(family.id, v.id))}
            downloads={downloads}
            canDownload={dirKnown}
            onChoose={(variantId) => useModelsStore.getState().choose(family.id, variantId)}
            onDownload={(variant) => void startDownload(family, variant)}
          />
          );
        })}
      </div>

      <ConsentDialog
        target={consent}
        lang={lang}
        onCancel={() => {
          setConsent(null);
          setQueue([]);
        }}
        onAgree={() => void agreeAndContinue()}
      />
    </div>
  );
}
