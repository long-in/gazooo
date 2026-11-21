"use client";

import { useState } from "react";
import { toast } from "sonner";

import { SecretInput } from "@/components/users/secret-input";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { toAppError } from "@/lib/tauri/errors";
import {
  isWeakPin,
  PASSPHRASE_MIN,
  PIN_MAX,
  PIN_MIN,
} from "@/lib/users/logic";
import { useUserStore } from "@/stores/user-store";

type Kind = "pin" | "passphrase";

/**
 * 設定 > その他: ユーザーのロック(docs/36 §7.1)。既定は無効。操作はすぐ反映し、[保存] ボタンの対象にしない。
 * 変更・解除には、いまのロックの入力が必要。
 */
export function LockSection() {
  const t = useT();
  const errorMessage = useErrorMessage();
  const current = useUserStore((s) => s.current);
  const [editing, setEditing] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [kind, setKind] = useState<Kind>("pin");
  const [pinLength, setPinLength] = useState(4);
  const [secret, setSecret] = useState("");
  const [again, setAgain] = useState("");
  const [currentSecret, setCurrentSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!current) return null;
  const locked = current.authKind !== "none";

  function reset() {
    setEditing(false);
    setRemoving(false);
    setSecret("");
    setAgain("");
    setCurrentSecret("");
    setError(null);
  }

  async function attempt(op: () => Promise<void>, done: string) {
    setBusy(true);
    setError(null);
    try {
      await op();
      toast.success(done);
      reset();
    } catch (e) {
      const err = toAppError(e);
      setError(
        err.code === "AUTH_FAILED" ? t("user.secret.wrong") : errorMessage(err),
      );
    } finally {
      setBusy(false);
    }
  }

  const store = () => useUserStore.getState();
  const valid =
    kind === "pin"
      ? secret.length === pinLength
      : secret.length >= PASSPHRASE_MIN;
  const matches = secret === again;
  const needsCurrent = locked;
  const canRegister =
    !busy && valid && matches && (!needsCurrent || currentSecret.length > 0);

  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-medium">{t("settings.lock.title")}</h2>
      <div className="flex items-start gap-3">
        <Switch
          id="lock-enabled"
          checked={locked || editing}
          disabled={busy}
          onCheckedChange={(on) => {
            reset();
            if (on) setEditing(true);
            else if (locked) setRemoving(true);
          }}
        />
        <div className="flex flex-col gap-0.5">
          <Label htmlFor="lock-enabled">{t("settings.lock.enable")}</Label>
          <p className="text-xs text-muted-foreground">
            {locked
              ? current.authKind === "pin"
                ? t("settings.lock.statePin")
                : t("settings.lock.statePassphrase")
              : t("settings.lock.hint")}
          </p>
        </div>
      </div>

      {removing && (
        <div className="flex max-w-sm flex-col gap-2 rounded-md border p-3">
          <Label htmlFor="lock-remove-current">
            {t("settings.lock.currentValue")}
          </Label>
          <SecretInput
            id="lock-remove-current"
            value={currentSecret}
            onChange={setCurrentSecret}
            numeric={current.authKind === "pin"}
            autoFocus
          />
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <div className="flex gap-2">
            <Button
              variant="destructive"
              disabled={busy || !currentSecret}
              onClick={() =>
                void attempt(
                  () => store().clearLock(currentSecret),
                  t("settings.lock.removed"),
                )
              }
            >
              {t("settings.lock.remove")}
            </Button>
            <Button variant="ghost" onClick={reset}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      )}

      {locked && !editing && !removing && (
        <div>
          <Button variant="outline" onClick={() => setEditing(true)}>
            {t("settings.lock.change")}
          </Button>
        </div>
      )}

      {editing && (
        <div className="flex max-w-sm flex-col gap-3 rounded-md border p-3">
          {needsCurrent && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="lock-current">
                {t("settings.lock.currentValue")}
              </Label>
              <SecretInput
                id="lock-current"
                value={currentSecret}
                onChange={setCurrentSecret}
                numeric={current.authKind === "pin"}
              />
            </div>
          )}
          <fieldset className="flex flex-col gap-1.5">
            <legend className="mb-1 text-sm">{t("settings.lock.method")}</legend>
            {(["pin", "passphrase"] as const).map((k) => (
              <label key={k} className="flex items-center gap-2 text-sm">
                <input
                  type="radio"
                  name="lock-kind"
                  checked={kind === k}
                  onChange={() => {
                    setKind(k);
                    setSecret("");
                    setAgain("");
                  }}
                />
                {k === "pin"
                  ? t("settings.lock.methodPin")
                  : t("settings.lock.methodPassphrase")}
              </label>
            ))}
          </fieldset>
          {kind === "pin" && (
            <div className="flex items-center gap-2">
              <Label htmlFor="lock-pin-length">
                {t("settings.lock.pinLength")}
              </Label>
              <select
                id="lock-pin-length"
                value={pinLength}
                className="h-(--ctl-h) rounded-md border border-input bg-input-bg px-2 text-sm"
                onChange={(e) => {
                  setPinLength(Number(e.target.value));
                  setSecret("");
                  setAgain("");
                }}
              >
                {Array.from(
                  { length: PIN_MAX - PIN_MIN + 1 },
                  (_, i) => PIN_MIN + i,
                ).map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </div>
          )}
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="lock-new">
              {kind === "pin"
                ? t("settings.lock.newPin")
                : t("settings.lock.newPassphrase")}
            </Label>
            <SecretInput
              id="lock-new"
              value={secret}
              onChange={(v) =>
                setSecret(kind === "pin" ? v.slice(0, pinLength) : v)
              }
              numeric={kind === "pin"}
            />
            {kind === "passphrase" && (
              <p className="text-xs text-muted-foreground">
                {t("settings.lock.passphraseHint")}
              </p>
            )}
            {kind === "pin" && valid && isWeakPin(secret) && (
              <p className="text-xs text-amber-600 dark:text-amber-400">
                {t("settings.lock.weakPin")}
              </p>
            )}
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="lock-again">{t("settings.lock.again")}</Label>
            <SecretInput
              id="lock-again"
              value={again}
              onChange={(v) =>
                setAgain(kind === "pin" ? v.slice(0, pinLength) : v)
              }
              numeric={kind === "pin"}
            />
            {again.length > 0 && !matches && (
              <p className="text-xs text-destructive">
                {t("settings.lock.mismatch")}
              </p>
            )}
          </div>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <div className="flex gap-2">
            <Button
              disabled={!canRegister}
              onClick={() =>
                void attempt(
                  () =>
                    store().setLock({
                      kind,
                      secret,
                      pinLength: kind === "pin" ? pinLength : undefined,
                      current: needsCurrent ? currentSecret : undefined,
                    }),
                  t("settings.lock.saved"),
                )
              }
            >
              {t("settings.lock.register")}
            </Button>
            <Button variant="ghost" onClick={reset}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      )}

      <p className="text-xs text-muted-foreground">
        {t("settings.lock.notEncrypted")}
      </p>
      <p className="text-xs text-muted-foreground">
        {t("settings.lock.noRecovery")}
      </p>
    </section>
  );
}
