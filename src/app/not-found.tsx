"use client";

import Link from "next/link";

import { buttonVariants } from "@/components/ui/button";
import { useT } from "@/lib/i18n/use-t";

export default function NotFound() {
  const t = useT();
  return (
    <div className="mx-auto flex max-w-xl flex-col gap-4 p-10">
      <h1 className="text-2xl font-semibold">{t("error.notFound.title")}</h1>
      <div>
        <Link href="/" className={buttonVariants()}>
          {t("error.notFound.home")}
        </Link>
      </div>
    </div>
  );
}
