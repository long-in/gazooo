import { create } from "zustand";

import { type InputImage, inputImageFromGallery } from "@/lib/tauri/inputs";

/** 「画像からタグを作る」ダイアログの開閉と対象画像(docs/39)。生成画面とギャラリーの両方から開く。 */
interface TaggerState {
  open: boolean;
  /** 解析する画像。ダイアログ内で選び直せる。 */
  image: InputImage | null;
  openDialog: (image?: InputImage | null) => void;
  /** ギャラリーの画像を取り込んで開く(失敗時は `AppError` を throw)。 */
  openFromGallery: (imageId: string) => Promise<void>;
  setImage: (image: InputImage | null) => void;
  close: () => void;
}

export const useTaggerStore = create<TaggerState>((set, get) => ({
  open: false,
  image: null,
  // 画像を渡さなければ、前回の画像を保ったまま開く
  openDialog: (image) => set({ open: true, image: image === undefined ? get().image : image }),
  openFromGallery: async (imageId) => {
    const image = await inputImageFromGallery(imageId);
    set({ open: true, image });
  },
  setImage: (image) => set({ image }),
  close: () => set({ open: false }),
}));
