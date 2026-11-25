import type { AssistantProvider } from "./assistant";
import { invoke } from "./invoke";
import type { LlmStatus } from "./llm";
import type { GenerationParams } from "./jobs";

/**
 * プロンプト翻訳(docs/34-prompt-translate.md、docs/04-ipc-api.md「プロンプト翻訳」)。
 * プロバイダ・モデルは AI サポートとは別設定。キー・同意・ローカル LLM は AI サポートと共有する。
 */
/** 翻訳の種類(docs/34 §9.1)。plain = 校正なし、polish = 翻訳+校正。 */
export type TranslateMode = "plain" | "polish";

/** 翻訳エンジン(docs/34 §9.2)。dedicated = ローカルの翻訳専用モデル(未提供。導入の実装まで選べない)。 */
export type TranslateEngine = "general" | "dedicated";

/** ボタンごとの実行可否。 */
export interface TranslateReadiness {
  /** 今すぐ実行できるか。 */
  ready: boolean;
  /** 実行できない理由(`ready = false` のとき)。 */
  reason:
    | "no_key"
    | "no_consent"
    | "local_not_ready"
    | "mt_not_installed"
    | "error"
    | null;
}

export interface TranslateStatus {
  provider: AssistantProvider;
  model: string;
  mtModel: TranslateEngine;
  /** [翻訳のみ] の実行可否。 */
  plain: TranslateReadiness;
  /** [翻訳+校正] の実行可否。 */
  polish: TranslateReadiness;
}

export interface TranslatePatch {
  provider?: AssistantProvider;
  model?: string;
  mtModel?: TranslateEngine;
}

/** 翻訳先のモデル(フォームの選択中のもの)。 */
export interface TranslateTarget {
  kind: GenerationParams["model"]["kind"] | null;
  /** カタログのファミリー ID(カタログ外なら null)。 */
  familyId: string | null;
  /** 参照画像つき生成か。 */
  reference: boolean;
}

/** 入力の最大文字数(Rust の `MAX_INPUT_CHARS` と同じ)。 */
export const MAX_TRANSLATE_INPUT_CHARS = 2000;

export const translateStatus = () =>
  invoke<TranslateStatus>("translate_status");

export const translateUpdate = (patch: TranslatePatch) =>
  invoke<TranslateStatus>("translate_update", { patch });

export interface TranslateResult {
  /** 翻訳した本文(中断されたら null)。 */
  text: string | null;
  /** `polish_failed`: 校正できず、翻訳のみを返した。 */
  notice: "polish_failed" | null;
}

/** 翻訳した本文を返す(中断されたら `text` が null)。`target` は校正のときに使う。 */
export const translateRun = (
  requestId: string,
  text: string,
  mode: TranslateMode,
  target: TranslateTarget,
) =>
  invoke<TranslateResult>("translate_run", {
    requestId,
    text,
    mode,
    target,
  });

export const translateCancel = (requestId: string) =>
  invoke<void>("translate_cancel", { requestId });

/** 翻訳専用モデル(PLaMo 3 Translate)の導入(docs/34 §9.13)。状態は本体のローカル LLM と同じ形。 */
export const translateMtStatus = () => invoke<LlmStatus>("translate_mt_status");

export const translateMtInstall = () =>
  invoke<{ installId: string }>("translate_mt_install");

export const translateMtInstallCancel = (installId: string) =>
  invoke<void>("translate_mt_install_cancel", { installId });

export const translateMtRemove = () => invoke<LlmStatus>("translate_mt_remove");

/** PLaMo Community License の全文(アプリに同梱。導入前でも読める)。 */
export type LicenseLang = "ja" | "en";

export const translateMtLicense = (lang: LicenseLang) =>
  invoke<{ lang: LicenseLang; text: string }>("translate_mt_license", { lang });
