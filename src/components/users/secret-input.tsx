"use client";

import { Eye, EyeOff } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useT } from "@/lib/i18n/use-t";

/** パスフレーズ・PIN の入力欄(表示/非表示の切り替え付き)。 */
export function SecretInput({
  id,
  value,
  onChange,
  numeric = false,
  disabled,
  autoFocus,
  placeholder,
}: {
  id: string;
  value: string;
  onChange: (v: string) => void;
  /** PIN(数字だけ)のとき true。 */
  numeric?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  placeholder?: string;
}) {
  const t = useT();
  const [shown, setShown] = useState(false);
  return (
    <div className="flex gap-1">
      <Input
        id={id}
        type={shown ? "text" : "password"}
        inputMode={numeric ? "numeric" : "text"}
        autoComplete="off"
        value={value}
        disabled={disabled}
        autoFocus={autoFocus}
        placeholder={placeholder}
        maxLength={128}
        onChange={(e) =>
          onChange(numeric ? e.target.value.replace(/\D/g, "") : e.target.value)
        }
      />
      <Button
        type="button"
        variant="ghost"
        size="icon"
        aria-label={shown ? t("user.secret.hide") : t("user.secret.show")}
        aria-pressed={shown}
        onClick={() => setShown((v) => !v)}
      >
        {shown ? <EyeOff /> : <Eye />}
      </Button>
    </div>
  );
}
