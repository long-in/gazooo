"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { promptEditState } from "@/lib/assistant/prompt-edit";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import {
  assistantPromptGet,
  assistantPromptReset,
  assistantPromptSet,
  MAX_SYSTEM_PROMPT_CHARS,
  type PromptState,
} from "@/lib/tauri/assistant";
import { toAppError } from "@/lib/tauri/errors";

/** システムプロンプトの編集・保存・初期値に戻す(docs/11 §12)。操作した時点で即時に保存する。 */
export function AssistantPromptEditor() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [state, setState] = useState<PromptState | null>(null);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmReset, setConfirmReset] = useState(false);

  useEffect(() => {
    void assistantPromptGet()
      .then((s) => {
        setState(s);
        setText(s.text);
      })
      .catch((e) => toast.error(errorMessage(toAppError(e))));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!state) return null;

  const edit = promptEditState(text, state.text, state.customized);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(async () => {
      const r = await assistantPromptSet(text);
      setState({ ...state, text, customized: r.customized });
      toast.success(t("settings.assistant.prompt.saved"));
    });

  const reset = () =>
    run(async () => {
      const r = await assistantPromptReset();
      setState({ ...state, text: r.text, defaultText: r.text, customized: false });
      setText(r.text);
      toast.success(t("settings.assistant.prompt.resetDone"));
    });

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <Label htmlFor="assistant-system-prompt">{t("settings.assistant.prompt.title")}</Label>
        <Badge variant={state.customized ? "default" : "secondary"}>
          {state.customized ? t("settings.assistant.prompt.customized") : t("settings.assistant.prompt.default")}
        </Badge>
        {edit.dirty && <span className="text-xs text-muted-foreground">{t("settings.assistant.prompt.unsaved")}</span>}
      </div>
      <p className="text-xs text-muted-foreground">{t("settings.assistant.prompt.desc")}</p>
      <p className="text-xs text-muted-foreground">{t("settings.assistant.prompt.warn")}</p>
      <p className="text-xs text-muted-foreground">{t("settings.assistant.prompt.privacy")}</p>
      {state.customized && <p className="text-xs text-muted-foreground">{t("settings.assistant.prompt.customHint")}</p>}
      <Textarea
        id="assistant-system-prompt"
        className="min-h-48 resize-y font-mono text-xs"
        rows={12}
        spellCheck={false}
        value={text}
        aria-invalid={edit.empty || edit.tooLong}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="flex items-center gap-3">
        <span className={edit.tooLong ? "text-xs text-destructive" : "text-xs text-muted-foreground"}>
          {edit.length.toLocaleString()} / {MAX_SYSTEM_PROMPT_CHARS.toLocaleString()}
          {edit.empty && ` ・ ${t("settings.assistant.prompt.empty")}`}
          {edit.tooLong && ` ・ ${t("settings.assistant.prompt.tooLong")}`}
        </span>
        <div className="ml-auto flex gap-2">
          <Button onClick={save} disabled={busy || !edit.canSave}>
            {t("settings.assistant.prompt.save")}
          </Button>
          <Button variant="outline" onClick={() => setConfirmReset(true)} disabled={busy || !edit.canReset}>
            {t("settings.assistant.prompt.reset")}
          </Button>
        </div>
      </div>
      <details className="text-xs text-muted-foreground">
        <summary className="cursor-pointer">{t("settings.assistant.prompt.guide")}</summary>
        <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded-md border p-2">{state.modelGuide}</pre>
      </details>
      <ConfirmDialog
        open={confirmReset}
        title={t("settings.assistant.prompt.resetConfirmTitle")}
        description={t("settings.assistant.prompt.resetConfirmDesc")}
        confirmLabel={t("settings.assistant.prompt.reset")}
        cancelLabel={t("common.cancel")}
        onOpenChange={setConfirmReset}
        onConfirm={() => {
          setConfirmReset(false);
          void reset();
        }}
      />
    </div>
  );
}
