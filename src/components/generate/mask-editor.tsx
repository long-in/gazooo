"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Slider } from "@/components/ui/slider";
import { hasPaint, invertPaint, toGrayMask } from "@/lib/generation/inpaint";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import { type InputImage, inputImageGet, maskSave } from "@/lib/tauri/inputs";

import { imageSrc } from "@/lib/image-src";

/** 画面上の編集用キャンバスの長辺(px)。保存時に原寸へ拡大する。 */
const VIEW_LONG_SIDE = 640;
const MAX_UNDO = 20;

/**
 * マスクエディタ(docs/13 §3.1・§3.6)。塗った範囲 = 描き直す範囲。
 * 表示は元画像の上に半透明の赤を重ね、保存時に白黒の PNG へ変換して Rust へ渡す。
 */
export function MaskEditor({
  open,
  onOpenChange,
  image,
  existingMask,
  onSaved,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  image: InputImage;
  /** 既に保存済みのマスク(再編集用。相対パス)。 */
  existingMask: string;
  onSaved: (path: string) => void;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const undoStack = useRef<ImageData[]>([]);
  const drawing = useRef<{ x: number; y: number } | null>(null);
  const [brush, setBrush] = useState(40);
  const [erase, setErase] = useState(false);
  const [saving, setSaving] = useState(false);
  const [undoCount, setUndoCount] = useState(0);

  const scale = Math.min(1, VIEW_LONG_SIDE / Math.max(image.width, image.height));
  const viewW = Math.max(1, Math.round(image.width * scale));
  const viewH = Math.max(1, Math.round(image.height * scale));

  const ctx = useCallback(() => canvasRef.current?.getContext("2d", { willReadFrequently: true }) ?? null, []);

  // 開くたびに、保存済みのマスクがあれば読み込む(読めなければ空から始める)
  useEffect(() => {
    if (!open) return;
    undoStack.current = [];
    let cancelled = false;
    const id = requestAnimationFrame(() => {
      setUndoCount(0);
      const c = ctx();
      if (!c) return;
      c.clearRect(0, 0, viewW, viewH);
      if (!existingMask) return;
      void (async () => {
        try {
          const m = await inputImageGet(existingMask);
          const img = new Image();
          img.crossOrigin = "anonymous";
          img.src = imageSrc(m.filePath);
          await img.decode();
          if (cancelled) return;
          const off = document.createElement("canvas");
          off.width = viewW;
          off.height = viewH;
          const oc = off.getContext("2d", { willReadFrequently: true });
          if (!oc) return;
          oc.drawImage(img, 0, 0, viewW, viewH);
          const data = oc.getImageData(0, 0, viewW, viewH);
          for (let i = 0; i < data.data.length; i += 4) {
            const v = data.data[i]; // 白 = 描き直す範囲
            data.data[i] = 255;
            data.data[i + 1] = 0;
            data.data[i + 2] = 0;
            data.data[i + 3] = v > 127 ? 255 : 0;
          }
          c.putImageData(data, 0, 0);
        } catch {
          // 読み込めない(取得元の制限など)ときは空から始める
        }
      })();
    });
    return () => {
      cancelled = true;
      cancelAnimationFrame(id);
    };
  }, [open, existingMask, viewW, viewH, ctx]);

  function pushUndo() {
    const c = ctx();
    if (!c) return;
    undoStack.current.push(c.getImageData(0, 0, viewW, viewH));
    if (undoStack.current.length > MAX_UNDO) undoStack.current.shift();
    setUndoCount(undoStack.current.length);
  }

  function undo() {
    const c = ctx();
    const prev = undoStack.current.pop();
    if (!c || !prev) return;
    c.putImageData(prev, 0, 0);
    setUndoCount(undoStack.current.length);
  }

  function point(e: React.PointerEvent<HTMLCanvasElement>) {
    const r = e.currentTarget.getBoundingClientRect();
    return { x: ((e.clientX - r.left) / r.width) * viewW, y: ((e.clientY - r.top) / r.height) * viewH };
  }

  function stroke(from: { x: number; y: number }, to: { x: number; y: number }) {
    const c = ctx();
    if (!c) return;
    c.save();
    c.globalCompositeOperation = erase ? "destination-out" : "source-over";
    c.strokeStyle = "#ff0000";
    c.fillStyle = "#ff0000";
    c.lineWidth = brush;
    c.lineCap = "round";
    c.lineJoin = "round";
    c.beginPath();
    c.moveTo(from.x, from.y);
    c.lineTo(to.x, to.y);
    c.stroke();
    // 点だけの描画(クリック)用
    c.beginPath();
    c.arc(to.x, to.y, brush / 2, 0, Math.PI * 2);
    c.fill();
    c.restore();
  }

  function clearAll() {
    pushUndo();
    ctx()?.clearRect(0, 0, viewW, viewH);
  }

  function invert() {
    const c = ctx();
    if (!c) return;
    pushUndo();
    const data = c.getImageData(0, 0, viewW, viewH);
    invertPaint(data.data);
    c.putImageData(data, 0, 0);
  }

  async function save() {
    const c = ctx();
    if (!c) return;
    if (!hasPaint(c.getImageData(0, 0, viewW, viewH).data)) {
      toast.error(t("generate.inpaint.maskEmpty"));
      return;
    }
    setSaving(true);
    try {
      // 原寸の黒地に、塗った範囲を拡大して重ね、R チャンネルを白黒の値にする
      const out = document.createElement("canvas");
      out.width = image.width;
      out.height = image.height;
      const oc = out.getContext("2d", { willReadFrequently: true });
      if (!oc || !canvasRef.current) return;
      oc.fillStyle = "#000";
      oc.fillRect(0, 0, out.width, out.height);
      oc.imageSmoothingQuality = "high";
      oc.drawImage(canvasRef.current, 0, 0, out.width, out.height);
      const data = oc.getImageData(0, 0, out.width, out.height);
      toGrayMask(data.data);
      oc.putImageData(data, 0, 0);
      const blob = await new Promise<Blob | null>((resolve) => out.toBlob(resolve, "image/png"));
      if (!blob) throw new Error("toBlob");
      const saved = await maskSave(new Uint8Array(await blob.arrayBuffer()));
      onSaved(saved.path);
      onOpenChange(false);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    } finally {
      setSaving(false);
    }
  }

  // キーボード: Ctrl/Cmd+Z で元に戻す、[ ] でブラシサイズ
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        e.preventDefault();
        undo();
      } else if (e.key === "[") setBrush((b) => Math.max(2, b - 4));
      else if (e.key === "]") setBrush((b) => Math.min(200, b + 4));
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[95vh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("generate.inpaint.editor.title")}</DialogTitle>
          <DialogDescription>{t("generate.inpaint.editor.desc")}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" size="sm" variant={erase ? "outline" : "default"} onClick={() => setErase(false)}>
            {t("generate.inpaint.brush")}
          </Button>
          <Button type="button" size="sm" variant={erase ? "default" : "outline"} onClick={() => setErase(true)}>
            {t("generate.inpaint.eraser")}
          </Button>
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            {t("generate.inpaint.brushSize")}
            <Slider
              className="w-32"
              value={[brush]}
              min={2}
              max={200}
              step={2}
              onValueChange={(v) => setBrush(Array.isArray(v) ? v[0] : v)}
            />
            <span className="w-8 tabular-nums">{brush}</span>
          </label>
          <Button type="button" size="sm" variant="outline" disabled={undoCount === 0} onClick={undo}>
            {t("generate.inpaint.undo")}
          </Button>
          <Button type="button" size="sm" variant="outline" onClick={invert}>
            {t("generate.inpaint.invert")}
          </Button>
          <Button type="button" size="sm" variant="outline" onClick={clearAll}>
            {t("generate.inpaint.clear")}
          </Button>
        </div>

        <div className="flex justify-center">
          <div className="relative touch-none select-none" style={{ width: viewW, height: viewH, maxWidth: "100%" }}>
            <img
              src={imageSrc(image.filePath)}
              alt={t("generate.inpaint.image")}
              draggable={false}
              className="absolute inset-0 size-full rounded-md border object-fill"
            />
            <canvas
              ref={canvasRef}
              width={viewW}
              height={viewH}
              aria-label={t("generate.inpaint.canvas")}
              className="absolute inset-0 size-full cursor-crosshair rounded-md opacity-55"
              onPointerDown={(e) => {
                e.currentTarget.setPointerCapture(e.pointerId);
                pushUndo();
                const p = point(e);
                drawing.current = p;
                stroke(p, p);
              }}
              onPointerMove={(e) => {
                if (!drawing.current) return;
                const p = point(e);
                stroke(drawing.current, p);
                drawing.current = p;
              }}
              onPointerUp={() => (drawing.current = null)}
              onPointerCancel={() => (drawing.current = null)}
            />
          </div>
        </div>
        <p className="text-xs text-muted-foreground">{t("generate.inpaint.editor.hint")}</p>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button type="button" disabled={saving} onClick={() => void save()}>
            {t("generate.inpaint.editor.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
