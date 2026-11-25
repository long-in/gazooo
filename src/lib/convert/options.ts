import type { ConvertEntry, ConvertSettings, OutFormat } from "@/lib/tauri/convert";

/** 品質スライダーが効くか(docs/43 §4.1。PNG は常にロスレス、WebP はロスレスのとき無視される)。 */
export function qualityApplies(format: OutFormat, webpLossless: boolean): boolean {
  if (format === "png") return false;
  if (format === "webp" && webpLossless) return false;
  return true;
}

/** アルファを背景色に合成するのは JPEG のときだけ(§4.2)。 */
export function backgroundApplies(format: OutFormat): boolean {
  return format === "jpeg";
}

/** 変換の対象にできる行(エラーのない行)。 */
export function runnableEntries(entries: ConvertEntry[]): ConvertEntry[] {
  return entries.filter((e) => e.error === null);
}

/** 変換ボタンを押せない理由。押せるなら null(理由は i18n キーで返す)。 */
export function runBlockReason(
  entries: ConvertEntry[],
  settings: Pick<ConvertSettings, "outputDir"> | null,
  running: boolean,
): "running" | "noEntries" | "noOutputDir" | null {
  if (running) return "running";
  if (runnableEntries(entries).length === 0) return "noEntries";
  if (!settings || settings.outputDir.trim() === "") return "noOutputDir";
  return null;
}

/** 新しい行を一覧へ足す(同じ `id` は無視)。 */
export function mergeEntries(current: ConvertEntry[], added: ConvertEntry[]): ConvertEntry[] {
  const known = new Set(current.map((e) => e.id));
  return [...current, ...added.filter((e) => !known.has(e.id))];
}

/** 一覧の画素数・サイズの表示用(例: `1920×1080`)。 */
export function dimensionsLabel(e: Pick<ConvertEntry, "width" | "height">): string {
  return e.width > 0 && e.height > 0 ? `${e.width}×${e.height}` : "";
}

/** 入力形式の表示名。 */
export function formatLabel(f: ConvertEntry["detectedFormat"]): string {
  switch (f) {
    case "webp":
      return "WebP";
    case "png":
      return "PNG";
    case "jpeg":
      return "JPEG";
    case "gif":
      return "GIF";
    case "bmp":
      return "BMP";
    case "tiff":
      return "TIFF";
    case "heic":
      return "HEIC";
    default:
      return "";
  }
}
