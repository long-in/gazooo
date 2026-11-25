"use client";

import { ThemeProvider, useTheme } from "next-themes";
import { toast } from "sonner";
import { type ReactNode, useEffect, useRef } from "react";

import { Toaster } from "@/components/ui/sonner";
import { UserGate } from "@/components/users/user-gate";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useLanguage, useT } from "@/lib/i18n/use-t";
import {
  onAssistantDelta,
  onAssistantDone,
  onAssistantError,
  onAssistantSuggestion,
  onEngineLog,
  onEngineInstallLog,
  onEngineInstallState,
  onEngineProcState,
  onEngineRecovery,
  onEngineStatus,
  onLlmInstall,
  onLlmState,
  onTranslateMtInstall,
  onTranslateMtState,
  onImageCreated,
  onImageDeleted,
  onImageMoved,
  onProjectChanged,
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
import { useProjectStore } from "@/stores/project-store";
import { useJobStore } from "@/stores/job-store";
import { useModelsStore } from "@/stores/models-store";
import { useSettingsStore } from "@/stores/settings-store";
import { useUiStore } from "@/stores/ui-store";
import { applyAccent } from "@/lib/users/logic";
import { useUserStore } from "@/stores/user-store";

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

/**
 * プロジェクト一覧・件数の初期取得と、変化通知の購読(docs/40 §7)。画面状態の取り込み後に、
 * 保存済みの現在のプロジェクトを復元する。生成・削除でも件数が変わるため、画像のイベントでも取り直す。
 */
function ProjectBootstrap() {
  const uiHydrated = useUiStore((s) => s.hydrated);
  const loaded = useProjectStore((s) => s.loaded);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const unlisteners: (() => void)[] = [];
    // 連続した通知(複数枚の生成・まとめて移動)は 1 回の取得にまとめる
    const refresh = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void useProjectStore.getState().load(), 200);
    };
    void Promise.all([
      onProjectChanged(refresh),
      onImageCreated(refresh),
      onImageDeleted(refresh),
    ]).then((fns) => {
      if (cancelled) fns.forEach((fn) => fn());
      else unlisteners.push(...fns);
    });
    void useProjectStore.getState().load();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      unlisteners.forEach((fn) => fn());
    };
  }, []);

  // 一覧と画面状態の両方が揃ってから、現在のプロジェクトを 1 回だけ復元する
  const restored = useRef(false);
  useEffect(() => {
    if (restored.current || !uiHydrated || !loaded) return;
    restored.current = true;
    useProjectStore.getState().restoreCurrent();
  }, [uiHydrated, loaded]);

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
      // 画像の所属の変更(docs/40 §7): 一覧を範囲に合わせる
      onImageMoved(({ ids, projectId }) => {
        useGalleryStore.getState().applyMoved(ids, projectId);
        useStripStore.getState().applyMoved(ids, projectId);
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

/** ComfyUI の自動復旧(docs/47)の通知をトーストで出す。 */
function EngineRecoveryBootstrap() {
  const t = useT();
  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | undefined;
    void onEngineRecovery((e) => {
      if (e.phase === "restarting") toast.info(t("engine.recovery.restarting"));
      else if (e.phase === "recovered")
        toast.success(
          t("engine.recovery.recovered").replace(
            "{count}",
            String(e.requeuedJobs ?? 0),
          ),
        );
      else toast.error(t("engine.recovery.gaveUp"));
    }).then((fn) => {
      if (cancelled) fn();
      else un = fn;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, [t]);
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

/**
 * ログインするまでは、ユーザーの選択画面だけを出し、データを読む処理は一切始めない(docs/36 §5.1)。
 * ユーザーを切り替えるときはウィンドウをリロードするため、各ストアは毎回まっさらな状態で始まる。
 */
/** ログイン中のユーザーの色をアプリのアクセント色へ反映する(docs/38 §3.3)。未ログインは既定の青。 */
function AccentSync() {
  const color = useUserStore((s) =>
    s.phase === "in" ? (s.current?.color ?? null) : null,
  );
  const { resolvedTheme } = useTheme();
  useEffect(() => {
    applyAccent(document.documentElement, color, resolvedTheme === "dark");
  }, [color, resolvedTheme]);
  return null;
}

function UserBoundary({ children }: { children: ReactNode }) {
  const phase = useUserStore((s) => s.phase);

  useEffect(() => {
    void useUserStore.getState().init();
  }, []);

  if (phase === "loading") return null;
  if (phase === "gate") return <UserGate />;
  return (
    <>
      <SettingsBootstrap />
      <EngineBootstrap />
      <JobBootstrap />
      <PresetBootstrap />
      <ProjectBootstrap />
      <AssistantBootstrap />
      <LlmBootstrap />
      <DebugBootstrap />
      <ModelsBootstrap />
      <EngineProcessBootstrap />
      <EngineRecoveryBootstrap />
      <EngineInstallBootstrap />
      {children}
    </>
  );
}

export function Providers({ children }: { children: ReactNode }) {
  return (
    <ThemeProvider attribute="class" defaultTheme="dark" enableSystem>
      <TooltipProvider>
        <AccentSync />
        <UserBoundary>{children}</UserBoundary>
        <Toaster richColors />
      </TooltipProvider>
    </ThemeProvider>
  );
}
