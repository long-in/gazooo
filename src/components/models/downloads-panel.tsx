"use client";

import { useMemo } from "react";

import { isActiveStatus, localized } from "@/lib/models/catalog";
import { useLanguage, useT } from "@/lib/i18n/use-t";
import { useModelsStore } from "@/stores/models-store";

import { ActiveDownload } from "./family-card";

/**
 * 取得中・待機中のダウンロードの一覧。タブを切り替えても進捗を見失わないよう、ページ上部に置く。
 * 取得中が無ければ表示しない。
 */
export function DownloadsPanel() {
  const t = useT();
  const lang = useLanguage();
  const downloads = useModelsStore((s) => s.downloads);
  const view = useModelsStore((s) => s.view);

  const active = useMemo(() => downloads.filter((d) => isActiveStatus(d.status)), [downloads]);
  if (active.length === 0) return null;

  const nameOf = (familyId: string, variantId: string) => {
    const family = view?.catalog.families.find((f) => f.id === familyId);
    const variant = family?.variants.find((v) => v.id === variantId);
    return family && variant ? `${localized(family.name, lang)} — ${localized(variant.label, lang)}` : `${familyId}/${variantId}`;
  };

  return (
    <section className="rounded-lg border p-3" aria-labelledby="models-downloads-title">
      <h2 id="models-downloads-title" className="text-sm font-medium">
        {t("models.downloads.title")}
      </h2>
      <ul className="mt-2 flex flex-col gap-3">
        {active.map((d) => (
          <li key={d.downloadId} className="flex flex-col gap-1">
            <p className="truncate text-sm">{nameOf(d.familyId, d.variantId)}</p>
            <ActiveDownload dl={d} />
          </li>
        ))}
      </ul>
    </section>
  );
}
