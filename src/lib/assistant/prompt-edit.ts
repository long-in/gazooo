import { MAX_SYSTEM_PROMPT_CHARS } from "@/lib/tauri/assistant";

export interface PromptEditState {
  /** 保存済みの文面から変わっているか。 */
  dirty: boolean;
  /** 空(空白のみ)。保存不可。 */
  empty: boolean;
  /** 上限超過。保存不可。 */
  tooLong: boolean;
  length: number;
  canSave: boolean;
  /** 初期値に戻せるか(保存済みがカスタム、または未保存の変更がある)。 */
  canReset: boolean;
}

/** システムプロンプトの編集欄の状態(docs/11 §12.4)。`saved` は保存済み(未設定なら初期値)の文面。 */
export function promptEditState(text: string, saved: string, customized: boolean): PromptEditState {
  const length = Array.from(text).length;
  const dirty = text !== saved;
  const empty = text.trim() === "";
  const tooLong = length > MAX_SYSTEM_PROMPT_CHARS;
  return {
    dirty,
    empty,
    tooLong,
    length,
    canSave: dirty && !empty && !tooLong,
    canReset: customized || dirty,
  };
}
