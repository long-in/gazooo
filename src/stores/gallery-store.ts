import { create } from "zustand";

import { pickAfterRemoval } from "@/lib/viewer-nav";
import { type AppError, toAppError } from "@/lib/tauri/errors";
import { scopeMatches } from "@/lib/projects/scope";
import { imageDelete, imageList, type ImageScopeArg, imageMove, imageSetFavorite } from "@/lib/tauri/images";
import type { ImageItem } from "@/lib/tauri/jobs";

const PAGE_SIZE = 60;

interface GalleryState {
  items: ImageItem[];
  nextCursor: string | null;
  loading: boolean;
  loaded: boolean;
  error: AppError | null;
  query: string;
  favoritesOnly: boolean;
  /** 見る範囲(プロジェクト。docs/40 §5.1)。 */
  scope: ImageScopeArg;
  /** 保存されたスコープの復元が済んだか(アプリの起動ごとに 1 回。docs/40 §5.1)。 */
  scopeRestored: boolean;
  selected: Set<string>;
  /** Shift+クリックの範囲選択の起点(直前に選択を切り替えた画像。docs/44 §9)。 */
  anchorId: string | null;
  /** ビューア表示で選択中の画像 ID(docs/37)。一覧の増減に合わせて補正する。 */
  viewerId: string | null;

  setViewerId: (id: string | null) => void;
  /** 先頭から読み込み直す(フィルタ変更時・初回)。 */
  reload: () => Promise<void>;
  loadMore: () => Promise<void>;
  setQuery: (query: string) => void;
  setFavoritesOnly: (value: boolean) => void;
  setScope: (scope: ImageScopeArg) => void;

  applyCreated: (image: ImageItem) => void;
  applyDeleted: (ids: string[]) => void;
  /** 画像の所属が変わったとき(`image://moved`)。範囲から外れた画像は一覧から外す。 */
  applyMoved: (ids: string[], projectId: string | null) => void;

  toggleSelect: (id: string) => void;
  /** アンカーから `id` までを表示順で追加選択する。アンカーが無い・一覧に無ければ `id` だけを選択する。 */
  selectRange: (id: string) => void;
  clearSelection: () => void;
  selectAll: () => void;

  /** 失敗時は `AppError` を throw する(呼び出し側で表示)。 */
  setFavorite: (id: string, value: boolean) => Promise<void>;
  setFavoriteMany: (ids: string[], value: boolean) => Promise<void>;
  remove: (ids: string[], deleteFiles: boolean) => Promise<void>;
  /** 選んだ画像を別のプロジェクトへ移す(null = 未分類)。失敗時は `AppError` を throw する。 */
  moveToProject: (ids: string[], projectId: string | null) => Promise<void>;
}

// 古いリクエストの応答が、新しい絞り込みの結果を上書きしないための世代番号
let generation = 0;

export const useGalleryStore = create<GalleryState>((set, get) => ({
  items: [],
  nextCursor: null,
  loading: false,
  loaded: false,
  error: null,
  query: "",
  favoritesOnly: false,
  scope: "all",
  scopeRestored: false,
  selected: new Set(),
  anchorId: null,
  viewerId: null,

  setViewerId: (viewerId) => set({ viewerId }),

  reload: async () => {
    const gen = ++generation;
    const { query, favoritesOnly, scope } = get();
    set({ loading: true, error: null });
    try {
      const page = await imageList({ query, favoritesOnly, scope, limit: PAGE_SIZE });
      if (gen !== generation) return;
      set((s) => ({
        items: page.items,
        nextCursor: page.nextCursor ?? null,
        loading: false,
        loaded: true,
        selected: new Set(),
        anchorId: null,
        viewerId: pickAfterRemoval(
          s.items.map((i) => i.id),
          page.items.map((i) => i.id),
          s.viewerId,
        ),
      }));
    } catch (e) {
      if (gen !== generation) return;
      set({ loading: false, loaded: true, error: toAppError(e) });
    }
  },

  loadMore: async () => {
    const { nextCursor, loading, query, favoritesOnly, scope } = get();
    if (!nextCursor || loading) return;
    const gen = generation;
    set({ loading: true });
    try {
      const page = await imageList({ query, favoritesOnly, scope, cursor: nextCursor, limit: PAGE_SIZE });
      if (gen !== generation) return;
      set((s) => {
        const seen = new Set(s.items.map((i) => i.id));
        return {
          items: [...s.items, ...page.items.filter((i) => !seen.has(i.id))],
          nextCursor: page.nextCursor ?? null,
          loading: false,
        };
      });
    } catch (e) {
      if (gen !== generation) return;
      set({ loading: false, error: toAppError(e) });
    }
  },

  setQuery: (query) => set({ query }),
  setFavoritesOnly: (favoritesOnly) => set({ favoritesOnly }),
  setScope: (scope) => set({ scope }),

  // 絞り込み中は、新しい画像が条件に合うか判定できないため追加しない(解除時の再読み込みで現れる)。
  // 範囲(プロジェクト)は所属で判定できるので、範囲外の画像は追加しない
  applyCreated: (image) =>
    set((s) => {
      if (s.query.trim() !== "" || s.favoritesOnly) return s;
      if (!scopeMatches(s.scope, image.projectId)) return s;
      if (s.items.some((i) => i.id === image.id)) return s;
      return { items: [image, ...s.items] };
    }),

  applyMoved: (ids, projectId) => {
    const moved = new Set(ids);
    const { scope, items } = get();
    if (scope === "all") {
      // 範囲は変わらない。所属だけ更新する(チップの表示用)
      set({ items: items.map((i) => (moved.has(i.id) ? { ...i, projectId } : i)) });
      return;
    }
    if (scopeMatches(scope, projectId)) {
      // 範囲へ移ってきた画像は、位置を決めるため読み込み直す(一覧に無いものがあるときのみ)
      const known = new Set(items.map((i) => i.id));
      if (ids.some((id) => !known.has(id))) void get().reload();
      return;
    }
    get().applyDeleted(ids);
  },

  applyDeleted: (ids) =>
    set((s) => {
      const gone = new Set(ids);
      const selected = new Set([...s.selected].filter((id) => !gone.has(id)));
      const items = s.items.filter((i) => !gone.has(i.id));
      const viewerId = pickAfterRemoval(
        s.items.map((i) => i.id),
        items.map((i) => i.id),
        s.viewerId,
      );
      const anchorId = s.anchorId && !gone.has(s.anchorId) ? s.anchorId : null;
      return { items, selected, anchorId, viewerId };
    }),

  toggleSelect: (id) =>
    set((s) => {
      const selected = new Set(s.selected);
      if (!selected.delete(id)) selected.add(id);
      return { selected, anchorId: id };
    }),
  selectRange: (id) =>
    set((s) => {
      const to = s.items.findIndex((i) => i.id === id);
      const from = s.anchorId ? s.items.findIndex((i) => i.id === s.anchorId) : -1;
      const selected = new Set(s.selected);
      if (to < 0) return s;
      if (from < 0) {
        selected.add(id);
        return { selected, anchorId: id };
      }
      const [lo, hi] = from <= to ? [from, to] : [to, from];
      for (let i = lo; i <= hi; i++) selected.add(s.items[i].id);
      return { selected };
    }),
  clearSelection: () => set({ selected: new Set(), anchorId: null }),
  selectAll: () => set((s) => ({ selected: new Set(s.items.map((i) => i.id)) })),

  setFavorite: async (id, value) => {
    await imageSetFavorite(id, value);
    if (get().favoritesOnly && !value) {
      // お気に入り絞り込み中に解除したら一覧から外す(ビューアの選択も次へ送る)
      get().applyDeleted([id]);
      return;
    }
    set((s) => ({ items: s.items.map((i) => (i.id === id ? { ...i, isFavorite: value } : i)) }));
  },

  setFavoriteMany: async (ids, value) => {
    await Promise.all(ids.map((id) => imageSetFavorite(id, value)));
    set((s) => {
      const target = new Set(ids);
      return {
        items:
          s.favoritesOnly && !value
            ? s.items.filter((i) => !target.has(i.id))
            : s.items.map((i) => (target.has(i.id) ? { ...i, isFavorite: value } : i)),
      };
    });
  },

  moveToProject: async (ids, projectId) => {
    await imageMove(ids, projectId);
    // 通知(`image://moved`)でも反映されるが、重複しても結果は同じ
    get().applyMoved(ids, projectId);
  },

  remove: async (ids, deleteFiles) => {
    try {
      await imageDelete(ids, deleteFiles);
    } finally {
      // 記録は削除されているのにファイル移動だけ失敗した場合も、一覧は実状に合わせる
      get().applyDeleted(ids);
    }
  },
}));
