import { beforeEach, describe, expect, it, vi } from "vitest";

import { defaultParams } from "@/lib/generation/defaults";
import type { Project, ProjectDetail, ProjectList } from "@/lib/tauri/projects";

const projectList = vi.fn<() => Promise<ProjectList>>();
const projectGet = vi.fn<(id: string) => Promise<ProjectDetail>>();
const projectCreate = vi.fn<(name: string, color: string | null) => Promise<Project>>();
const projectUpdate = vi.fn<(id: string, name: string, color: string | null) => Promise<Project>>();
const projectDelete = vi.fn<(id: string) => Promise<number>>();
const projectReorder = vi.fn<(ids: string[]) => Promise<void>>();
vi.mock("@/lib/tauri/projects", () => ({
  projectList: () => projectList(),
  projectGet: (id: string) => projectGet(id),
  projectCreate: (n: string, c: string | null) => projectCreate(n, c),
  projectUpdate: (i: string, n: string, c: string | null) => projectUpdate(i, n, c),
  projectDelete: (id: string) => projectDelete(id),
  projectReorder: (ids: string[]) => projectReorder(ids),
}));
// 画面状態の保存は設定ストア経由。ここでは保存先を使わない
vi.mock("./settings-store", () => ({ useSettingsStore: { getState: () => ({ update: vi.fn().mockResolvedValue(undefined) }) } }));

import { useFormStore } from "./form-store";
import { useProjectStore } from "./project-store";
import { useUiStore } from "./ui-store";

const proj = (id: string, over: Partial<Project> = {}): Project => ({
  id,
  name: id,
  color: null,
  sortOrder: 0,
  imageCount: 0,
  coverImageId: null,
  hasLastParams: false,
  createdAt: 0,
  updatedAt: 0,
  ...over,
});

const list = (ids: string[]): ProjectList => ({
  projects: ids.map((i) => proj(i)),
  totalImages: 10,
  favoriteImages: 3,
  unassignedImages: 4,
});

const detail = (id: string, prompt: string | null): ProjectDetail => ({
  project: proj(id),
  lastParams: prompt === null ? null : { ...defaultParams("sdxl"), prompt },
});

beforeEach(() => {
  for (const m of [projectList, projectGet, projectCreate, projectUpdate, projectDelete, projectReorder]) m.mockReset();
  projectList.mockResolvedValue(list(["p1", "p2"]));
  useProjectStore.setState({ projects: [], totals: { total: 0, favorites: 0, unassigned: 0 }, loaded: false, currentId: null });
  useUiStore.setState({ currentProjectId: null });
  useFormStore.setState({ params: { ...defaultParams("sdxl"), prompt: "current form" } });
});

describe("project-store: 読み込み", () => {
  it("一覧と件数(すべて・お気に入り・未分類)を取り込む", async () => {
    await useProjectStore.getState().load();
    const s = useProjectStore.getState();
    expect(s.projects.map((p) => p.id)).toEqual(["p1", "p2"]);
    expect(s.totals).toEqual({ total: 10, favorites: 3, unassigned: 4 });
    expect(s.loaded).toBe(true);
  });

  it("現在のプロジェクトが無くなっていたら、未分類へ戻して保存する", async () => {
    useProjectStore.setState({ currentId: "gone" });
    await useProjectStore.getState().load();
    expect(useProjectStore.getState().currentId).toBeNull();
    expect(useUiStore.getState().currentProjectId).toBeNull();
  });

  it("保存された現在のプロジェクトを復元する", async () => {
    await useProjectStore.getState().load();
    useUiStore.setState({ currentProjectId: "p2" });
    useProjectStore.getState().restoreCurrent();
    expect(useProjectStore.getState().currentId).toBe("p2");
    useUiStore.setState({ currentProjectId: "unknown" });
    useProjectStore.getState().restoreCurrent();
    expect(useProjectStore.getState().currentId).toBeNull();
  });
});

describe("project-store: 切り替え(docs/40 §4.1)", () => {
  it("前回の設定があればフォームへ適用し、「元に戻す」で戻せる", async () => {
    projectGet.mockResolvedValue(detail("p1", "last of p1"));
    const r = await useProjectStore.getState().switchTo("p1");
    expect(r.restored).toBe(true);
    expect(useFormStore.getState().params.prompt).toBe("last of p1");
    expect(useProjectStore.getState().currentId).toBe("p1");
    expect(useUiStore.getState().currentProjectId).toBe("p1");

    r.undo?.();
    expect(useFormStore.getState().params.prompt).toBe("current form");
  });

  it("前回の設定が無ければ、今のフォームを維持する(新規プロジェクトで直前の設定から始める)", async () => {
    projectGet.mockResolvedValue(detail("p2", null));
    const r = await useProjectStore.getState().switchTo("p2");
    expect(r).toEqual({ restored: false, undo: null });
    expect(useFormStore.getState().params.prompt).toBe("current form");
  });

  it("未分類への切り替えはフォームに触れず、IPC も呼ばない", async () => {
    useProjectStore.setState({ currentId: "p1" });
    const r = await useProjectStore.getState().switchTo(null);
    expect(r.restored).toBe(false);
    expect(projectGet).not.toHaveBeenCalled();
    expect(useProjectStore.getState().currentId).toBeNull();
    expect(useFormStore.getState().params.prompt).toBe("current form");
  });

  it("取得に失敗したら、切り替えを取り消してエラーを投げる", async () => {
    useProjectStore.setState({ currentId: "p1" });
    useUiStore.setState({ currentProjectId: "p1" });
    projectGet.mockRejectedValue({ code: "PROJECT_NOT_FOUND", message: "x" });
    await expect(useProjectStore.getState().switchTo("p2")).rejects.toMatchObject({ code: "PROJECT_NOT_FOUND" });
    expect(useProjectStore.getState().currentId).toBe("p1");
    expect(useUiStore.getState().currentProjectId).toBe("p1");
  });

  it("続けて切り替えたとき、遅れて届いた古い応答でフォームを上書きしない", async () => {
    let resolveFirst: (d: ProjectDetail) => void = () => {};
    projectGet.mockImplementationOnce(() => new Promise((r) => (resolveFirst = r)));
    projectGet.mockResolvedValueOnce(detail("p2", "last of p2"));
    const first = useProjectStore.getState().switchTo("p1");
    await useProjectStore.getState().switchTo("p2");
    resolveFirst(detail("p1", "last of p1"));
    const r1 = await first;
    expect(r1.restored).toBe(false);
    expect(useFormStore.getState().params.prompt).toBe("last of p2");
    expect(useProjectStore.getState().currentId).toBe("p2");
  });
});

describe("project-store: 管理", () => {
  it("削除すると、現在のプロジェクトだった場合は未分類へ戻して一覧を取り直す", async () => {
    await useProjectStore.getState().load();
    useProjectStore.setState({ currentId: "p1" });
    projectDelete.mockResolvedValue(5);
    projectList.mockResolvedValue(list(["p2"]));
    expect(await useProjectStore.getState().remove("p1")).toBe(5);
    expect(useProjectStore.getState().currentId).toBeNull();
    expect(useProjectStore.getState().projects.map((p) => p.id)).toEqual(["p2"]);
  });

  it("名前の変更は、その場で一覧に反映する", async () => {
    await useProjectStore.getState().load();
    projectUpdate.mockResolvedValue(proj("p1", { name: "改名", color: "blue" }));
    await useProjectStore.getState().update("p1", "改名", "blue");
    expect(useProjectStore.getState().projects[0].name).toBe("改名");
  });

  it("並べ替えはすぐ反映し、失敗したら元の並びへ戻す", async () => {
    await useProjectStore.getState().load();
    projectReorder.mockResolvedValueOnce();
    await useProjectStore.getState().reorder(["p2", "p1"]);
    expect(useProjectStore.getState().projects.map((p) => p.id)).toEqual(["p2", "p1"]);

    projectReorder.mockRejectedValueOnce(new Error("boom"));
    await expect(useProjectStore.getState().reorder(["p1", "p2"])).rejects.toBeTruthy();
    expect(useProjectStore.getState().projects.map((p) => p.id)).toEqual(["p2", "p1"]);
  });

  it("作成したら一覧を取り直す(件数・並びはバックエンドが正)", async () => {
    projectCreate.mockResolvedValue(proj("p3"));
    projectList.mockResolvedValue(list(["p1", "p2", "p3"]));
    const created = await useProjectStore.getState().create("p3", null);
    expect(created.id).toBe("p3");
    expect(useProjectStore.getState().projects).toHaveLength(3);
  });
});
