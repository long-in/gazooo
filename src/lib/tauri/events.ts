import { isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { SuggestedChanges } from "./assistant";
import type { BackupProgress } from "./backup";
import type {
  ConvertEntry,
  ConvertFinished,
  ConvertItemResult,
  ConvertProgress,
} from "./convert";
import type { EngineStatus } from "./engine";
import type { InstallState } from "./engine-install";
import type { EngineProcState } from "./engine-process";
import type { ImageExportDone, ImageExportProgress } from "./images";
import type { ImageItem, Job, JobProgress } from "./jobs";
import type { LlmInstallState, LlmState } from "./llm";
import type { DownloadState } from "./models";

/** Tauri イベント購読の集約先(docs/04-ipc-api.md「イベント」)。Tauri 外では何もしない。 */
async function subscribe<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return listen<T>(event, (e) => handler(e.payload));
}

export const onEngineStatus = (handler: (s: EngineStatus) => void) =>
  subscribe<EngineStatus>("engine://status", handler);

export const onJobUpdated = (handler: (job: Job) => void) =>
  subscribe<Job>("job://updated", handler);

export const onJobProgress = (handler: (p: JobProgress) => void) =>
  subscribe<JobProgress>("job://progress", handler);

export const onQueueState = (handler: (payload: { paused: boolean }) => void) =>
  subscribe<{ paused: boolean }>("queue://state", handler);

export const onImageCreated = (handler: (image: ImageItem) => void) =>
  subscribe<ImageItem>("image://created", handler);

export const onImageDeleted = (handler: (payload: { ids: string[] }) => void) =>
  subscribe<{ ids: string[] }>("image://deleted", handler);

/** プロジェクトの作成・更新・削除・並べ替え・件数の変化(docs/40 §7)。 */
export const onProjectChanged = (handler: () => void) =>
  subscribe<null>("project://changed", () => handler());

/** 画像の所属が変わった(`projectId` = 移動先。null = 未分類)。 */
export const onImageMoved = (
  handler: (payload: { ids: string[]; projectId: string | null }) => void,
) =>
  subscribe<{ ids: string[]; projectId: string | null }>(
    "image://moved",
    handler,
  );

export const onAssistantDelta = (
  handler: (p: { requestId: string; text: string }) => void,
) =>
  subscribe<{ requestId: string; text: string }>("assistant://delta", handler);

export interface AssistantSuggestionEvent {
  requestId: string;
  index: number;
  summary: string;
  changes: SuggestedChanges;
  /** モデルの提案のとき(docs/18 §6.3)。 */
  model?: { familyId: string; reason: string } | null;
}

export const onAssistantSuggestion = (
  handler: (p: AssistantSuggestionEvent) => void,
) => subscribe<AssistantSuggestionEvent>("assistant://suggestion", handler);

export const onAssistantDone = (
  handler: (p: { requestId: string; cancelled: boolean }) => void,
) =>
  subscribe<{ requestId: string; cancelled: boolean }>(
    "assistant://done",
    handler,
  );

export const onAssistantError = (
  handler: (p: { requestId: string; code: string; message: string }) => void,
) =>
  subscribe<{ requestId: string; code: string; message: string }>(
    "assistant://error",
    handler,
  );

export const onLlmState = (
  handler: (p: { state: LlmState; message: string | null }) => void,
) =>
  subscribe<{ state: LlmState; message: string | null }>(
    "llm://state",
    handler,
  );

export const onLlmInstall = (handler: (s: LlmInstallState) => void) =>
  subscribe<LlmInstallState>("llm://install-progress", handler);

export const onTranslateMtState = (
  handler: (p: { state: LlmState; message: string | null }) => void,
) =>
  subscribe<{ state: LlmState; message: string | null }>(
    "translate-mt://state",
    handler,
  );

export const onTranslateMtInstall = (handler: (s: LlmInstallState) => void) =>
  subscribe<LlmInstallState>("translate-mt://install-progress", handler);

export const onModelsProgress = (handler: (s: DownloadState) => void) =>
  subscribe<DownloadState>("models://progress", handler);

export const onModelsDone = (handler: (s: DownloadState) => void) =>
  subscribe<DownloadState>("models://done", handler);

export const onEngineProcState = (handler: (s: EngineProcState) => void) =>
  subscribe<EngineProcState>("engine://proc-state", handler);

/** ComfyUI の自動復旧の通知(docs/47 §4.4)。 */
export interface EngineRecoveryEvent {
  phase: "restarting" | "recovered" | "gave_up";
  attempt: number;
  requeuedJobs?: number;
}

export const onEngineRecovery = (handler: (e: EngineRecoveryEvent) => void) =>
  subscribe<EngineRecoveryEvent>("engine://recovery", handler);

export const onEngineInstallState = (handler: (s: InstallState) => void) =>
  subscribe<InstallState>("engine://install-state", handler);

export const onEngineInstallLog = (handler: (p: { line: string }) => void) =>
  subscribe<{ line: string }>("engine://install-log", handler);

export const onEngineLog = (handler: (p: { line: string }) => void) =>
  subscribe<{ line: string }>("engine://log", handler);

export const onBackupProgress = (handler: (p: BackupProgress) => void) =>
  subscribe<BackupProgress>("backup:progress", handler);

/** ギャラリーの一括ダウンロード(docs/44 §4)。 */
export const onImageExportProgress = (
  handler: (p: ImageExportProgress) => void,
) => subscribe<ImageExportProgress>("image-export://progress", handler);

export const onImageExportDone = (handler: (d: ImageExportDone) => void) =>
  subscribe<ImageExportDone>("image-export://done", handler);

/** 画像の形式変換(docs/43 §8)。ドロップで一覧に加わった行。 */
export const onConvertAdded = (handler: (entries: ConvertEntry[]) => void) =>
  subscribe<ConvertEntry[]>("convert://added", handler);

/** ファイルをドラッグしている間(ドロップ領域の強調用)。 */
export const onConvertDrag = (handler: (over: boolean) => void) =>
  subscribe<{ over: boolean }>("convert://drag", (p) => handler(p.over));

export const onConvertProgress = (handler: (p: ConvertProgress) => void) =>
  subscribe<ConvertProgress>("convert://progress", handler);

export const onConvertItem = (handler: (r: ConvertItemResult) => void) =>
  subscribe<ConvertItemResult>("convert://item", handler);

export const onConvertFinished = (handler: (f: ConvertFinished) => void) =>
  subscribe<ConvertFinished>("convert://finished", handler);
