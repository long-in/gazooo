"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import type { MessageKey } from "@/lib/i18n";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { formatMemory, installPercent, isInstalling } from "@/lib/llm/logic";
import { toAppError } from "@/lib/tauri/errors";
import type { LicenseLang } from "@/lib/tauri/translate";
import { useTranslateMtStore } from "@/stores/translate-mt-store";

const STEP_LABEL: Record<string, MessageKey> = {
  runtime: "llm.install.step.runtime",
  extract: "llm.install.step.extract",
  model: "llm.install.step.model",
  license: "llm.install.step.license",
};

/** ライセンス全文の言語。 */
const LICENSE_LANGS = [
  { lang: "ja", label: "settings.translate.mt.licenseLang.ja" },
  { lang: "en", label: "settings.translate.mt.licenseLang.en" },
] as const;

/**
 * 翻訳専用モデル(PLaMo 3 Translate 8B)の導入・削除(docs/34 §9.13)。
 * PLaMo Community License 第 4 条のため、規約の全文をアプリ内で見られるようにし、「Built with PLaMo」を表示する。
 */
export function TranslateMtSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const mt = useTranslateMtStore((s) => s.status);
  const busy = useTranslateMtStore((s) => s.busy);
  const [consentOpen, setConsentOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const [licenseOpen, setLicenseOpen] = useState(false);

  if (!mt) return null;
  const installing = isInstalling(mt.install);
  const failed = mt.install.status === "failed" ? mt.install.error : null;
  const pct = installPercent(mt.install);
  const step = mt.install.stepId ? STEP_LABEL[mt.install.stepId] : undefined;

  return (
    <div className="flex flex-col gap-2 rounded-md border p-3">
      <div className="flex items-center justify-between gap-2">
        <h3 className="font-medium">{t("settings.translate.mt.title")}</h3>
        <span className="rounded-full bg-muted px-2 py-0.5 text-xs">
          {t("settings.translate.mt.builtWith")}
        </span>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.translate.mt.desc")}
      </p>
      <p className="text-xs text-muted-foreground">
        {t("settings.translate.mt.commercial")}
      </p>

      {!mt.supported ? (
        <p className="text-sm text-muted-foreground">{t("llm.unsupported")}</p>
      ) : installing ? (
        <div className="flex flex-col gap-1">
          <Progress value={pct ?? 0}>
            <span className="text-sm">
              {step ? t(step) : t("llm.install.preparing")}
              {pct !== null ? `  ${pct}%` : ""}
            </span>
          </Progress>
          <div>
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                void useTranslateMtStore.getState().cancelInstall()
              }
            >
              {t("llm.install.cancel")}
            </Button>
          </div>
        </div>
      ) : !mt.installed ? (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <Button onClick={() => setConsentOpen(true)} disabled={busy}>
              {t("settings.translate.mt.install").replace(
                "{size}",
                formatMemory(mt.downloadMb),
              )}
            </Button>
            {mt.install.status === "cancelled" && (
              <span className="text-sm text-muted-foreground">
                {t("llm.install.cancelledHint")}
              </span>
            )}
          </div>
          {failed && (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage({ code: failed.code, message: failed.message })}
            </p>
          )}
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm">
            {t("settings.translate.mt.installed").replace(
              "{model}",
              mt.model ?? "",
            )}
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setLicenseOpen(true)}
          >
            {t("settings.translate.mt.license")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={busy || mt.state === "starting" || mt.state === "ready"}
            onClick={() => setRemoveOpen(true)}
          >
            {t("settings.translate.mt.remove")}
          </Button>
        </div>
      )}

      {consentOpen && (
        <MtConsentDialog
          onOpenChange={setConsentOpen}
          onStart={async () => {
            await useTranslateMtStore.getState().install();
            setConsentOpen(false);
          }}
        />
      )}
      {licenseOpen && <LicenseDialog onClose={() => setLicenseOpen(false)} />}
      <ConfirmDialog
        open={removeOpen}
        title={t("settings.translate.mt.removeTitle")}
        description={t("settings.translate.mt.removeDesc")}
        confirmLabel={t("llm.remove.confirm")}
        cancelLabel={t("common.cancel")}
        onOpenChange={setRemoveOpen}
        onConfirm={() => {
          setRemoveOpen(false);
          void useTranslateMtStore
            .getState()
            .remove()
            .catch((e) => toast.error(errorMessage(toAppError(e))));
        }}
      />
    </div>
  );
}

/** ライセンス全文(日本語 / English)を表示する。 */
function LicenseDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const [lang, setLang] = useState<LicenseLang>("ja");
  const [loaded, setLoaded] = useState<{
    lang: LicenseLang;
    text: string;
  } | null>(null);
  const text = loaded?.lang === lang ? loaded.text : null;

  // 表示する言語が変わったら読み込む
  useEffect(() => {
    let cancelled = false;
    useTranslateMtStore
      .getState()
      .readLicense(lang)
      .then((v) => !cancelled && setLoaded({ lang, text: v }))
      .catch(() => !cancelled && setLoaded({ lang, text: "" }));
    return () => {
      cancelled = true;
    };
  }, [lang]);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("settings.translate.mt.licenseTitle")}</DialogTitle>
          <DialogDescription>
            {t("settings.translate.mt.builtWith")}
          </DialogDescription>
        </DialogHeader>
        <div className="flex gap-2">
          {LICENSE_LANGS.map((f) => (
            <Button
              key={f.lang}
              size="sm"
              variant={lang === f.lang ? "default" : "outline"}
              onClick={() => setLang(f.lang)}
            >
              {t(f.label)}
            </Button>
          ))}
        </div>
        <pre className="max-h-[50vh] overflow-auto rounded bg-muted p-2 text-xs whitespace-pre-wrap">
          {text ?? "…"}
        </pre>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** 導入前に、取得元・サイズ・ライセンスを示して同意を得る(docs/34 §9.13)。 */
function MtConsentDialog({
  onOpenChange,
  onStart,
}: {
  onOpenChange: (open: boolean) => void;
  onStart: () => Promise<void>;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const mt = useTranslateMtStore((s) => s.status);
  const [accepted, setAccepted] = useState(false);
  const [starting, setStarting] = useState(false);
  const [licenseOpen, setLicenseOpen] = useState(false);
  if (!mt) return null;

  async function start() {
    setStarting(true);
    try {
      await onStart();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    } finally {
      setStarting(false);
    }
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("settings.translate.mt.consent.title")}</DialogTitle>
          <DialogDescription>
            {t("settings.translate.mt.consent.body").replace(
              "{size}",
              formatMemory(mt.downloadMb),
            )}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2 text-sm">
          <p className="font-medium">{t("llm.consent.hosts")}</p>
          <ul className="list-disc pl-5">
            <li>github.com(llama.cpp)</li>
            <li>huggingface.co(PLaMo 3 Translate 8B)</li>
          </ul>
          <p className="text-xs text-muted-foreground">
            {t("llm.consent.hostsNote")}
          </p>
          <p className="font-medium">{t("llm.consent.licenses")}</p>
          <ul className="list-disc pl-5">
            <li>llama.cpp — {mt.runtimeLicense}</li>
            <li>PLaMo 3 Translate 8B — {mt.modelLicense}</li>
          </ul>
          <p className="text-xs">{t("settings.translate.mt.commercial")}</p>
          <div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setLicenseOpen(true)}
            >
              {t("settings.translate.mt.license")}
            </Button>
          </div>
          <label className="flex items-center gap-2">
            <Switch checked={accepted} onCheckedChange={setAccepted} />
            {t("settings.translate.mt.consent.accept")}
          </label>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button onClick={() => void start()} disabled={!accepted || starting}>
            {t("llm.consent.start")}
          </Button>
        </DialogFooter>
      </DialogContent>
      {licenseOpen && <LicenseDialog onClose={() => setLicenseOpen(false)} />}
    </Dialog>
  );
}
