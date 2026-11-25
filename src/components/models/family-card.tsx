"use client";

import { CheckCircle2, ExternalLink, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ModelProfileView } from "@/components/models/model-profile-view";
import { Progress } from "@/components/ui/progress";
import { type BundleItem, bundleMissingBytes, downloadFor, etaSeconds, formatBytes, formatDuration, formatSpeed, isActiveStatus, localized, pickVariant, progressPercent } from "@/lib/models/catalog";
import type { MessageKey } from "@/lib/i18n";
import { familyKinds, featuresFor } from "@/lib/models/profile";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { type DownloadState, type ModelFamily, type ModelVariant, openExternal } from "@/lib/tauri/models";
import type { Language } from "@/lib/tauri/settings";
import { useModelsStore } from "@/stores/models-store";

const USED_BY_KEY: Record<string, MessageKey> = {
  generate: "models.usedBy.generate",
  controlnet: "models.usedBy.controlnet",
  faceDetailer: "models.usedBy.faceDetailer",
};

/** ファミリー(モデルの種類)ごとのカード。バージョン(variant)を選んでダウンロードする。 */
export function FamilyCard({
  family,
  variants,
  bundle = [],
  onDownloadAll,
  lang,
  chosenId,
  installed,
  downloads,
  canDownload,
  onChoose,
  onDownload,
}: {
  family: ModelFamily;
  /** 選択中の variant と、一緒に必要なファイル(テキストエンコーダ・VAE)。無ければ空(docs/17 §5)。 */
  bundle?: BundleItem[];
  /** 必要なファイルをまとめて取得する。 */
  onDownloadAll?: () => void;
  /** 絞り込み後に表示する variant。 */
  variants: ModelVariant[];
  lang: Language;
  chosenId: string | undefined;
  installed: (v: ModelVariant) => { installed: boolean; sizeMatches: boolean } | undefined;
  downloads: DownloadState[];
  canDownload: boolean;
  onChoose: (variantId: string) => void;
  onDownload: (variant: ModelVariant) => void;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [verifying, setVerifying] = useState(false);

  const selected = pickVariant(variants, chosenId);
  const inst = installed(selected);
  const dl = downloadFor(downloads, family.id, selected.id);
  const active = dl !== undefined && isActiveStatus(dl.status);
  const hasBase = variants.some((v) => v.baseModel !== undefined);

  async function verify() {
    setVerifying(true);
    try {
      const ok = await useModelsStore.getState().verify(family.id, selected.id);
      if (ok) toast.success(t("models.verify.ok"));
      else toast.error(t("models.verify.ng"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    } finally {
      setVerifying(false);
    }
  }

  const groupName = `variant-${family.id}`;

  return (
    <section className="rounded-lg border p-4" aria-labelledby={`fam-${family.id}`}>
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <h2 id={`fam-${family.id}`} className="text-base font-medium">
            {localized(family.name, lang)}
          </h2>
          {family.tags.length > 0 && (
            <div className="mt-1 flex flex-wrap gap-1">
              {family.tags.map((tag) => (
                <Badge key={tag} variant="secondary">
                  {t(`models.tag.${tag}` as MessageKey)}
                </Badge>
              ))}
            </div>
          )}
          <p className="mt-0.5 text-sm text-muted-foreground">{localized(family.description, lang)}</p>
        </div>
        {family.usedBy.length > 0 && (
          <p className="text-xs text-muted-foreground">
            {t("models.usedBy")}: {family.usedBy.map((u) => (USED_BY_KEY[u] ? t(USED_BY_KEY[u]) : u)).join(" / ")}
          </p>
        )}
      </div>

      {family.profile && (
        <details className="mt-3 rounded-md border px-3 py-2">
          <summary className="cursor-pointer text-sm font-medium">{t("modelProfile.title")}</summary>
          <div className="mt-2">
            <ModelProfileView
              profile={family.profile}
              lang={lang}
              features={familyKinds(family)[0] ? featuresFor(familyKinds(family)[0]) : undefined}
            />
          </div>
        </details>
      )}

      <fieldset className="mt-3">
        <legend className="mb-1 text-xs font-medium text-muted-foreground">{t("models.version")}</legend>
        <div className="flex flex-col gap-1">
          {variants.map((v) => {
            const i = installed(v);
            const id = `${groupName}-${v.id}`;
            return (
              <label
                key={v.id}
                htmlFor={id}
                className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-muted has-checked:bg-muted"
              >
                <input
                  id={id}
                  type="radio"
                  name={groupName}
                  className="size-4 accent-primary"
                  checked={selected.id === v.id}
                  onChange={() => onChoose(v.id)}
                />
                <span className="min-w-0 flex-1">
                  {localized(v.label, lang)}
                  <span className="ml-2 text-xs text-muted-foreground">
                    {formatBytes(v.sizeBytes)}
                    {v.precision ? ` · ${v.precision}` : ""}
                    {hasBase ? ` · ${t(v.baseModel ? (`models.base.${v.baseModel}` as MessageKey) : "models.base.none")}` : ""}
                  </span>
                </span>
                {v.recommended && <Badge variant="secondary">{t("models.recommended")}</Badge>}
                {i?.installed && (
                  <Badge variant="outline" className="gap-1">
                    <CheckCircle2 />
                    {t("models.installed")}
                  </Badge>
                )}
              </label>
            );
          })}
        </div>
        {family.category === "controlnet" && (
          <p className="mt-1 px-2 text-xs text-muted-foreground">{t("models.base.mismatch")}</p>
        )}
      </fieldset>

      {/* 選択中の variant の詳細 */}
      <div className="mt-3 flex flex-col gap-1 text-xs text-muted-foreground">
        <p>
          {t("models.license")}: {selected.license.name}
          {" — "}
          {selected.license.commercialUse === true
            ? t("models.license.free")
            : selected.license.commercialUse === false
              ? t("models.license.noncommercial")
              : t("models.license.see")}
          <LinkButton url={selected.license.url} label={t("models.license")} />
        </p>
        <p>
          {t("models.source")}: {selected.source.repo}
          <LinkButton url={`https://huggingface.co/${selected.source.repo}`} label={t("models.source")} />
        </p>
        {selected.note && (
          <p className="flex items-start gap-1">
            <ShieldCheck className="mt-0.5 size-3.5 shrink-0" />
            {localized(selected.note, lang)}
          </p>
        )}
        {inst?.installed && !inst.sizeMatches && (
          <p role="alert" className="text-amber-600 dark:text-amber-400">
            {t("models.installed.mismatch")}
          </p>
        )}
      </div>

      {/* 一緒に必要なファイル(Anima・Qwen-Image 2.1・Krea 2)。足りないものをまとめて取得できる */}
      {bundle.length > 1 && (
        <div className="mt-3 rounded-md border p-3 text-sm" role="group" aria-label={t("models.bundle.title")}>
          <p className="text-xs font-medium text-muted-foreground">{t("models.bundle.title")}</p>
          <ul className="mt-1 flex flex-col gap-0.5 text-xs">
            {bundle.map((b) => (
              <li key={`${b.family.id}/${b.variant.id}`} className="flex items-center gap-2">
                <span className="min-w-0 flex-1">
                  {b.family.id === family.id ? t("models.bundle.main") : localized(b.family.name, lang)}
                  <span className="ml-2 text-muted-foreground">{formatBytes(b.variant.sizeBytes)}</span>
                </span>
                {b.installed && (
                  <Badge variant="outline" className="gap-1">
                    <CheckCircle2 />
                    {t("models.installed")}
                  </Badge>
                )}
              </li>
            ))}
          </ul>
          <div className="mt-2">
            {bundle.every((b) => b.installed) ? (
              <p className="text-xs text-muted-foreground">{t("models.bundle.complete")}</p>
            ) : (
              <Button
                size="sm"
                variant="outline"
                disabled={!canDownload}
                title={canDownload ? undefined : t("models.download.disabled")}
                onClick={() => onDownloadAll?.()}
              >
                {t("models.bundle.download")}
                <span className="ml-1 text-xs opacity-80">({formatBytes(bundleMissingBytes(bundle))})</span>
              </Button>
            )}
          </div>
        </div>
      )}

      {/* 操作 */}
      <div className="mt-3 flex flex-col gap-2">
        {active && dl && <ActiveDownload dl={dl} />}
        {dl?.status === "failed" && dl.error && (
          <p role="alert" className="text-sm text-destructive">
            {t(`error.${dl.error.code}` as MessageKey) ?? dl.error.message}
          </p>
        )}
        {dl?.status === "cancelled" && (
          <p className="text-sm text-muted-foreground">{t("models.downloads.cancelled")}</p>
        )}
        <div className="flex flex-wrap items-center gap-2">
          {inst?.installed ? (
            <Button variant="outline" size="sm" disabled={verifying} onClick={() => void verify()}>
              {t("models.verify")}
            </Button>
          ) : (
            !active && (
              <Button
                size="sm"
                disabled={!canDownload}
                title={canDownload ? undefined : t("models.download.disabled")}
                onClick={() => onDownload(selected)}
              >
                {dl?.status === "failed" || dl?.status === "cancelled" ? t("models.retry") : t("models.download")}
                <span className="ml-1 text-xs opacity-80">({formatBytes(selected.sizeBytes)})</span>
              </Button>
            )
          )}
        </div>
      </div>
    </section>
  );
}

function LinkButton({ url, label }: { url: string; label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={url}
      className="ml-1 inline-flex translate-y-0.5 text-muted-foreground hover:text-foreground"
      onClick={() => void openExternal(url)}
    >
      <ExternalLink className="size-3.5" />
    </button>
  );
}

/** 進捗・速度・残り時間と、キャンセル/破棄。 */
export function ActiveDownload({ dl }: { dl: DownloadState }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const percent = progressPercent(dl);
  const eta = etaSeconds(dl);

  async function cancel(discard: boolean) {
    try {
      await useModelsStore.getState().cancel(dl.downloadId, discard);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const label =
    dl.status === "queued" ? t("models.queued") : dl.status === "verifying" ? t("models.verifying") : t("models.downloading");

  return (
    <div className="flex flex-col gap-1" aria-live="off">
      <Progress value={dl.status === "queued" ? null : percent} aria-label={label} />
      <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
        <span>
          {label} {dl.status !== "queued" && `${percent}% · ${formatBytes(dl.receivedBytes)} / ${formatBytes(dl.totalBytes)}`}
          {dl.bytesPerSec ? ` · ${formatSpeed(dl.bytesPerSec)}` : ""}
          {eta !== null ? ` · ${formatDuration(eta)}` : ""}
        </span>
        <span className="flex gap-1">
          <Button variant="outline" size="xs" onClick={() => void cancel(false)}>
            {t("models.cancel")}
          </Button>
          <Button variant="ghost" size="xs" onClick={() => void cancel(true)}>
            {t("models.discard")}
          </Button>
        </span>
      </div>
    </div>
  );
}
