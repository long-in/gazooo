"use client";

import { useRouter } from "next/navigation";
import { useMemo } from "react";
import { toast } from "sonner";

import { supportsImg2img, supportsInpaint } from "@/lib/generation/defaults";
import { supportsFaceSwap } from "@/lib/generation/face-swap";
import { supportsReference } from "@/lib/generation/reference";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { inputImageFromGallery } from "@/lib/tauri/inputs";
import type { GenerationParams, ImageItem } from "@/lib/tauri/jobs";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";
import { useTaggerStore } from "@/stores/tagger-store";

/**
 * ギャラリーの画像を、生成フォームの設定・元画像にする操作(docs/27 §3.2)。
 * 詳細シート(ギャラリー)と生成ページのギャラリー帯の両方から使う。
 * `stay` が true のときは画面を移動しない(生成ページ上)。false なら生成画面(`/`)へ移動する。
 */
export function useImageActions(stay: boolean) {
  const t = useT();
  const router = useRouter();
  const errorMessage = useErrorMessage();

  return useMemo(() => {
    const goGenerate = () => {
      if (!stay) router.push("/");
    };
    const fail = (e: unknown) => toast.error(errorMessage(toAppError(e)));

    /** この画像の設定をフォームに読み込む(「この設定で生成」。シードは画像のシードで再現する)。 */
    function loadSettings(image: ImageItem, params: GenerationParams) {
      useFormStore.getState().loadParams({ ...params, seed: image.seed });
      goGenerate();
    }

    /** プロンプトとネガティブだけをフォームへ入れる。 */
    function loadPrompt(params: GenerationParams, mode: "replace" | "append") {
      const { patch } = useFormStore.getState();
      if (mode === "replace") {
        patch({ prompt: params.prompt, negativePrompt: params.negativePrompt });
        return;
      }
      const cur = useFormStore.getState().params;
      const join = (a: string, b: string) => (a.trim() === "" ? b : b.trim() === "" ? a : `${a.trim()}, ${b}`);
      patch({ prompt: join(cur.prompt, params.prompt), negativePrompt: join(cur.negativePrompt, params.negativePrompt) });
    }

    /** ControlNet の参照画像にする。 */
    async function setAsReference(image: ImageItem) {
      try {
        const input = await inputImageFromGallery(image.id);
        const controlnets = useEngineStore.getState().models?.controlnets ?? [];
        const enabled = useFormStore.getState().setReferenceImage(input.path, controlnets);
        if (enabled) toast.success(t("gallery.referenceSet"));
        else toast.info(t("gallery.referenceFlux"));
        goGenerate();
      } catch (e) {
        fail(e);
      }
    }

    /** Inpaint の元画像にする(マスクは生成画面で描く)。 */
    async function setAsInpaint(image: ImageItem) {
      try {
        const input = await inputImageFromGallery(image.id);
        const form = useFormStore.getState();
        form.setInpaintImage(input.path, input.width, input.height);
        if (supportsInpaint(form.params.model.kind)) {
          form.setInpaintEnabled(true);
          toast.success(t("gallery.inpaintSet"));
        } else {
          toast.info(t("gallery.inpaintFlux"));
        }
        goGenerate();
      } catch (e) {
        fail(e);
      }
    }

    /** Qwen-Image 2.1 の参照画像に追加する(docs/24 §6.2)。種別が違うときは取り込むだけで、有効にはしない。 */
    async function setAsQwenReference(image: ImageItem) {
      try {
        const input = await inputImageFromGallery(image.id);
        const form = useFormStore.getState();
        form.addReferenceImage(input.path, input.width, input.height);
        if (supportsReference(form.params.model.kind)) {
          form.setReferenceEnabled(true);
          toast.success(t("gallery.qwenReferenceSet"));
        } else {
          toast.info(t("gallery.qwenReferenceOther"));
        }
        goGenerate();
      } catch (e) {
        fail(e);
      }
    }

    /** 顔の入れ替えの対象画像に追加する(docs/42 §6.2)。種別が違うときは取り込むだけで、有効にはしない。 */
    async function setAsFaceSwapTarget(image: ImageItem) {
      try {
        const input = await inputImageFromGallery(image.id);
        const form = useFormStore.getState();
        form.addFaceSwapTargets([input.path]);
        if (supportsFaceSwap(form.params.model.kind)) {
          if (form.params.faceSwap?.enabled !== true) form.setFaceSwapEnabled(true);
          toast.success(t("gallery.faceSwapTargetSet"));
        } else {
          toast.info(t("gallery.faceSwapOther"));
        }
        goGenerate();
      } catch (e) {
        fail(e);
      }
    }

    /** 顔の入れ替えの顔画像にする。 */
    async function setAsFaceSwapFace(image: ImageItem) {
      try {
        const input = await inputImageFromGallery(image.id);
        const form = useFormStore.getState();
        form.patchFaceSwap({ face: input.path });
        if (supportsFaceSwap(form.params.model.kind)) {
          if (form.params.faceSwap?.enabled !== true) form.setFaceSwapEnabled(true);
          toast.success(t("gallery.faceSwapFaceSet"));
        } else {
          toast.info(t("gallery.faceSwapOther"));
        }
        goGenerate();
      } catch (e) {
        fail(e);
      }
    }

    /** img2img の元画像にする。 */
    async function setAsImg2img(image: ImageItem) {
      try {
        const input = await inputImageFromGallery(image.id);
        const form = useFormStore.getState();
        form.setImg2imgImage(input.path, input.width, input.height);
        if (supportsImg2img(form.params.model.kind)) {
          form.setImg2imgEnabled(true);
          toast.success(t("gallery.img2imgSet"));
        } else {
          toast.info(t("gallery.img2imgFlux"));
        }
        goGenerate();
      } catch (e) {
        fail(e);
      }
    }

    /** 「画像からタグを作る」ダイアログをこの画像で開く(docs/39 §4.1)。 */
    async function openTagger(image: ImageItem) {
      try {
        await useTaggerStore.getState().openFromGallery(image.id);
      } catch (e) {
        fail(e);
      }
    }

    return {
      loadSettings,
      loadPrompt,
      setAsReference,
      setAsInpaint,
      setAsQwenReference,
      setAsFaceSwapTarget,
      setAsFaceSwapFace,
      setAsImg2img,
      openTagger,
    };
  }, [stay, router, t, errorMessage]);
}
