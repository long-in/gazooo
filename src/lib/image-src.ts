import { convertFileSrc, isTauri } from "@tauri-apps/api/core";

/** ローカルの画像ファイルの絶対パスを、WebView で表示できる URL にする(Tauri 外では空)。 */
export function imageSrc(path: string): string {
  return isTauri() ? convertFileSrc(path) : "";
}
