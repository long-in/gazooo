import { invoke } from "./invoke";

/** 画像のタグ付けの結果(docs/39 §6.1)。 */
export interface ImageTagResult {
  /** カンマ + スペース区切りのタグ列。タグが 1 つも見つからなければ空文字。 */
  tags: string;
  /** 実際に使った解析モデル。 */
  model: string;
}

export interface ImageTagOptions {
  model?: string;
  threshold?: number;
  characterThreshold?: number;
  excludeTags?: string;
}

/**
 * 取り込み済みの参照画像(`InputImage.path`)を解析し、タグ列を返す。
 * 生成ジョブの実行中は `ENGINE_BUSY`、ノードが無ければ `ENGINE_NODE_MISSING`。
 */
export const imageTag = (image: string, options: ImageTagOptions = {}) =>
  invoke<ImageTagResult>("image_tag", { image, ...options });
