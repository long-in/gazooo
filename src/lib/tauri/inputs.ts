import { invoke } from "./invoke";

/** ControlNet の参照画像(docs/10-assist-features.md §6)。 */
export interface InputImage {
  /** 画像保存先からの相対パス。`ControlnetUnit.image` に入れる。 */
  path: string;
  /** 絶対パス。表示には `convertFileSrc` を使う。 */
  filePath: string;
  width: number;
  height: number;
}

/** ファイル選択ダイアログは Rust 側で開く。キャンセルされたら null。 */
export const inputImagePick = () => invoke<InputImage | null>("input_image_pick");

/** 複数の画像を選んで取り込む(顔の入れ替えの対象画像。docs/42 §3)。キャンセルされたら空。 */
export const inputImagePickMany = () => invoke<InputImage[]>("input_image_pick_many");

export const inputImageFromGallery = (imageId: string) =>
  invoke<InputImage>("input_image_from_gallery", { imageId });

/** マスクエディタが描いた PNG を保存する(docs/13 §3.5)。Rust 側が形式・大きさを検証し、グレースケールにそろえる。 */
export const maskSave = (png: Uint8Array) => invoke<InputImage>("mask_save", png);

/** 存在確認を兼ねる。無ければ `IMAGE_NOT_FOUND`。 */
export const inputImageGet = (path: string) => invoke<InputImage>("input_image_get", { path });
