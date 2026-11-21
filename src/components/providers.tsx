"use client";

import { ThemeProvider, useTheme } from "next-themes";
import { type ReactNode, useEffect } from "react";

import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useLanguage } from "@/lib/i18n/use-t";
import {
  onAssistantDelta,
  onAssistantDone,
  onAssistantError,
  onAssistantSuggestion,
  onEngineLog,
  onEngineInstallLog,
  onEngineInstallState,
  onEngineProcState,
  onEngineStatus,
  onLlmInstall,
  onLlmState,
  onTranslateMtInstall,
  onTranslateMtState,
  onImageCreated,
  onImageDeleted,
  onJobProgress,
  onJobUpdated,
  onModelsDone,
  onModelsProgress,
  onQueueState,
} from "@/lib/tauri/events";
import { toAppError } from "@/lib/tauri/errors";
import { useAssistantStore } from "@/stores/assistant-store";
import { useEngineInstallStore } from "@/stores/engine-install-store";
import { useEngineProcessStore } from "@/stores/engine-process-store";
import { useEngineStore } from "@/stores/engine-store";
import { useGalleryStore } from "@/stores/gallery-store";
import { useDebugStore } from "@/stores/debug-store";
import { useLlmStore } from "@/stores/llm-store";
import { useTranslateMtStore } from "@/stores/translate-mt-store";
import { useStripStore } from "@/stores/strip-store";
import { usePresetStore } from "@/stores/preset-store";
import { useJobStore } from "@/stores/job-store";
import { useModelsStore } from "@/stores/models-store";
import { useSettingsStore } from "@/stores/settings-store";
import { useUiStore } from "@/stores/ui-store";

/** DB 上の設定(正)をテーマ・言語・lang 属性へ反映する。 */
function SettingsBootstrap() {
  const load = useSettingsStore((s) => s.load);
  const theme = useSettingsStore((s) => s.settings.theme);
  const loaded = useSettingsStore((s) => s.loaded);
  const language = useLanguage();
  const { setTheme } = useTheme();
  const density = useUiStore((s) => s.density);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    setTheme(theme);
  }, [theme, setTheme]);

  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);

  // 画面の状態(パネルの開閉・幅・密度)を設定から 1 回だけ取り込む(docs/29 §6)
  useEffect(() => {
    if (loaded && !useUiStore.getState().hydrated)
      useUiStore
        .getState()
        .hydrate(useSettingsStore.getState().settings.uiState);
  }, [loaded]);

  useEffect(() => {
    document.documentElement.dataset.density = density;
  }, [density]);

  return null;
}

/** エンジン状態の初期取得と、Rust 側からの変化通知の購読(アプリ全体で 1 回だけ)。 */
function EngineBootstrap() {
  const applyStatus = useEngineStore((s) => s.applyStatus);
  const refresh = useEngineStore((s) => s.refresh);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void onEngineStatus(applyStatus).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    void refresh();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [applyStatus, refresh]);

  return null;
}

/** プリセット一覧の初期取得(生成フォームの適用メニューと管理画面で共有する)。 */
function PresetBootstrap() {
  const load = usePresetStore((s) => s.load);
  useEffect(() => {
    void load();
  }, [load]);
  return null;
}

/** ジョブ・進捗・生成画像のイベント購読と、初期のジョブ一覧の取得。 */
function JobBootstrap() {
  const { load, applyUpdated, applyProgress } = useJobStore.getState();

  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];
    // 購読を先に張ってから一覧を取得する(取得中の更新を取りこぼさない。重複は id で畳み込まれる)
    void Promise.all([
      onJobUpdated(applyUpdated),
      onJobProgress(applyProgress),
      onQueueState(({ paused }) =>
        useJobStore.getState().applyQueueState(paused),
      ),
      // 生成画像: ギャラリーに反映する
      onImageCreated((image) => {
        useGalleryStore.getState().applyCreated(image);
        useStripStore.getState().applyCreated(image);
      }),
      onImageDeleted(({ ids }) => {
        useGalleryStore.getState().applyDeleted(ids);
        useStripStore.getState().applyDeleted(ids);
      }),
    ]).then((fns) => {
      if (cancelled) fns.forEach((fn) => fn());
      else unlisteners.push(...fns);
    });
    void load();
    void useJobStore.getState().loadQueueState();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [load, applyUpdated, applyProgress]);

  return null;
}

/** AI サポートの応答イベントの購読(画面を移動しても取りこぼさないよう、アプリ全体で 1 回だけ)。 */
function AssistantBootstrap() {
  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];
    const st = () => useAssistantStore.getState();
    void Promise.all([
      onAssistantDelta((p) => st().onDelta(p.requestId, p.text)),
      onAssistantSuggestion((p) =>
        st().onSuggestion(
          p.requestId,
          p.index,
          p.summary,
          p.changes,
          p.model ?? undefined,
        ),
      ),
      onAssistantDone((p) => st().onDone(p.requestId)),
      onAssistantError((p) =>
        st().onError(
          p.requestId,
          toAppError({ code: p.code, message: p.message }),
        ),
      ),
    ]).then((fns) => {
      if (cancelled) fns.forEach((fn) => fn());
      else unlisteners.push(...fns);
    });
    void st().loadStatus();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);
  return null;
}

/** ローカル LLM の状態・導入の進捗の購読(画面を移動しても取りこぼさないよう、アプリ全体で 1 回だけ)。 */
function LlmBootstrap() {
  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];
    const st = () => useLlmStore.getState();
    void Promise.all([
      onLlmState((p) => st().applyState(p.state, p.message)),
      onLlmInstall((s) => st().applyInstall(s)),
      onTranslateMtState((p) =>
        useTranslateMtStore.getState().applyState(p.state, p.message),
      ),
      onTranslateMtInstall((s) =>
        useTranslateMtStore.getState().applyInstall(s),
      ),
    ]).then((fns) => {
      if (cancelled) fns.forEach((fn) => fn());
      else unlisteners.push(...fns);
    });
    void st().load();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);
  return null;
}

/** デバッグログの状態(ステータスバーの表示用)を起動時に 1 回取得する。 */
function DebugBootstrap() {
  useEffect(() => {
    void useDebugStore.getState().load();
  }, []);
  return null;
}

/** ComfyUI の起動管理の状態・ログの購読(画面を移動しても取りこぼさないよう、アプリ全体で 1 回だけ)。 */
function EngineProcessBootstrap() {
  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];
    const st = () => useEngineProcessStore.getState();
    void Promise.all([
      onEngineProcState((s) => st().applyState(s)),
      onEngineLog((p) => st().appendLog(p.line)),
    ]).then((fns) => {
      if (cancelled) fns.forEach((fn) => fn());
      else unlisteners.push(...fns);
    });
    void st().load();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);
  return null;
}

/** ComfyUI のアプリ内インストールの状態・ログの購読(画面を移動しても取りこぼさないよう、アプリ全体で 1 回だけ)。 */
function EngineInstallBootstrap() {
  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];
    const st = () => useEngineInstallStore.getState();
    void Promise.all([
      onEngineInstallState((s) => st().applyState(s)),
      onEngineInstallLog((p) => st().appendLog(p.line)),
    ]).then((fns) => {
      if (cancelled) fns.forEach((fn) => fn());
      else unlisteners.push(...fns);
    });
    void st().load();
    void st().loadLogs();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);
  return null;
}

/**
 * モデル取得の進捗・完了イベントの購読(画面を移動しても取りこぼさないよう、アプリ全体で 1 回だけ)。
 * 完了すると、生成画面で選べるモデル一覧も取り直される(models-store)。
 */
function ModelsBootstrap() {
  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];
    const { applyDownload, loadDownloads } = useModelsStore.getState();
    void Promise.all([
      onModelsProgress(applyDownload),
      onModelsDone(applyDownload),
    ]).then((fns) => {
      if (cancelled) fns.forEach((fn) => fn());
      else unlisteners.push(...fns);
    });
    void loadDownloads();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);
  return null;
}

export function Providers({ children }: { children: ReactNode }) {
  return (
    <ThemeProvider attribute="class" defaultTheme="dark" enableSystem>
      <TooltipProvider>
        <SettingsBootstrap />
        <EngineBootstrap />
        <JobBootstrap />
        <PresetBootstrap />
        <AssistantBootstrap />
        <LlmBootstrap />
        <DebugBootstrap />
        <ModelsBootstrap />
        <EngineProcessBootstrap />
        <EngineInstallBootstrap />
        {children}
        <Toaster richColors />
      </TooltipProvider>
    </ThemeProvider>
  );
}
