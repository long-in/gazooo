"use client";

import { MoreHorizontal, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { imageCountNonFavorite } from "@/lib/tauri/images";
import { useGalleryStore } from "@/stores/gallery-store";

type Counts = { total: number; favorites: number };

/**
 * ツールバーの [⋯] メニューと「お気に入り以外を削除」の確認ダイアログ(docs/49)。
 * 件数は開くときに Rust から取得し、実行時に増えていたら `STALE_COUNT` で何も消えない。
 */
export function DeleteNonFavorites({
  scopeName,
  searching,
  favoritesScope,
}: {
  /** 確認ダイアログに出す表示範囲の名前。 */
  scopeName: string;
  /** 検索語が入力されている(Q-420 により無効)。 */
  searching: boolean;
  /** お気に入りのスコープ(対象が常に 0 のため無効)。 */
  favoritesScope: boolean;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [menuOpen, setMenuOpen] = useState(false);
  const [counts, setCounts] = useState<Counts | null>(null);
  const [busy, setBusy] = useState(false);

  const disabledReason = searching
    ? t("gallery.deleteNonFavDisabledSearch")
    : favoritesScope
      ? t("gallery.deleteNonFavDisabledFav")
      : null;

  async function openConfirm() {
    setMenuOpen(false);
    try {
      const c = await imageCountNonFavorite(useGalleryStore.getState().scope);
      if (c.total === 0) {
        toast.info(t("gallery.deleteNonFavDisabledNone"));
        return;
      }
      setCounts(c);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  async function run() {
    if (!counts) return;
    setBusy(true);
    try {
      const r = await useGalleryStore.getState().removeNonFavorites(counts.total);
      setCounts(null);
      const msg = r.failed > 0 ? t("gallery.deleteNonFavPartial") : t("gallery.deleteNonFavDone");
      toast.success(msg.replace("{count}", String(r.deleted)).replace("{failed}", String(r.failed)));
    } catch (e) {
      const err = toAppError(e);
      toast.error(errorMessage(err));
      if (err.code === "STALE_COUNT") setCounts(null); // 開き直して再確認してもらう
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Popover open={menuOpen} onOpenChange={setMenuOpen}>
        <PopoverTrigger
          render={
            <Button type="button" variant="ghost" size="icon-sm" aria-label={t("gallery.moreActions")} title={t("gallery.moreActions")} />
          }
        >
          <MoreHorizontal />
        </PopoverTrigger>
        <PopoverContent align="end" className="flex w-60 flex-col gap-1 p-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="justify-start text-destructive"
            disabled={disabledReason !== null}
            onClick={() => void openConfirm()}
          >
            <Trash2 />
            {t("gallery.deleteNonFav")}
          </Button>
          {disabledReason && <p className="px-2 text-xs text-muted-foreground">{disabledReason}</p>}
        </PopoverContent>
      </Popover>

      <AlertDialog open={counts !== null} onOpenChange={(v) => !v && !busy && setCounts(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("gallery.deleteNonFavTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("gallery.deleteNonFavDesc")
                .replace("{scope}", scopeName)
                .replace("{count}", String(counts?.total ?? 0))
                .replace("{favorites}", String(counts?.favorites ?? 0))}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            {/* DOM 順の先頭 = 既定フォーカスはキャンセル(docs/49 Q-422・37 §10.5 Q-434) */}
            <AlertDialogCancel disabled={busy}>{t("gallery.deleteCancel")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" disabled={busy} onClick={() => void run()}>
              {t("gallery.deleteNonFavConfirm").replace("{count}", String(counts?.total ?? 0))}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
