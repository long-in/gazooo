"use client";

import { ExternalLink } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { localized } from "@/lib/models/catalog";
import { useT } from "@/lib/i18n/use-t";
import { type ModelFamily, type ModelVariant, openExternal } from "@/lib/tauri/models";
import type { Language } from "@/lib/tauri/settings";

/** ライセンスの確認(docs/12 §7)。同意したモデルだけをダウンロードする。 */
export function ConsentDialog({
  target,
  lang,
  onCancel,
  onAgree,
}: {
  target: { family: ModelFamily; variant: ModelVariant } | null;
  lang: Language;
  onCancel: () => void;
  onAgree: () => void;
}) {
  const t = useT();
  return (
    <Dialog open={target !== null} onOpenChange={(open) => !open && onCancel()}>
      <DialogContent>
        {target && (
          <>
            <DialogHeader>
              <DialogTitle>{t("models.consent.title")}</DialogTitle>
              <DialogDescription>{t("models.consent.body")}</DialogDescription>
            </DialogHeader>
            <div className="flex flex-col gap-1 rounded-md bg-muted p-3 text-sm">
              <p className="font-medium">
                {localized(target.family.name, lang)} — {localized(target.variant.label, lang)}
              </p>
              <p className="text-muted-foreground">
                {t("models.license")}: {target.variant.license.name}
              </p>
              <div>
                <Button variant="link" size="sm" className="h-auto p-0" onClick={() => void openExternal(target.variant.license.url)}>
                  <ExternalLink className="size-3.5" />
                  {t("models.consent.link")}
                </Button>
              </div>
            </div>
            {target.family.profile && ["loose", "moderate"].includes(target.family.profile.moderation.level) && (
              <div role="note" className="rounded-md border p-3 text-sm">
                <p className="font-medium">{t("models.consent.moderation.title")}</p>
                <p className="mt-1 text-muted-foreground">{t("models.consent.moderation.body")}</p>
              </div>
            )}
            <DialogFooter>
              <Button variant="outline" onClick={onCancel}>
                {t("models.cancel")}
              </Button>
              <Button onClick={onAgree}>{t("models.consent.agree")}</Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
