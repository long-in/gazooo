"use client";

import Link from "next/link";

import { useT } from "@/lib/i18n/use-t";
import type { BaseModel, ModelCategory } from "@/lib/tauri/models";

/** 「モデルがありません」の案内から、モデル取得ページ(該当カテゴリ・ベースモデル)へ移る(docs/12 §5)。 */
export function ModelsLink({ category, kind }: { category: ModelCategory; kind?: string }) {
  const t = useT();
  const params = new URLSearchParams({ category });
  // 生成モデルの種別が SD 1.5 / SDXL のときは、合うバージョンに絞る
  if (kind === "sd15" || kind === "sdxl") params.set("base", kind satisfies BaseModel);
  return (
    <Link href={`/models?${params.toString()}`} className="text-xs font-medium underline underline-offset-2">
      {t("models.link.open")}
    </Link>
  );
}
