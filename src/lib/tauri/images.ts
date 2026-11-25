import { invoke } from "./invoke";
import type { ImageItem, Job, Page } from "./jobs";

export interface ImageDetail {
  image: ImageItem;
  job: Job;
}

/** 一覧の範囲(docs/40 §7)。省略 = "all"。 */
export type ImageScopeArg = "all" | "unassigned" | { projectId: string };

export interface ImageListOptions {
  query?: string;
  favoritesOnly?: boolean;
  scope?: ImageScopeArg;
  cursor?: string;
  limit?: number;
}

export const imageList = (opts: ImageListOptions = {}) => invoke<Page<ImageItem>>("image_list", { ...opts });

export const imageGet = (id: string) => invoke<ImageDetail>("image_get", { id });

export const imageSetFavorite = (id: string, value: boolean) =>
  invoke<void>("image_set_favorite", { id, value });

/** `deleteFiles` が true なら、ファイルは OS のゴミ箱へ移動される。 */
export const imageDelete = (ids: string[], deleteFiles: boolean) =>
  invoke<void>("image_delete", { ids, deleteFiles });

export const imageExport = (id: string, destPath: string) => invoke<void>("image_export", { id, destPath });

/** 一括ダウンロード(docs/44)。Rust 側でフォルダを選ばせる。キャンセルは null。進捗・結果はイベント。 */
export interface ImageExportProgress {
  jobId: string;
  done: number;
  total: number;
}
export interface ImageExportDone {
  jobId: string;
  saved: number;
  failed: { id: string; error: string }[];
  cancelled: boolean;
  error: string | null;
}

export const imageExportMany = (ids: string[]) => invoke<string | null>("image_export_many", { ids });
export const imageExportCancel = (jobId: string) => invoke<void>("image_export_cancel", { jobId });
export const imageExportOpenDir = () => invoke<void>("image_export_open_dir");

export const imageReveal = (id: string) => invoke<void>("image_reveal", { id });

/** 画像を別のプロジェクトへ移す(`projectId` が null なら未分類へ。ファイルは動かさない)。 */
export const imageMove = (ids: string[], projectId: string | null) =>
  invoke<void>("image_move", { ids, projectId });
