"use client";

import { Delete } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { useT } from "@/lib/i18n/use-t";
import { applyPinKey, pinKeyFromEvent, type PinKey } from "@/lib/users/logic";
import { cn } from "@/lib/utils";

const ROWS = [
  ["1", "2", "3"],
  ["4", "5", "6"],
  ["7", "8", "9"],
] as const;

const keyClass =
  "flex h-14 items-center justify-center rounded-xl border bg-card text-xl font-medium tabular-nums outline-none transition-[transform,background-color] hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40";

/**
 * 金庫のような数字キーパッド(docs/36 §5.3)。クリックでも、キーボードの数字・テンキー・
 * Backspace・Esc・Enter でも入力できる。桁数に達したら自動で送信し、失敗(`false`)なら揺れて全消去する。
 */
export function PinPad({
  length,
  disabled = false,
  onSubmit,
}: {
  length: number;
  disabled?: boolean;
  /** 成功なら `true`。`false` / 例外のときは入力を消して揺らす。 */
  onSubmit: (pin: string) => Promise<boolean>;
}) {
  const t = useT();
  const [value, setValue] = useState("");
  const [pressed, setPressed] = useState<string | null>(null);
  const [shake, setShake] = useState(0);
  const busy = useRef(false);
  const valueRef = useRef("");
  const pressTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );

  const submit = useCallback(
    async (pin: string) => {
      if (busy.current) return;
      busy.current = true;
      let ok = false;
      try {
        ok = await onSubmit(pin);
      } catch {
        ok = false;
      } finally {
        busy.current = false;
      }
      if (!ok) {
        valueRef.current = "";
        setValue("");
        setShake((n) => n + 1);
      }
    },
    [onSubmit],
  );

  const press = useCallback(
    (key: PinKey) => {
      if (disabled || busy.current) return;
      if (key.kind === "enter") {
        if (valueRef.current.length === length) void submit(valueRef.current);
        return;
      }
      const next = applyPinKey(valueRef.current, key, length);
      valueRef.current = next;
      setValue(next);
      if (key.kind === "digit" && next.length === length) void submit(next);
    },
    [disabled, length, submit],
  );

  // キーボード入力。入力欄は持たず、ウィンドウ全体で数字キーを受ける
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const key = pinKeyFromEvent(e.key);
      if (!key) return;
      e.preventDefault();
      setPressed(key.kind === "digit" ? key.digit : key.kind);
      clearTimeout(pressTimer.current);
      pressTimer.current = setTimeout(() => setPressed(null), 120);
      press(key);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      clearTimeout(pressTimer.current);
    };
  }, [press]);

  const down = (id: string) => pressed === id && "scale-95 bg-accent";

  return (
    <div className="flex flex-col items-center gap-6">
      <div
        key={shake}
        role="status"
        aria-live="polite"
        aria-label={`${value.length} / ${length}`}
        className={cn(
          "flex h-4 items-center gap-3",
          shake > 0 && "pin-shake",
        )}
      >
        {Array.from({ length }, (_, i) => (
          <span
            key={i}
            className={cn(
              "size-3 rounded-full border transition-colors",
              i < value.length ? "border-primary bg-primary" : "border-muted-foreground/60",
            )}
          />
        ))}
      </div>

      <div className="grid grid-cols-3 gap-3" role="group" aria-label={t("user.pin.pad")}>
        {ROWS.flat().map((d) => (
          <button
            key={d}
            type="button"
            disabled={disabled}
            aria-label={d}
            className={cn(keyClass, "w-16", down(d))}
            onClick={() => press({ kind: "digit", digit: d })}
          >
            {d}
          </button>
        ))}
        <button
          type="button"
          disabled={disabled}
          aria-label={t("user.pin.clear")}
          className={cn(keyClass, "w-16 text-sm", down("clear"))}
          onClick={() => press({ kind: "clear" })}
        >
          C
        </button>
        <button
          type="button"
          disabled={disabled}
          aria-label="0"
          className={cn(keyClass, "w-16", down("0"))}
          onClick={() => press({ kind: "digit", digit: "0" })}
        >
          0
        </button>
        <button
          type="button"
          disabled={disabled}
          aria-label={t("user.pin.backspace")}
          className={cn(keyClass, "w-16", down("backspace"))}
          onClick={() => press({ kind: "backspace" })}
        >
          <Delete className="size-5" />
        </button>
      </div>
    </div>
  );
}
