"use client";

import { Fragment, type ReactNode } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import type { SettingsGroup, SettingsScope } from "@/lib/tauri/settings";
import { cn } from "@/lib/utils";
import { useSettingsScopeStore } from "@/stores/settings-scope-store";

/** 「全体設定を使う / 個別に設定する」の切り替え(2 択)。 */
export function ScopeChoice({
  label,
  overridden,
  onChange,
}: {
  label: string;
  overridden: boolean;
  onChange: (overridden: boolean) => void;
}) {
  const t = useT();
  return (
    <div role="radiogroup" aria-label={label} className="flex w-fit gap-1 rounded-lg bg-muted p-1">
      {[false, true].map((value) => (
        <button
          key={String(value)}
          type="button"
          role="radio"
          aria-checked={overridden === value}
          onClick={() => overridden !== value && onChange(value)}
          className={cn(
            "rounded-md px-3 py-1 text-xs transition-colors",
            overridden === value ? "bg-background font-medium shadow-sm" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {t(value ? "settings.scope.useOwn" : "settings.scope.useGlobal")}
        </button>
      ))}
    </div>
  );
}

/**
 * 上書きできる項目の組(docs/52 §3)。個別設定では、全体設定を使う間は値を表示するだけにする。
 * 全体設定では、標準値に戻せる。切り替えのたびに中身を作り直して、値を読み直す。
 */
export function Overridable({
  group,
  scope,
  label,
  children,
}: {
  group: SettingsGroup;
  scope: SettingsScope;
  label: string;
  children: ReactNode;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const overridden = useSettingsScopeStore((s) => s.overridden.includes(group));
  const revision = useSettingsScopeStore((s) => s.revision);

  async function attempt(op: () => Promise<void>) {
    try {
      await op();
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  const locked = scope === "user" && !overridden;

  return (
    <div className="flex flex-col gap-2 rounded-lg border p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-sm font-medium">{label}</span>
        {scope === "user" ? (
          <ScopeChoice
            label={label}
            overridden={overridden}
            onChange={(v) => void attempt(() => useSettingsScopeStore.getState().setOverride(group, v))}
          />
        ) : (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => void attempt(() => useSettingsScopeStore.getState().resetGlobal(group))}
          >
            {t("settings.scope.reset")}
          </Button>
        )}
      </div>
      <div inert={locked} className={cn("flex flex-col gap-3", locked && "opacity-60")}>
        <Fragment key={`${scope}-${overridden}-${revision}`}>{children}</Fragment>
      </div>
    </div>
  );
}
