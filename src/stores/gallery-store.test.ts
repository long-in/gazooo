import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ImageItem, Page } from "@/lib/tauri/jobs";

const imageList = vi.fn<(opts: Record<string, unknown>) => Promise<Page<ImageItem>>>();
const imageSetFavorite = vi.fn<(id: string, value: boolean) => Promise<void>>();
const imageDelete = vi.fn<(ids: string[], deleteFiles: boolean) => Promise<void>>();
const imageMove = vi.fn<(ids: string[], projectId: string | null) => Promise<void>>();
vi.mock("@/lib/tauri/images", () => ({
  imageMove: (ids: string[], p: string | null) => imageMove(ids, p),
  imageList: (o: Record<string, unknown>) => imageList(o),
  imageSetFavorite: (id: string, v: boolean) => imageSetFavorite(id, v),
  imageDelete: (ids: string[], d: boolean) => imageDelete(ids, d),
}));

import { useGalleryStore } from "./gallery-store";

const img = (id: string, over: Partial<ImageItem> = {}): ImageItem => ({
  id,
  jobId: "j",
  projectId: null,
  width: 1,
  height: 1,
  seed: 1,
  isFavorite: false,
  createdAt: 0,
  filePath: `/x/${id}.png`,
  thumbPath: `/x/${id}.jpg`,
  ...over,
});

const page = (ids: string[], nextCursor?: string): Page<ImageItem> => ({ items: ids.map((i) => img(i)), nextCursor });

beforeEach(() => {
  imageList.mockReset();
  imageSetFavorite.mockReset().mockResolvedValue();
  imageDelete.mockReset().mockResolvedValue();
  imageMove.mockReset().mockResolvedValue();
  useGalleryStore.setState({
    items: [], nextCursor: null, loading: false, loaded: false, error: null,
    query: "", favoritesOnly: false, scope: "all", selected: new Set(),
  });
});

describe("gallery-store paging", () => {
  it("reload は先頭ページを読み、loadMore は続きを重複なしで追加する", async () => {
    imageList.mockResolvedValueOnce(page(["a", "b"], "c1")).mockResolvedValueOnce(page(["b", "c"]));
    await useGalleryStore.getState().reload();
    expect(useGalleryStore.getState().nextCursor).toBe("c1");
    await useGalleryStore.getState().loadMore();
    const s = useGalleryStore.getState();
    expect(s.items.map((i) => i.id)).toEqual(["a", "b", "c"]);
    expect(s.nextCursor).toBeNull();
    expect(imageList).toHaveBeenLastCalledWith(expect.objectContaining({ cursor: "c1" }));
  });

  it("続きが無い・読み込み中は loadMore しない", async () => {
    await useGalleryStore.getState().loadMore();
    useGalleryStore.setState({ nextCursor: "c", loading: true });
    await useGalleryStore.getState().loadMore();
    expect(imageList).not.toHaveBeenCalled();
  });

  it("絞り込み変更後は、古い応答が新しい結果を上書きしない", async () => {
    let resolveOld: (p: Page<ImageItem>) => void = () => {};
    imageList.mockImplementationOnce(() => new Promise((r) => (resolveOld = r)));
    imageList.mockResolvedValueOnce(page(["new"]));

    const old = useGalleryStore.getState().reload();
    useGalleryStore.getState().setQuery("cat");
    await useGalleryStore.getState().reload();
    resolveOld(page(["stale"]));
    await old;

    expect(useGalleryStore.getState().items.map((i) => i.id)).toEqual(["new"]);
  });

  it("読み込み失敗はエラーとして保持する", async () => {
    imageList.mockRejectedValueOnce({ code: "DB_ERROR", message: "x" });
    await useGalleryStore.getState().reload();
    const s = useGalleryStore.getState();
    expect(s.error?.code).toBe("DB_ERROR");
    expect(s.loading).toBe(false);
  });
});

describe("gallery-store events and actions", () => {
  it("新しい画像は先頭に追加。絞り込み中・重複は追加しない", () => {
    useGalleryStore.setState({ items: [img("a")] });
    useGalleryStore.getState().applyCreated(img("b"));
    useGalleryStore.getState().applyCreated(img("b"));
    expect(useGalleryStore.getState().items.map((i) => i.id)).toEqual(["b", "a"]);

    useGalleryStore.setState({ query: "cat" });
    useGalleryStore.getState().applyCreated(img("c"));
    expect(useGalleryStore.getState().items).toHaveLength(2);
    useGalleryStore.setState({ query: "", favoritesOnly: true });
    useGalleryStore.getState().applyCreated(img("d"));
    expect(useGalleryStore.getState().items).toHaveLength(2);
  });

  it("削除イベントで一覧と選択から外れる", () => {
    useGalleryStore.setState({ items: [img("a"), img("b")], selected: new Set(["a", "b"]) });
    useGalleryStore.getState().applyDeleted(["a"]);
    const s = useGalleryStore.getState();
    expect(s.items.map((i) => i.id)).toEqual(["b"]);
    expect([...s.selected]).toEqual(["b"]);
  });

  it("選択の切り替え・全選択・解除", () => {
    useGalleryStore.setState({ items: [img("a"), img("b")] });
    const s = useGalleryStore.getState();
    s.toggleSelect("a");
    expect([...useGalleryStore.getState().selected]).toEqual(["a"]);
    s.toggleSelect("a");
    expect(useGalleryStore.getState().selected.size).toBe(0);
    s.selectAll();
    expect(useGalleryStore.getState().selected.size).toBe(2);
    s.clearSelection();
    expect(useGalleryStore.getState().selected.size).toBe(0);
  });

  it("Shift+クリックの範囲選択(双方向・既存選択を保つ・アンカー固定)", () => {
    useGalleryStore.setState({ items: ["a", "b", "c", "d", "e"].map((id) => img(id)), selected: new Set(), anchorId: null });
    const s = useGalleryStore.getState();
    s.selectRange("b"); // アンカー無し → 1 枚だけ
    expect([...useGalleryStore.getState().selected]).toEqual(["b"]);
    s.selectRange("d");
    expect([...useGalleryStore.getState().selected].sort()).toEqual(["b", "c", "d"]);
    s.selectRange("a"); // 同じアンカー(b)から逆方向。既存は保つ
    expect([...useGalleryStore.getState().selected].sort()).toEqual(["a", "b", "c", "d"]);
    s.toggleSelect("e"); // アンカーが e に移る
    s.selectRange("d");
    expect(useGalleryStore.getState().anchorId).toBe("e");
    s.clearSelection();
    expect(useGalleryStore.getState().anchorId).toBeNull();
    useGalleryStore.setState({ anchorId: "zzz" }); // 一覧に無いアンカー
    s.selectRange("c");
    expect([...useGalleryStore.getState().selected]).toEqual(["c"]);
  });

  it("お気に入りの切り替え。お気に入り絞り込み中の解除は一覧から外れる", async () => {
    useGalleryStore.setState({ items: [img("a"), img("b")] });
    await useGalleryStore.getState().setFavorite("a", true);
    expect(useGalleryStore.getState().items[0].isFavorite).toBe(true);

    useGalleryStore.setState({ favoritesOnly: true });
    await useGalleryStore.getState().setFavorite("a", false);
    expect(useGalleryStore.getState().items.map((i) => i.id)).toEqual(["b"]);
  });

  it("失敗したお気に入り操作は状態を変えない", async () => {
    imageSetFavorite.mockRejectedValueOnce({ code: "IMAGE_NOT_FOUND", message: "x" });
    useGalleryStore.setState({ items: [img("a")] });
    await expect(useGalleryStore.getState().setFavorite("a", true)).rejects.toMatchObject({ code: "IMAGE_NOT_FOUND" });
    expect(useGalleryStore.getState().items[0].isFavorite).toBe(false);
  });

  it("削除は IPC を呼んで一覧を更新する。ファイル移動に失敗しても記録は消えているので一覧は更新する", async () => {
    useGalleryStore.setState({ items: [img("a"), img("b")] });
    await useGalleryStore.getState().remove(["a"], true);
    expect(imageDelete).toHaveBeenCalledWith(["a"], true);
    expect(useGalleryStore.getState().items.map((i) => i.id)).toEqual(["b"]);

    imageDelete.mockRejectedValueOnce({ code: "STORAGE_ERROR", message: "x" });
    await expect(useGalleryStore.getState().remove(["b"], true)).rejects.toMatchObject({ code: "STORAGE_ERROR" });
    expect(useGalleryStore.getState().items).toHaveLength(0);
  });
});

describe("viewer selection", () => {
  it("moves to the next image when the selected one is deleted", async () => {
    useGalleryStore.setState({ items: [img("a"), img("b"), img("c")], viewerId: "b" });
    await useGalleryStore.getState().remove(["b"], true);
    expect(useGalleryStore.getState().viewerId).toBe("c");
  });

  it("falls back to the first image after reload when the selection vanished", async () => {
    useGalleryStore.setState({ items: [img("a"), img("b")], viewerId: "b" });
    imageList.mockResolvedValue(page(["x", "y"]));
    await useGalleryStore.getState().reload();
    expect(useGalleryStore.getState().viewerId).toBe("x");
  });

  it("advances the selection when unfavoriting under the favorites filter", async () => {
    useGalleryStore.setState({ items: [img("a"), img("b")], viewerId: "a", favoritesOnly: true });
    await useGalleryStore.getState().setFavorite("a", false);
    expect(useGalleryStore.getState().viewerId).toBe("b");
  });
});

describe("gallery-store のスコープ(docs/40 §5.1)", () => {
  it("reload / loadMore は scope を渡す", async () => {
    imageList.mockResolvedValueOnce(page(["a"], "c1")).mockResolvedValueOnce(page(["b"]));
    useGalleryStore.getState().setScope({ projectId: "p1" });
    await useGalleryStore.getState().reload();
    await useGalleryStore.getState().loadMore();
    expect(imageList).toHaveBeenNthCalledWith(1, expect.objectContaining({ scope: { projectId: "p1" } }));
    expect(imageList).toHaveBeenNthCalledWith(2, expect.objectContaining({ scope: { projectId: "p1" }, cursor: "c1" }));
  });

  it("お気に入りのスコープは scope = all + favoritesOnly", async () => {
    imageList.mockResolvedValueOnce(page([]));
    useGalleryStore.setState({ scope: "all", favoritesOnly: true });
    await useGalleryStore.getState().reload();
    expect(imageList).toHaveBeenCalledWith(expect.objectContaining({ scope: "all", favoritesOnly: true }));
  });

  it("生成された画像は、範囲に合うときだけ先頭へ追加する", () => {
    useGalleryStore.setState({ items: [img("a")], scope: { projectId: "p1" } });
    useGalleryStore.getState().applyCreated(img("x", { projectId: "p2" }));
    useGalleryStore.getState().applyCreated(img("y", { projectId: null }));
    expect(useGalleryStore.getState().items.map((i) => i.id)).toEqual(["a"]);
    useGalleryStore.getState().applyCreated(img("z", { projectId: "p1" }));
    expect(useGalleryStore.getState().items.map((i) => i.id)).toEqual(["z", "a"]);

    useGalleryStore.setState({ items: [], scope: "unassigned" });
    useGalleryStore.getState().applyCreated(img("u", { projectId: null }));
    useGalleryStore.getState().applyCreated(img("v", { projectId: "p1" }));
    expect(useGalleryStore.getState().items.map((i) => i.id)).toEqual(["u"]);
  });

  it("「すべて」では、移動しても一覧に残り、所属だけ更新する", () => {
    useGalleryStore.setState({ items: [img("a"), img("b")], scope: "all" });
    useGalleryStore.getState().applyMoved(["a"], "p1");
    const items = useGalleryStore.getState().items;
    expect(items.map((i) => i.id)).toEqual(["a", "b"]);
    expect(items[0].projectId).toBe("p1");
    expect(items[1].projectId).toBeNull();
  });

  it("プロジェクトの一覧では、範囲外へ移した画像を外し、ビューアの選択も次へ送る", () => {
    useGalleryStore.setState({
      items: [img("a", { projectId: "p1" }), img("b", { projectId: "p1" }), img("c", { projectId: "p1" })],
      scope: { projectId: "p1" },
      viewerId: "b",
    });
    useGalleryStore.getState().applyMoved(["b"], "p2");
    const s = useGalleryStore.getState();
    expect(s.items.map((i) => i.id)).toEqual(["a", "c"]);
    expect(s.viewerId).toBe("c");
  });

  it("範囲へ移ってきた画像が一覧に無ければ読み込み直す(あれば何もしない)", async () => {
    imageList.mockResolvedValue(page(["a", "n"]));
    useGalleryStore.setState({ items: [img("a", { projectId: "p1" })], scope: { projectId: "p1" } });
    useGalleryStore.getState().applyMoved(["a"], "p1");
    expect(imageList).not.toHaveBeenCalled();
    useGalleryStore.getState().applyMoved(["n"], "p1");
    expect(imageList).toHaveBeenCalledTimes(1);
  });

  it("moveToProject は IPC を呼んで一覧に反映する", async () => {
    useGalleryStore.setState({ items: [img("a", { projectId: "p1" })], scope: { projectId: "p1" } });
    await useGalleryStore.getState().moveToProject(["a"], null);
    expect(imageMove).toHaveBeenCalledWith(["a"], null);
    expect(useGalleryStore.getState().items).toEqual([]);
  });

  it("moveToProject が失敗したら一覧は変えず、エラーを投げる", async () => {
    imageMove.mockRejectedValueOnce(new Error("boom"));
    useGalleryStore.setState({ items: [img("a", { projectId: "p1" })], scope: { projectId: "p1" } });
    await expect(useGalleryStore.getState().moveToProject(["a"], null)).rejects.toThrow("boom");
    expect(useGalleryStore.getState().items).toHaveLength(1);
  });
});
