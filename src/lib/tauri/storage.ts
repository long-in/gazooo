import { invoke } from "./invoke";

export interface MoveReport {
  /** コピーしたファイル数(原寸 + サムネイル)。 */
  moved: number;
  /** 元ファイルが見つからずスキップした数。 */
  missing: number;
}

/** 画像の保存先を変更する。`newDir = null` は既定に戻す。時間がかかることがある。 */
export const storageChangeImagesDir = (newDir: string | null, moveExisting: boolean) =>
  invoke<MoveReport>("storage_change_images_dir", { newDir, moveExisting });
