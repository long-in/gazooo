"use client";

import { ArrowLeft, Lock, Pencil, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useErrorMessage, useT } from "@/lib/i18n/use-t";
import { type AppError, toAppError } from "@/lib/tauri/errors";
import type { UserSummary } from "@/lib/tauri/user";
import {
  colorClass,
  formatRemaining,
  lockedUntilFromError,
  lockRemainingMs,
  USER_COLORS,
} from "@/lib/users/logic";
import { cn } from "@/lib/utils";
import { useUserStore } from "@/stores/user-store";

import { PinPad } from "./pin-pad";
import { SecretInput } from "./secret-input";
import { UserAvatar } from "./user-avatar";

/**
 * ユーザーの選択画面(docs/36 §5.1)。ログインするまで本体の画面は出さない。
 * ユーザーが 0 人のときは、最初のユーザーの作成から始める。
 */
export function UserGate() {
  const t = useT();
  const users = useUserStore((s) => s.users);
  const loadError = useUserStore((s) => s.loadError);
  const [selected, setSelected] = useState<UserSummary | null>(null);
  const [creating, setCreating] = useState(false);
  const [manage, setManage] = useState<{
    user: UserSummary;
    mode: "rename" | "delete";
  } | null>(null);
  const errorMessage = useErrorMessage();

  async function enter(user: UserSummary) {
    if (user.authKind !== "none") {
      setSelected(user);
      return;
    }
    try {
      await useUserStore.getState().login(user.id);
    } catch (e) {
      toast.error(errorMessage(toAppError(e)));
    }
  }

  // 一覧の更新で選択中のユーザーの情報(待機など)も追従させる
  const selectedNow = selected
    ? (users.find((u) => u.id === selected.id) ?? selected)
    : null;

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-10 bg-background p-6">
      {selectedNow ? (
        <UnlockPanel user={selectedNow} onBack={() => setSelected(null)} />
      ) : creating || users.length === 0 ? (
        <CreatePanel
          first={users.length === 0}
          onBack={() => setCreating(false)}
        />
      ) : (
        <>
          <h1 className="text-2xl font-medium">{t("user.gate.title")}</h1>
          {loadError && (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage(loadError)}
            </p>
          )}
          <ul className="flex max-w-3xl flex-wrap items-start justify-center gap-6">
            {users.map((u) => (
              <li key={u.id} className="group relative">
                <button
                  type="button"
                  onClick={() => void enter(u)}
                  className="flex w-28 flex-col items-center gap-2 rounded-xl p-2 outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <span className="relative">
                    <UserAvatar
                      name={u.name}
                      color={u.color}
                      className="size-20 text-3xl"
                    />
                    {u.authKind !== "none" && (
                      <span className="absolute -bottom-1 -right-1 flex size-6 items-center justify-center rounded-full border bg-background">
                        <Lock
                          className="size-3.5"
                          aria-label={t("user.gate.locked")}
                        />
                      </span>
                    )}
                  </span>
                  <span className="max-w-full truncate text-sm">{u.name}</span>
                </button>
                <span className="absolute right-0 top-0 flex gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label={t("user.manage.rename")}
                    onClick={() => setManage({ user: u, mode: "rename" })}
                  >
                    <Pencil />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label={t("user.manage.delete")}
                    onClick={() => setManage({ user: u, mode: "delete" })}
                  >
                    <Trash2 />
                  </Button>
                </span>
              </li>
            ))}
            <li>
              <button
                type="button"
                onClick={() => setCreating(true)}
                className="flex w-28 flex-col items-center gap-2 rounded-xl p-2 outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="flex size-20 items-center justify-center rounded-full border-2 border-dashed text-muted-foreground">
                  <Plus className="size-8" />
                </span>
                <span className="text-sm">{t("user.gate.add")}</span>
              </button>
            </li>
          </ul>
        </>
      )}
      {manage && (
        <ManageDialog
          user={manage.user}
          mode={manage.mode}
          onClose={() => setManage(null)}
        />
      )}
    </main>
  );
}

/** ロックの入力(PIN のキーパッド / パスフレーズ)。 */
function UnlockPanel({
  user,
  onBack,
}: {
  user: UserSummary;
  onBack: () => void;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [error, setError] = useState<string | null>(null);
  const [lockedUntil, setLockedUntil] = useState<number | null>(
    user.lockedUntil,
  );
  const [passphrase, setPassphrase] = useState("");
  // 待機中だけ、1 秒ごとに現在時刻を更新してカウントダウンする
  const [now, setNow] = useState(() => Date.now());
  const counting = lockedUntil !== null;
  useEffect(() => {
    if (!counting) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [counting]);
  const remaining = lockRemainingMs(lockedUntil, now);
  const locked = remaining > 0;

  function fail(e: AppError) {
    const until = lockedUntilFromError(e, Date.now());
    if (until) {
      setNow(Date.now());
      setLockedUntil(until);
    }
    setError(
      e.code === "AUTH_FAILED"
        ? t(user.authKind === "pin" ? "user.pin.wrong" : "user.passphrase.wrong")
        : errorMessage(e),
    );
  }

  async function login(secret: string): Promise<boolean> {
    setError(null);
    try {
      await useUserStore.getState().login(user.id, secret);
      return true;
    } catch (e) {
      fail(toAppError(e));
      return false;
    }
  }

  return (
    <div className="flex w-full max-w-xs flex-col items-center gap-6">
      <UserAvatar name={user.name} color={user.color} className="size-20 text-3xl" />
      <h1 className="text-xl font-medium">{user.name}</h1>

      {user.authKind === "pin" && user.pinLength ? (
        <PinPad length={user.pinLength} disabled={locked} onSubmit={login} />
      ) : (
        <form
          className="flex w-full flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (passphrase) void login(passphrase).then((ok) => !ok && setPassphrase(""));
          }}
        >
          <Label htmlFor="gate-passphrase" className="sr-only">
            {t("user.passphrase.label")}
          </Label>
          <SecretInput
            id="gate-passphrase"
            value={passphrase}
            onChange={setPassphrase}
            disabled={locked}
            autoFocus
            placeholder={t("user.passphrase.label")}
          />
          <Button type="submit" disabled={locked || !passphrase}>
            {t("user.gate.login")}
          </Button>
        </form>
      )}

      <div role="alert" className="min-h-10 text-center text-sm text-destructive">
        {locked ? (
          <>
            {t("user.lockout")} {formatRemaining(remaining)}
          </>
        ) : (
          error
        )}
      </div>
      <Button variant="ghost" onClick={onBack}>
        <ArrowLeft />
        {t("user.gate.back")}
      </Button>
    </div>
  );
}

/** ユーザーの作成(名前と色)。作成したらそのままログインする。 */
function CreatePanel({
  first,
  onBack,
}: {
  first: boolean;
  onBack: () => void;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const [name, setName] = useState("");
  const [color, setColor] = useState<string>(USER_COLORS[5]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const user = await useUserStore.getState().create(name, color);
      await useUserStore.getState().login(user.id);
    } catch (e) {
      setError(errorMessage(toAppError(e)));
      setBusy(false);
    }
  }

  return (
    <form
      className="flex w-full max-w-xs flex-col items-center gap-5"
      onSubmit={(e) => {
        e.preventDefault();
        if (name.trim()) void submit();
      }}
    >
      <UserAvatar
        name={name || "?"}
        color={color}
        className="size-20 text-3xl"
      />
      <h1 className="text-xl font-medium">
        {first ? t("user.create.first") : t("user.create.title")}
      </h1>
      <div className="flex w-full flex-col gap-1.5">
        <Label htmlFor="user-name">{t("user.create.name")}</Label>
        <Input
          id="user-name"
          value={name}
          maxLength={24}
          autoFocus
          onChange={(e) => setName(e.target.value)}
        />
      </div>
      <div
        role="radiogroup"
        aria-label={t("user.create.color")}
        className="flex flex-wrap justify-center gap-2"
      >
        {USER_COLORS.map((c) => (
          <button
            key={c}
            type="button"
            role="radio"
            aria-checked={color === c}
            aria-label={c}
            onClick={() => setColor(c)}
            className={cn(
              "size-7 rounded-full outline-none ring-offset-2 ring-offset-background focus-visible:ring-2 focus-visible:ring-ring",
              colorClass(c),
              color === c && "ring-2 ring-foreground",
            )}
          />
        ))}
      </div>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        {!first && (
          <Button type="button" variant="ghost" onClick={onBack}>
            {t("user.gate.back")}
          </Button>
        )}
        <Button type="submit" disabled={busy || !name.trim()}>
          {t("user.create.submit")}
        </Button>
      </div>
    </form>
  );
}

/** 名前の変更・削除(docs/36 §5.5)。ロックありのユーザーは、そのロックの入力が必要。 */
function ManageDialog({
  user,
  mode,
  onClose,
}: {
  user: UserSummary;
  mode: "rename" | "delete";
  onClose: () => void;
}) {
  const t = useT();
  const errorMessage = useErrorMessage();
  const locked = user.authKind !== "none";
  const [name, setName] = useState(user.name);
  const [secret, setSecret] = useState("");
  const [forgot, setForgot] = useState(false);
  const [confirmName, setConfirmName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    setBusy(true);
    setError(null);
    try {
      const store = useUserStore.getState();
      if (mode === "rename") {
        await store.rename(user.id, name, locked ? secret : undefined);
      } else {
        const external = await store.remove(
          user.id,
          forgot ? { confirmName } : { secret: locked ? secret : undefined },
        );
        if (external) toast.info(`${t("user.delete.external")} ${external}`);
      }
      onClose();
    } catch (e) {
      const err = toAppError(e);
      setError(
        err.code === "AUTH_FAILED" ? t("user.secret.wrong") : errorMessage(err),
      );
      setBusy(false);
    }
  }

  const canRun =
    !busy &&
    (mode === "rename"
      ? name.trim().length > 0 && (!locked || secret.length > 0)
      : forgot
        ? confirmName === user.name
        : !locked || secret.length > 0);

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {mode === "rename"
              ? t("user.manage.rename")
              : `${t("user.manage.delete")}: ${user.name}`}
          </DialogTitle>
          {mode === "delete" && (
            <DialogDescription>{t("user.delete.warning")}</DialogDescription>
          )}
        </DialogHeader>
        {mode === "rename" && (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="rename-name">{t("user.create.name")}</Label>
            <Input
              id="rename-name"
              value={name}
              maxLength={24}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
        )}
        {locked && !forgot && (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="manage-secret">
              {user.authKind === "pin"
                ? t("user.pin.label")
                : t("user.passphrase.label")}
            </Label>
            <SecretInput
              id="manage-secret"
              value={secret}
              onChange={setSecret}
              numeric={user.authKind === "pin"}
            />
          </div>
        )}
        {mode === "delete" && forgot && (
          <div className="flex flex-col gap-1.5">
            <p className="text-sm text-destructive">{t("user.delete.forgotWarning")}</p>
            <Label htmlFor="confirm-name">{t("user.delete.typeName")}</Label>
            <Input
              id="confirm-name"
              value={confirmName}
              onChange={(e) => setConfirmName(e.target.value)}
              placeholder={user.name}
            />
          </div>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          {mode === "delete" && locked && !forgot && (
            <Button
              variant="link"
              className="mr-auto px-0"
              onClick={() => setForgot(true)}
            >
              {t("user.delete.forgot")}
            </Button>
          )}
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            variant={mode === "delete" ? "destructive" : "default"}
            disabled={!canRun}
            onClick={() => void run()}
          >
            {mode === "rename" ? t("user.manage.save") : t("user.manage.delete")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
