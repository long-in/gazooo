import type { GpuStatus, InstallState, InstallView, Preflight } from "@/lib/tauri/engine-install";

/** 選択したコンポーネントの合計(基本を含む。MB)。 */
export function selectionTotals(view: InstallView, selected: string[]): { downloadMb: number; diskMb: number } {
  let downloadMb = view.base.downloadMb;
  let diskMb = view.base.diskMb;
  for (const c of view.components) {
    if (selected.includes(c.id)) {
      downloadMb += c.downloadMb;
      diskMb += c.diskMb;
    }
  }
  return { downloadMb, diskMb };
}

/** 導入済みの記録に無い部品(「追加で導入」の対象。docs/23 §8)。 */
export function missingComponents(view: InstallView): InstallView["components"] {
  const have = view.record?.components ?? [];
  return view.components.filter((c) => !have.includes(c.id));
}

/** 「追加で導入」の合計(導入済みの部品と基本は含めない。MB)。 */
export function addTotals(view: InstallView, installed: string[], selected: string[]): { downloadMb: number; diskMb: number } {
  let downloadMb = 0;
  let diskMb = 0;
  for (const c of view.components) {
    if (selected.includes(c.id) && !installed.includes(c.id)) {
      downloadMb += c.downloadMb;
      diskMb += c.diskMb;
    }
  }
  return { downloadMb, diskMb };
}

/** MB を「3.5 GB」「600 MB」の形にする。 */
export function formatMb(mb: number): string {
  return mb >= 1000 ? `${(mb / 1000).toFixed(mb >= 10000 ? 0 : 1)} GB` : `${Math.round(mb)} MB`;
}

/** バイト数を「12.3 GB」の形にする。 */
export function formatBytes(n: number): string {
  return formatMb(n / 1e6);
}

/** 先へ進めるか(対象外の環境・容量不足では進めない)。 */
export function canProceed(p: Preflight | null): boolean {
  return p !== null && p.supported && p.enoughSpace;
}

/** 警告として出すべき GPU の状態(NVIDIA が見つからない Windows)。 */
export function needsGpuConfirm(gpu: GpuStatus): boolean {
  return gpu === "missing";
}

/**
 * ダウンロード中の手順内の進捗(0〜100)。サイズが分からない手順(pip など)は null。
 */
export function stepPercent(s: Pick<InstallState, "received" | "total">): number | null {
  if (s.received === null || s.total === null || s.total <= 0) return null;
  return Math.min(100, Math.floor((s.received / s.total) * 100));
}

/** 手順全体の進捗(0〜100)。手順番号を基準に、手順内のダウンロード進捗を加味する。 */
export function overallPercent(s: InstallState): number {
  if (s.status === "done") return 100;
  if (s.step <= 0 || s.totalSteps <= 0) return 0;
  const inStep = (stepPercent(s) ?? 0) / 100;
  return Math.min(100, Math.floor(((s.step - 1 + inStep) / s.totalSteps) * 100));
}

export const isRunning = (s: InstallState | null | undefined) => s?.status === "running";
