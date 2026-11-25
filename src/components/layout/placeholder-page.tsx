"use client";

import { type MessageKey } from "@/lib/i18n";
import { useT } from "@/lib/i18n/use-t";

export function PlaceholderPage({ titleKey }: { titleKey: MessageKey }) {
  const t = useT();
  return (
    <div className="p-8">
      <h1 className="text-2xl font-semibold">{t(titleKey)}</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        {t("placeholder.comingSoon")}
      </p>
    </div>
  );
}
