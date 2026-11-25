"use client";

import { CircleHelp } from "lucide-react";
import type { ReactNode } from "react";
import { useState } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useT } from "@/lib/i18n/use-t";
import { Slider } from "@/components/ui/slider";

/** 項目の説明を開く「?」ボタン。クリック・キーボードで開閉できる(docs/21 §3.1)。 */
export function HelpTip({ label, text }: { label: string; text: string }) {
  const t = useT();
  return (
    <Popover>
      <PopoverTrigger
        type="button"
        aria-label={`${label} ${t("help.aria")}`}
        className="inline-flex size-4 shrink-0 items-center justify-center rounded-full text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
      >
        <CircleHelp className="size-3.5" aria-hidden />
      </PopoverTrigger>
      <PopoverContent>
        <p className="whitespace-pre-line">{text}</p>
      </PopoverContent>
    </Popover>
  );
}

export function Field({
  label,
  htmlFor,
  error,
  children,
  hint,
  help,
  action,
}: {
  label: string;
  htmlFor?: string;
  error?: string | null;
  children: ReactNode;
  hint?: string;
  /** 説明(翻訳済みの文)。指定するとラベルの横に「?」を出す。 */
  help?: string;
  /** ラベルの右端に置く操作(クリア・初期値に戻す など)。 */
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-1.5">
        <Label htmlFor={htmlFor}>{label}</Label>
        {help && <HelpTip label={label} text={help} />}
        {action && <div className="ml-auto flex items-center gap-1">{action}</div>}
      </div>
      {children}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}

/** 入力途中(空文字など)を許容する数値入力。フォーカス中だけ下書きの文字列を表示する。 */
export function NumberInput({
  id,
  value,
  onChange,
  min,
  max,
  step,
  invalid,
  disabled,
}: {
  id?: string;
  value: number;
  onChange: (n: number) => void;
  min?: number;
  max?: number;
  step?: number;
  invalid?: boolean;
  disabled?: boolean;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <Input
      id={id}
      type="number"
      inputMode="decimal"
      value={draft ?? String(value)}
      min={min}
      max={max}
      step={step}
      aria-invalid={invalid || undefined}
      disabled={disabled}
      onChange={(e) => {
        setDraft(e.target.value);
        const n = Number(e.target.value);
        if (e.target.value.trim() !== "" && Number.isFinite(n)) onChange(n);
      }}
      onBlur={() => setDraft(null)}
    />
  );
}

/** スライダー + 数値入力。 */
export function SliderNumber({
  id,
  value,
  onChange,
  min,
  max,
  step,
  invalid,
}: {
  id?: string;
  value: number;
  onChange: (n: number) => void;
  min: number;
  max: number;
  step: number;
  invalid?: boolean;
}) {
  return (
    <div className="flex items-center gap-3">
      <Slider
        className="flex-1"
        value={[Math.min(Math.max(value, min), max)]}
        min={min}
        max={max}
        step={step}
        onValueChange={(v) => onChange(Array.isArray(v) ? v[0] : v)}
      />
      <div className="w-20 shrink-0">
        <NumberInput id={id} value={value} onChange={onChange} min={min} max={max} step={step} invalid={invalid} />
      </div>
    </div>
  );
}
