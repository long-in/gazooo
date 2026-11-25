"use client";

import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { modelContext, profileFor } from "@/lib/models/profile";
import { toAppError } from "@/lib/tauri/errors";
import type { GenerationParams } from "@/lib/tauri/jobs";
import {
  MAX_TRANSLATE_INPUT_CHARS,
  type TranslateMode,
  translateCancel,
  translateRun,
} from "@/lib/tauri/translate";
import { type ApplyMode, applyTranslation } from "@/lib/translate/apply";
import { useAssistantStore } from "@/stores/assistant-store";
import { useEngineStore } from "@/stores/engine-store";
import { useFormStore } from "@/stores/form-store";
import { useModelsStore } from "@/stores/models-store";
import { useTranslateStore } from "@/stores/translate-store";

const OPEN_KEY = "gazooo.translate.open";

// localStorage が使えないときのための、メモリ上の値
let memOpen: boolean | null = null;

function loadOpen(): boolean {
  if (memOpen !== null) return memOpen;
  try {
    return localStorage.getItem(OPEN_KEY) === "1";
  } catch {
    return false;
  }
}

// 開閉の保存(localStorage)を、ハイドレーションのずれなく読むための購読
const openListeners = new Set<() => void>();
const subscribeOpen = (fn: () => void) => {
  openListeners.add(fn);
  return () => void openListeners.delete(fn);
};

/**
 * 「日本語から翻訳」(docs/34 §2.1)。日本語の入力を、選択中のモデルに合ったプロンプトへ翻訳・校正し、
 * プロンプト欄へ入れる。入力欄の中身はフォームの下書きには含めない。
 */
export function TranslateBox({ params }: { params: GenerationParams }) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const status = useTranslateStore((s) => s.status);
  const assistantStatus = useAssistantStore((s) => s.status);
  const catalog = useModelsStore((s) => s.view?.catalog);
  const models = useEngineStore((s) => s.models);

  const open = useSyncExternalStore(subscribeOpen, loadOpen, () => false);
  const [text, setText] = useState("");
  const [mode, setMode] = useState<ApplyMode>("replace");
  const [requestId, setRequestId] = useState<string | null>(null);
  const [runningMode, setRunningMode] = useState<TranslateMode | null>(null);
  const running = requestId !== null;
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // 設定・AI サポートのキー/同意が変わったら、実行できるかを取り直す
  useEffect(() => {
    void useTranslateStore.getState().loadStatus();
  }, [assistantStatus?.hasKey, assistantStatus?.consent]);

  function toggle() {
    memOpen = !open;
    try {
      localStorage.setItem(OPEN_KEY, open ? "0" : "1");
    } catch {
      // 保存できなくても動作に影響しない
    }
    openListeners.forEach((fn) => fn());
  }

  const familyId = modelContext(catalog, params, models)?.familyId ?? null;
  const keepsJapanese =
    profileFor(catalog, params.model.kind, params.model.name)?.profile
      .promptStyle.language === "multilingual";
  const tooLong = text.length > MAX_TRANSLATE_INPUT_CHARS;
  // ボタンごとの無効理由(docs/34 §9.1)
  const blockedReason = (m: TranslateMode) => {
    const r = status?.[m];
    return r && !r.ready && r.reason && r.reason !== "error" ? r.reason : null;
  };
  const blockedPlain = blockedReason("plain");
  const blockedPolish = blockedReason("polish");
  // 外部へ送るのは、Anthropic のプロバイダを使うとき(翻訳専用モデルの [翻訳のみ] はローカルのみ)
  const sendsToAnthropic = (m: TranslateMode) =>
    status?.provider === "anthropic" &&
    !(m === "plain" && status.mtModel === "dedicated");

  async function run(kind: TranslateMode) {
    const input = text.trim();
    if (input === "" || tooLong || running) return;
    if ((kind === "plain" ? blockedPlain : blockedPolish) !== null) return;
    const id = crypto.randomUUID();
    setRequestId(id);
    setRunningMode(kind);
    try {
      const { text: result, notice } = await translateRun(id, input, kind, {
        kind: params.model.kind,
        familyId,
        reference: params.reference?.enabled === true || params.faceSwap?.enabled === true,
      });
      if (result === null) return; // 中止
      const previous = useFormStore.getState().params.prompt;
      useFormStore
        .getState()
        .patch({ prompt: applyTranslation(previous, result, mode) });
      toast(
        t(
          notice === "polish_failed"
            ? "translate.polishFailed"
            : "translate.done",
        ),
        {
          duration: 8000,
          action: {
            label: t("translate.undo"),
            onClick: () => useFormStore.getState().patch({ prompt: previous }),
          },
        },
      );
    } catch (e) {
      const err = toAppError(e);
      // API エラーは、原因(HTTP ステータス・リクエスト ID。本文は含まない)も併せて出す
      const detail =
        err.code === "ASSISTANT_API_ERROR" ? `(${err.message})` : "";
      toast.error(errorMessage(err) + detail);
      void useTranslateStore.getState().loadStatus();
    } finally {
      if (mounted.current) {
        setRequestId(null);
        setRunningMode(null);
      }
    }
  }

  function cancel() {
    if (requestId) void translateCancel(requestId).catch(() => undefined);
  }

  const Icon = open ? ChevronDownIcon : ChevronRightIcon;

  return (
    <div className="rounded-lg border">
      <button
        type="button"
        className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-left text-sm font-medium"
        aria-expanded={open}
        onClick={toggle}
      >
        <Icon className="size-4" />
        {t("translate.title")}
      </button>
      {open && (
        <div className="flex flex-col gap-2 border-t p-2.5">
          <Textarea
            rows={3}
            value={text}
            disabled={running}
            placeholder={t("translate.placeholder")}
            aria-invalid={tooLong ? true : undefined}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              // 親フォームの Ctrl+Enter(生成)と区別する
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                e.stopPropagation();
                void run("polish");
              }
            }}
          />
          <div className="flex flex-wrap items-center gap-2">
            <div
              className="flex overflow-hidden rounded-md border text-xs"
              role="radiogroup"
            >
              {(["replace", "append"] as const).map((m) => (
                <button
                  key={m}
                  type="button"
                  role="radio"
                  aria-checked={mode === m}
                  disabled={running}
                  className={`px-2 py-1 ${mode === m ? "bg-primary text-primary-foreground" : "bg-background"}`}
                  onClick={() => setMode(m)}
                >
                  {t(
                    m === "replace"
                      ? "translate.mode.replace"
                      : "translate.mode.append",
                  )}
                </button>
              ))}
            </div>
            <div className="ml-auto flex items-center gap-2">
              {running && (
                <Button
                  type="button"
                  size="xs"
                  variant="outline"
                  onClick={cancel}
                >
                  {t("translate.cancel")}
                </Button>
              )}
              {(["plain", "polish"] as const).map((m) => (
                <Button
                  key={m}
                  type="button"
                  size="xs"
                  variant={m === "polish" ? "default" : "outline"}
                  disabled={
                    running ||
                    text.trim() === "" ||
                    tooLong ||
                    (m === "plain" ? blockedPlain : blockedPolish) !== null
                  }
                  onClick={() => void run(m)}
                >
                  {runningMode === m
                    ? t("translate.running")
                    : t(`translate.run.${m}` as const)}
                </Button>
              ))}
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("translate.run.hint")}
          </p>
          {(["plain", "polish"] as const).map((m) => {
            const reason = m === "plain" ? blockedPlain : blockedPolish;
            return (
              reason && (
                <p key={m} className="text-xs text-destructive">
                  {t(`translate.reason.${m}` as const)}:{" "}
                  {t(`translate.reason.${reason}` as const)}
                </p>
              )
            );
          })}
          {keepsJapanese && (
            <p className="text-xs text-muted-foreground">
              {t("translate.keepJa")}
            </p>
          )}
          {status?.mtModel === "dedicated" && (
            <p className="text-xs text-muted-foreground">
              {t("translate.mtNote")}
            </p>
          )}
          {status && (
            <p className="text-xs text-muted-foreground">
              {sendsToAnthropic("plain") || sendsToAnthropic("polish")
                ? t("translate.cost")
                : t("translate.local")}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
