"use client";

import { save } from "@tauri-apps/plugin-dialog";
import { Brush, Copy, Download, FolderOpen, ImagePlus, Sparkles, Star, Tags, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { MoveToProjectMenu } from "@/components/projects/move-to-project";
import { ProjectDot } from "@/components/projects/project-dot";
import { imageSrc } from "@/lib/image-src";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { useImageActions } from "@/lib/gallery/use-image-as";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { durationMs, formatDuration } from "@/lib/queue";
import { toAppError } from "@/lib/tauri/errors";
import { type ImageDetail, imageExport, imageGet, imageReveal } from "@/lib/tauri/images";
import { useGalleryStore } from "@/stores/gallery-store";
import { useProjectStore } from "@/stores/project-store";
import { useStripStore } from "@/stores/strip-store";

import { DeleteConfirm } from "./delete-confirm";

type State = { id: string; status: "loading" } | { id: string; status: "missing" } | { id: string; status: "ready"; detail: ImageDetail };

/** ギャラリーの詳細パネル(`/gallery?id=` で開く)。 */
/** `stay` が true のとき(生成ページ上)は、「〜に使う」で画面を移動しない。 */
export function ImageDetailSheet({ id, onClose, stay = false }: { id: string | null; onClose: () => void; stay?: boolean }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const actions = useImageActions(stay);

  const [state, setState] = useState<State | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const items = useGalleryStore((s) => s.items);
  const stripItems = useStripStore((s) => s.items);
  const projects = useProjectStore((s) => s.projects);
  const setGalleryFavorite = useGalleryStore((s) => s.setFavorite);
  const setFavorite = async (id: string, value: boolean) => {
    await setGalleryFavorite(id, value);
    useStripStore.getState().applyFavorite(id, value);
  };
  const remove = useGalleryStore((s) => s.remove);

  useEffect(() => {
    if (!id) return;
    let cancelled = false;
    imageGet(id)
      .then((detail) => !cancelled && setState({ id, status: "ready", detail }))
      .catch(() => !cancelled && setState({ id, status: "missing" }));
    return () => {
      cancelled = true;
    };
  }, [id]);

  // 一覧側でお気に入りを切り替えたとき等に表示を合わせる
  const current = state && state.id === id ? state : null;
  const detail = current?.status === "ready" ? current.detail : null;
  const image = detail ? (items.find((i) => i.id === detail.image.id) ?? stripItems.find((i) => i.id === detail.image.id) ?? detail.image) : null;
  const params = detail?.job.params;

  const run = async (action: () => Promise<unknown>, okKey?: Parameters<typeof t>[0]) => {
    try {
      await action();
      if (okKey) toast.success(t(okKey));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  };

  async function copy(text: string) {
    await run(() => navigator.clipboard.writeText(text), "gallery.copied");
  }

  async function exportImage() {
    if (!image) return;
    const ext = image.filePath.split(".").pop() ?? "png";
    const dest = await save({
      defaultPath: `gazooo-${image.seed}.${ext}`,
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
    if (dest) await run(() => imageExport(image.id, dest), "gallery.exported");
  }

  return (
    <>
      <Sheet open={id !== null} onOpenChange={(open) => !open && onClose()}>
        <SheetContent side="right" className="w-full overflow-y-auto sm:max-w-xl">
          <SheetHeader>
            <SheetTitle>{t("gallery.detail")}</SheetTitle>
            <SheetDescription className="sr-only">{t("gallery.detail")}</SheetDescription>
          </SheetHeader>

          {current?.status === "missing" && <p className="p-4 text-sm text-muted-foreground">{t("gallery.notFound")}</p>}

          {image && params && (
            <div className="flex flex-col gap-4 px-4 pb-6">
              <img
                src={imageSrc(image.filePath)}
                alt={params.prompt}
                width={image.width}
                height={image.height}
                className="max-h-[60vh] w-full rounded-lg border bg-muted/30 object-contain"
              />

              <div className="flex flex-wrap gap-2">
                <Button type="button" onClick={() => image && params && actions.loadSettings(image, params)}>
                  <Sparkles />
                  {t("gallery.useSettings")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void actions.setAsReference(image)}>
                  <ImagePlus />
                  {t("gallery.useAsReference")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void actions.setAsImg2img(image)}>
                  <ImagePlus />
                  {t("gallery.useAsImg2img")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void actions.setAsQwenReference(image)}>
                  <ImagePlus />
                  {t("gallery.useAsQwenReference")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void actions.setAsFaceSwapTarget(image)}>
                  <ImagePlus />
                  {t("gallery.useAsFaceSwapTarget")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void actions.setAsFaceSwapFace(image)}>
                  <ImagePlus />
                  {t("gallery.useAsFaceSwapFace")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void actions.setAsInpaint(image)}>
                  <Brush />
                  {t("gallery.useAsInpaint")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void actions.openTagger(image)}>
                  <Tags />
                  {t("gallery.makeTags")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  aria-pressed={image.isFavorite}
                  onClick={() => void run(() => setFavorite(image.id, !image.isFavorite))}
                >
                  <Star className={image.isFavorite ? "fill-amber-400 text-amber-400" : undefined} />
                  {image.isFavorite ? t("gallery.unfavorite") : t("gallery.favorite")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void exportImage()}>
                  <Download />
                  {t("gallery.export")}
                </Button>
                <Button type="button" variant="outline" onClick={() => void run(() => imageReveal(image.id))}>
                  <FolderOpen />
                  {t("gallery.reveal")}
                </Button>
                <Button type="button" variant="outline" className="text-destructive" onClick={() => setConfirmDelete(true)}>
                  <Trash2 />
                  {t("gallery.delete")}
                </Button>
              </div>

              <dl className="grid grid-cols-[7rem_1fr] gap-x-3 gap-y-2 text-sm">
                <dt className="text-muted-foreground">{t("gallery.project")}</dt>
                <dd className="flex items-center gap-2">
                  {(() => {
                    const project = projects.find((p) => p.id === image.projectId);
                    return (
                      <span className="flex min-w-0 items-center gap-1.5">
                        <ProjectDot color={project?.color ?? null} />
                        <span className="truncate">{project?.name ?? t("project.none")}</span>
                      </span>
                    );
                  })()}
                  <MoveToProjectMenu images={[image]} align="start" trigger={t("gallery.projectChange")} />
                </dd>
                <dt className="text-muted-foreground">{t("gallery.params.prompt")}</dt>
                <dd className="flex items-start gap-1 break-words">
                  <span className="min-w-0 flex-1 whitespace-pre-wrap">{params.prompt}</span>
                  <Button type="button" variant="ghost" size="icon-xs" aria-label={t("gallery.copyPrompt")} title={t("gallery.copyPrompt")} onClick={() => void copy(params.prompt)}>
                    <Copy />
                  </Button>
                </dd>
                {params.negativePrompt && (
                  <>
                    <dt className="text-muted-foreground">{t("gallery.params.negative")}</dt>
                    <dd className="whitespace-pre-wrap break-words">{params.negativePrompt}</dd>
                  </>
                )}
                <dt className="text-muted-foreground">{t("gallery.params.model")}</dt>
                <dd className="break-all">{params.model.name}</dd>
                <dt className="text-muted-foreground">{t("gallery.params.size")}</dt>
                <dd>
                  {image.width}×{image.height}
                </dd>
                <dt className="text-muted-foreground">{t("gallery.params.steps")}</dt>
                <dd>
                  {params.steps}
                  {params.model.kind.startsWith("flux") ? ` · guidance ${params.guidance}` : ` · CFG ${params.cfg}`}
                </dd>
                <dt className="text-muted-foreground">{t("gallery.params.sampler")}</dt>
                <dd>
                  {params.sampler} / {params.scheduler}
                </dd>
                {params.controlnet?.enabled && params.controlnet.units[0] && (
                  <>
                    <dt className="text-muted-foreground">{t("gallery.params.controlnet")}</dt>
                    <dd className="break-all">
                      {params.controlnet.units[0].preprocessor} · {params.controlnet.units[0].model} ·{" "}
                      {params.controlnet.units[0].strength}
                    </dd>
                  </>
                )}
                {params.faceSwap?.enabled && (
                  <>
                    <dt className="text-muted-foreground">{t("gallery.params.faceSwap")}</dt>
                    <dd>
                      {params.faceSwap.keepOutside ? params.faceSwap.region : t("gallery.params.faceSwap.noComposite")} ·{" "}
                      {params.faceSwap.resolution}
                    </dd>
                  </>
                )}
                {params.reference?.enabled && (
                  <>
                    <dt className="text-muted-foreground">{t("gallery.params.reference")}</dt>
                    <dd>
                      {params.reference.images.length} · {params.reference.resolution}
                    </dd>
                  </>
                )}
                {params.img2img?.enabled && (
                  <>
                    <dt className="text-muted-foreground">{t("gallery.params.img2img")}</dt>
                    <dd>denoise {params.img2img.denoise}</dd>
                  </>
                )}
                {params.inpaint?.enabled && (
                  <>
                    <dt className="text-muted-foreground">{t("gallery.params.inpaint")}</dt>
                    <dd>
                      denoise {params.inpaint.denoise} · grow {params.inpaint.growMask} · blur {params.inpaint.feather}
                    </dd>
                  </>
                )}
                {params.faceDetailer?.enabled && (
                  <>
                    <dt className="text-muted-foreground">{t("gallery.params.face")}</dt>
                    <dd className="break-all">
                      {params.faceDetailer.detector.replace(/^bbox\//, "")} · {params.faceDetailer.denoise}
                      {params.faceDetailer.prompt.trim() && ` · ${params.faceDetailer.prompt}`}
                    </dd>
                  </>
                )}
                <dt className="text-muted-foreground">{t("gallery.params.seed")}</dt>
                <dd className="flex items-center gap-1">
                  <span>{image.seed}</span>
                  <Button type="button" variant="ghost" size="icon-xs" aria-label={t("gallery.copySeed")} title={t("gallery.copySeed")} onClick={() => void copy(String(image.seed))}>
                    <Copy />
                  </Button>
                </dd>
                {detail && durationMs(detail.job) !== null && (
                  <>
                    <dt className="text-muted-foreground">{t("queue.detail.took")}</dt>
                    <dd>{formatDuration(durationMs(detail.job) ?? 0)}</dd>
                  </>
                )}
                <dt className="text-muted-foreground">{t("gallery.params.created")}</dt>
                <dd>{new Date(image.createdAt).toLocaleString()}</dd>
              </dl>
            </div>
          )}
        </SheetContent>
      </Sheet>

      <DeleteConfirm
        open={confirmDelete}
        count={1}
        onOpenChange={setConfirmDelete}
        onConfirm={() => {
          setConfirmDelete(false);
          if (!image) return;
          void run(() => remove([image.id], true), "gallery.deleted").then(onClose);
        }}
      />
    </>
  );
}
