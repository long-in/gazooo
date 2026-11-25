import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ImageItem, Page } from "@/lib/tauri/jobs";

const imageList = vi.fn<(opts: Record<string, unknown>) => Promise<Page<ImageItem>>>();
vi.mock("@/lib/tauri/images", () => ({
  imageList: (o: Record<string, unknown>) => imageList(o),
}));

import { useStripStore } from "./strip-store";

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
  useStripStore.setState({
    items: [], nextCursor: null, loading: false, loaded: false, error: null,
    query: "", favoritesOnly: false, collapsed: false, selectedId: null, selectedJobId: null, filter: "all",
  });
});

describe("strip-store", () => {
  it("loadMore は続きを重複なしで追加する", async () => {
    imageList.mockResolvedValueOnce(page(["a", "b"], "c1")).mockResolvedValueOnce(page(["b", "c"]));
    await useStripStore.getState().reload();
    await useStripStore.getState().loadMore();
    expect(useStripStore.getState().items.map((i) => i.id)).toEqual(["a", "b", "c"]);
    expect(useStripStore.getState().nextCursor).toBeNull();
  });

  it("生成された画像は先頭に追加し、重複は無視する", () => {
    useStripStore.setState({ items: [img("a")] });
    useStripStore.getState().applyCreated(img("n"));
    useStripStore.getState().applyCreated(img("n"));
    expect(useStripStore.getState().items.map((i) => i.id)).toEqual(["n", "a"]);
  });

  it("フィルタ中は生成された画像を追加しない", () => {
    useStripStore.setState({ items: [img("a")], query: "cat" });
    useStripStore.getState().applyCreated(img("n"));
    expect(useStripStore.getState().items).toHaveLength(1);
    useStripStore.setState({ query: "", favoritesOnly: true });
    useStripStore.getState().applyCreated(img("n"));
    expect(useStripStore.getState().items).toHaveLength(1);
  });

  it("削除された画像は一覧と選択から外す", () => {
    useStripStore.setState({ items: [img("a"), img("b")], selectedId: "a" });
    useStripStore.getState().applyDeleted(["a"]);
    expect(useStripStore.getState().items.map((i) => i.id)).toEqual(["b"]);
    expect(useStripStore.getState().selectedId).toBeNull();
  });

  it("お気に入りの切り替えを反映し、お気に入りのみ表示中の解除は一覧から外す", () => {
    useStripStore.setState({ items: [img("a"), img("b", { isFavorite: true })] });
    useStripStore.getState().applyFavorite("a", true);
    expect(useStripStore.getState().items[0].isFavorite).toBe(true);
    useStripStore.setState({ favoritesOnly: true });
    useStripStore.getState().applyFavorite("b", false);
    expect(useStripStore.getState().items.map((i) => i.id)).toEqual(["a"]);
  });

  it("画像とキューのカードは同時に選べない", () => {
    useStripStore.getState().toggleSelect("a");
    useStripStore.getState().toggleSelectJob("j1");
    expect(useStripStore.getState().selectedId).toBeNull();
    expect(useStripStore.getState().selectedJobId).toBe("j1");
    useStripStore.getState().select("b");
    expect(useStripStore.getState().selectedJobId).toBeNull();
  });

  it("同じ画像をもう一度選ぶと選択を解除する", () => {
    useStripStore.getState().toggleSelect("a");
    expect(useStripStore.getState().selectedId).toBe("a");
    useStripStore.getState().toggleSelect("a");
    expect(useStripStore.getState().selectedId).toBeNull();
  });
});
