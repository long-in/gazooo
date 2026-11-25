import { beforeEach, describe, expect, it, vi } from "vitest";

import { defaultParams } from "@/lib/generation/defaults";

const api = vi.hoisted(() => ({
  assistantStatus: vi.fn(),
  assistantAsk: vi.fn(),
  assistantCancel: vi.fn(),
  assistantConversationList: vi.fn(),
  assistantConversationGet: vi.fn(),
  assistantConversationDelete: vi.fn(),
  assistantSuggestionApplied: vi.fn(),
}));
vi.mock("@/lib/tauri/assistant", () => api);

import { useAssistantStore } from "./assistant-store";

beforeEach(() => {
  Object.values(api).forEach((f) => f.mockReset());
  api.assistantConversationList.mockResolvedValue([]);
  api.assistantSuggestionApplied.mockResolvedValue(undefined);
  api.assistantAsk.mockResolvedValue({ requestId: "r1", conversationId: "c1", assistantMessageId: "m1" });
  useAssistantStore.setState({ conversationId: null, messages: [], requestId: null, error: null });
});

describe("assistant store", () => {
  it("streams an answer and a suggestion into the pending message", async () => {
    const s = useAssistantStore.getState();
    await s.send("質問", defaultParams("sd15"));
    expect(useAssistantStore.getState().messages.map((m) => m.role)).toEqual(["user", "assistant"]);
    expect(useAssistantStore.getState().conversationId).toBe("c1");

    s.onDelta("r1", "こんに");
    s.onDelta("r1", "ちは");
    s.onDelta("other", "混入しない");
    s.onSuggestion("r1", 0, "増やす", { steps: 40 });
    s.onDone("r1");

    const st = useAssistantStore.getState();
    expect(st.requestId).toBeNull();
    const a = st.messages[1];
    expect(a.text).toBe("こんにちは");
    expect(a.pending).toBe(false);
    expect(a.suggestions[0]).toEqual({ summary: "増やす", changes: { steps: 40 }, applied: false });
  });

  it("drops an empty answer and keeps the error on failure", async () => {
    await useAssistantStore.getState().send("q", defaultParams("sd15"));
    useAssistantStore.getState().onError("r1", { code: "ASSISTANT_RATE_LIMIT", message: "x" });
    const st = useAssistantStore.getState();
    expect(st.messages).toHaveLength(1);
    expect(st.error?.code).toBe("ASSISTANT_RATE_LIMIT");
    expect(st.requestId).toBeNull();
  });

  it("marks a suggestion applied locally and on the backend", async () => {
    useAssistantStore.setState({
      messages: [
        {
          id: "m1",
          role: "assistant",
          text: "a",
          suggestions: [{ summary: "s", changes: { steps: 1 }, applied: false }],
        },
      ],
    });
    await useAssistantStore.getState().markApplied("m1", 0);
    expect(useAssistantStore.getState().messages[0].suggestions[0].applied).toBe(true);
    expect(api.assistantSuggestionApplied).toHaveBeenCalledWith("m1", 0);
  });

  it("does not switch conversations while a request is running", () => {
    useAssistantStore.setState({ requestId: "r", conversationId: "c", messages: [{ id: "x", role: "user", text: "t", suggestions: [] }] });
    useAssistantStore.getState().newConversation();
    expect(useAssistantStore.getState().conversationId).toBe("c");
  });

  it("clears the open conversation when it is deleted", async () => {
    api.assistantConversationDelete.mockResolvedValue({ deleted: 1 });
    useAssistantStore.setState({ conversationId: "c", messages: [{ id: "x", role: "user", text: "t", suggestions: [] }] });
    await useAssistantStore.getState().deleteConversations(["c"]);
    expect(useAssistantStore.getState().conversationId).toBeNull();
    expect(useAssistantStore.getState().messages).toEqual([]);
  });
});

describe("assistant store: モデルの提案(docs/18)", () => {
  it("モデルの提案を、設定の提案とは別のカードとして受け取る", async () => {
    const s = useAssistantStore.getState();
    await s.send("アニメ調にしたい", defaultParams("sdxl"));
    s.onDelta("r1", "NoobAI-XL がおすすめです");
    s.onSuggestion("r1", 0, "Danbooru タグで指定できる", {}, { familyId: "checkpoint-noobai-xl", reason: "Danbooru タグで指定できる" });
    s.onSuggestion("r1", 1, "増やす", { steps: 28 });
    s.onDone("r1");
    const [model, params] = useAssistantStore.getState().messages[1].suggestions;
    expect(model.model).toEqual({ familyId: "checkpoint-noobai-xl", reason: "Danbooru タグで指定できる" });
    expect(model.applied).toBe(false);
    expect(params.model).toBeUndefined();
    expect(params.changes).toEqual({ steps: 28 });
  });

  it("「AI に相談」: 下書きを入れてパネルを開く(送信はしない)", () => {
    useAssistantStore.setState({ draft: "", panelOpen: false });
    useAssistantStore.getState().openWithDraft("このモデルは向いていますか?");
    const st = useAssistantStore.getState();
    expect(st.draft).toBe("このモデルは向いていますか?");
    expect(st.panelOpen).toBe(true);
    expect(api.assistantAsk).not.toHaveBeenCalled();
  });

  it("送信時に、選択中・導入済みのモデルをファミリー ID だけで添える(カタログ未取得なら添えない)", async () => {
    const { useModelsStore } = await import("./models-store");
    const { useEngineStore } = await import("./engine-store");

    await useAssistantStore.getState().send("q", defaultParams("sdxl"));
    expect(api.assistantAsk.mock.calls[0][3]).toBeUndefined();

    const L = { ja: "x", en: "x" };
    const fam = {
      id: "checkpoint-sdxl",
      category: "checkpoint",
      name: L,
      description: L,
      usedBy: [],
      tags: [],
      variants: [
        {
          id: "b",
          label: L,
          baseModel: "sdxl",
          recommended: true,
          source: { url: "https://huggingface.co/a/b/resolve/c/m", repo: "a/b" },
          sizeBytes: 1,
          sha256: "0".repeat(64),
          destDir: "checkpoints",
          fileName: "sd_xl_base_1.0.safetensors",
          license: { name: "l", url: "https://x", commercialUse: true },
          requiresConsent: false,
        },
      ],
    };
    useModelsStore.setState({
      view: { catalog: { schemaVersion: 2, families: [fam] }, installed: [], modelsDir: null, modelsDirSource: "unknown", freeBytes: null },
    } as never);
    useEngineStore.setState({
      models: { checkpoints: ["sd_xl_base_1.0.safetensors", "secret-private.safetensors"], unets: [], ggufUnets: [] },
    } as never);
    useAssistantStore.setState({ requestId: null });

    const params = { ...defaultParams("sdxl"), model: { kind: "sdxl" as const, name: "secret-private.safetensors", clip: [], vae: null } };
    await useAssistantStore.getState().send("q2", params);
    const sent = api.assistantAsk.mock.calls[1][3];
    expect(sent).toEqual({ familyId: null, installedFamilies: ["checkpoint-sdxl"] });
    expect(JSON.stringify(sent)).not.toContain("secret-private");
  });
});
