"use client";

import { type ReactNode, useEffect, useState } from "react";
import { toast } from "sonner";

import { reloadAfterStorageChange } from "@/components/storage/migration-dialog";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import type { SettingsScope } from "@/lib/tauri/settings";
import { pickStorageDir, storageChangeImagesDir } from "@/lib/tauri/storage";
import { useMigrationStore } from "@/stores/migration-store";
import { useModelsStore } from "@/stores/models-store";
import { useSettingsStore } from "@/stores/settings-store";

import { ScopeChoice } from "./overridable";

function Card({
  id,
  title,
  note,
  value,
  header,
  children,
}: {
  id: string;
  title: string;
  note?: string;
  value: string;
  header?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-2 rounded-lg border p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Label htmlFor={id}>{title}</Label>
        {header}
      </div>
      {note && <p className="text-xs text-muted-foreground">{note}</p>}
      <Input id={id} readOnly value={value} title={value} />
      {children && <div className="flex flex-wrap items-center gap-2">{children}</div>}
    </div>
  );
}

/**
 * 画像の保存先の変更(全体 / 個別で共通)。[データを移行…] と、ファイルを移さない切り替え。
 */
function useImagesSwitch(scope: SettingsScope) {
  const t = useT();
  const errorMessage = useErrorMessage();
  // 「移行せずに変更」で選んだ保存先(確認待ち)
  const [switchTo, setSwitchTo] = useState<string | null>(null);

  async function run(action: () => Promise<void>) {
    try {
      await action();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const pick = () => void run(async () => setSwitchTo(await pickStorageDir()));
  const confirm = () =>
    void run(async () => {
      const dir = switchTo;
      setSwitchTo(null);
      if (dir === null) return;
      await storageChangeImagesDir(scope, dir);
      await reloadAfterStorageChange(scope === "global" ? "imagesGlobal" : "imagesUser");
      toast.success(t("settings.storage.changed"));
    });

  const dialog = (
    <ConfirmDialog
      open={switchTo !== null}
      title={t("settings.storage.switchTitle")}
      description={`${t(scope === "global" ? "settings.storage.switchDescGlobal" : "settings.storage.switchDesc")} ${switchTo ?? ""}`}
      confirmLabel={t("settings.storage.switchConfirm")}
      cancelLabel={t("preset.cancel")}
      destructive={false}
      onOpenChange={(open) => !open && setSwitchTo(null)}
      onConfirm={confirm}
    />
  );
  return { pick, dialog };
}

const target = (scope: SettingsScope) => (scope === "global" ? "imagesGlobal" : "imagesUser");

/** 移行先を決めずに(全体設定・標準値へ戻す)確認画面を開く。 */
function openReset(scope: SettingsScope) {
  useMigrationStore.getState().open(target(scope));
  void useMigrationStore.getState().choose(null);
}

function UserImagesCard() {
  const t = useT();
  const imagesDir = useSettingsStore((s) => s.settings.imagesDir);
  const imagesRoot = useSettingsStore((s) => s.settings.imagesRoot);
  const { pick, dialog } = useImagesSwitch("user");
  // [個別に設定する] を選んで、まだ移行先を決めていない状態
  const [choosing, setChoosing] = useState(false);
  const own = imagesDir !== null;

  const globalLabel = imagesRoot ? `${imagesRoot} ${t("settings.storage.perUser")}` : t("settings.storage.default");

  return (
    <Card
      id="images-dir"
      title={t("settings.storage.imagesDir")}
      note={t("settings.storage.userNote")}
      value={imagesDir ?? globalLabel}
      header={
        <ScopeChoice
          label={t("settings.storage.imagesDir")}
          overridden={own || choosing}
          onChange={(v) => {
            setChoosing(v && !own);
            if (!v && own) openReset("user");
          }}
        />
      }
    >
      {(own || choosing) && (
        <>
          <Button type="button" onClick={() => useMigrationStore.getState().open("imagesUser")}>
            {t("settings.storage.migrate")}
          </Button>
          <Button type="button" variant="outline" onClick={pick}>
            {t("settings.storage.switchOnly")}
          </Button>
        </>
      )}
      {dialog}
    </Card>
  );
}

function GlobalImagesCard() {
  const t = useT();
  const imagesRoot = useSettingsStore((s) => s.settings.imagesRoot);
  const { pick, dialog } = useImagesSwitch("global");
  return (
    <Card
      id="images-root"
      title={t("settings.storage.imagesRoot")}
      note={t("settings.storage.globalNote")}
      value={imagesRoot ?? t("settings.storage.default")}
    >
      <Button type="button" onClick={() => useMigrationStore.getState().open("imagesGlobal")}>
        {t("settings.storage.migrate")}
      </Button>
      <Button type="button" variant="outline" onClick={pick}>
        {t("settings.storage.switchOnly")}
      </Button>
      <Button type="button" variant="ghost" disabled={imagesRoot === null} onClick={() => openReset("global")}>
        {t("settings.storage.reset")}
      </Button>
      {dialog}
    </Card>
  );
}

function ModelsCard() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const modelsDir = useModelsStore((s) => s.view?.modelsDir ?? null);
  const modelsSource = useModelsStore((s) => s.view?.modelsDirSource);

  useEffect(() => {
    void useModelsStore.getState().load();
  }, []);

  async function changeOnly() {
    try {
      const dir = await pickStorageDir();
      if (dir === null) return;
      await useModelsStore.getState().setDir(dir);
      toast.success(t("settings.storage.changed"));
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  return (
    <Card
      id="models-dir"
      title={t("settings.storage.modelsDir")}
      note={modelsSource === "detected" ? t("models.dir.source.detected") : undefined}
      value={modelsDir ?? t("settings.storage.unknown")}
    >
      <Button type="button" disabled={modelsDir === null} onClick={() => useMigrationStore.getState().open("models")}>
        {t("settings.storage.migrate")}
      </Button>
      <Button type="button" variant="outline" onClick={() => void changeOnly()}>
        {t("settings.storage.switchOnly")}
      </Button>
      {modelsDir === null && <p className="w-full text-xs text-muted-foreground">{t("settings.storage.unknownHint")}</p>}
    </Card>
  );
}

/** 保存先(docs/51・52 §4)。操作はすぐ反映する。 */
export function StorageSection({ scope }: { scope: SettingsScope }) {
  const t = useT();
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.storage.title")}</h2>
      {scope === "global" ? (
        <>
          <GlobalImagesCard />
          <ModelsCard />
        </>
      ) : (
        <UserImagesCard />
      )}
    </section>
  );
}
