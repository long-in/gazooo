"use client";

import { Button } from "@/components/ui/button";
import { useT } from "@/lib/i18n/use-t";

/** 画面の描画中に起きた想定外のエラーを受け、アプリ全体を白画面にしない(docs/05-ui-design.md §6.1)。 */
export default function ErrorBoundary({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
  const t = useT();
  return (
    <div role="alert" className="mx-auto flex max-w-xl flex-col gap-4 p-10">
      <h1 className="text-2xl font-semibold">{t("error.boundary.title")}</h1>
      <p className="text-sm text-muted-foreground">{t("error.boundary.desc")}</p>
      <pre className="max-h-40 overflow-auto rounded-lg border bg-muted/40 p-3 text-xs">{error.message}</pre>
      <div>
        <Button type="button" onClick={reset}>
          {t("error.boundary.retry")}
        </Button>
      </div>
    </div>
  );
}
