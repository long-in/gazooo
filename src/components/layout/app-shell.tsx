"use client";

import { ArrowRightLeft, BookmarkCheck, Download, Images, type LucideIcon, Moon, Settings, Sparkles, Sun } from "lucide-react";
import { useTheme } from "next-themes";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { type ReactNode, useSyncExternalStore } from "react";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { type MessageKey } from "@/lib/i18n";
import { useT } from "@/lib/i18n/use-t";
import { detectMac, modifierLabel } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";

import { ImageTagDialog } from "@/components/generate/image-tag-dialog";
import { UserMenu } from "@/components/users/user-menu";

import { StatusBar } from "./status-bar";
import { useGlobalShortcuts } from "./use-global-shortcuts";

const NAV_ITEMS: { href: string; labelKey: MessageKey; icon: LucideIcon }[] = [
  { href: "/", labelKey: "nav.generate", icon: Sparkles },
  { href: "/gallery", labelKey: "nav.gallery", icon: Images },
  { href: "/presets", labelKey: "nav.presets", icon: BookmarkCheck },
  { href: "/models", labelKey: "nav.models", icon: Download },
  { href: "/convert", labelKey: "nav.convert", icon: ArrowRightLeft },
  { href: "/settings", labelKey: "nav.settings", icon: Settings },
];

function isActive(pathname: string, href: string) {
  if (href === "/") return pathname === "/";
  return pathname === href || pathname.startsWith(`${href}/`);
}

const subscribe = () => () => {};

const toolButton =
  "relative flex size-9 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-sidebar-accent hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring";

/**
 * アプリの外枠(docs/29 §4): 左のツールバー(アイコンだけ)・上のコンテキストバー・下のステータスバー。
 */
export function AppShell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const t = useT();
  useGlobalShortcuts();
  const { resolvedTheme, setTheme } = useTheme();
  // 表示用の修飾キー(サーバー側の事前レンダリングでは Ctrl 表記で固定し、不一致を避ける)
  const isMac = useSyncExternalStore(subscribe, detectMac, () => false);
  const current = NAV_ITEMS.find((n) => isActive(pathname, n.href));
  // 画面表示だけの切り替え(設定の「テーマ」は変えない)
  const dark = resolvedTheme !== "light";

  return (
    <div className="flex h-screen flex-col bg-background">
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Main" className="flex w-12 shrink-0 flex-col items-center gap-1 border-r bg-sidebar py-2 text-sidebar-foreground">
          <UserMenu />
          <div aria-hidden className="my-1 h-px w-6 bg-border" />
          {NAV_ITEMS.map(({ href, labelKey, icon: Icon }, index) => {
            const active = isActive(pathname, href);
            return (
              <Tooltip key={href}>
                <TooltipTrigger
                  render={
                    <Link
                      href={href}
                      aria-current={active ? "page" : undefined}
                      aria-label={t(labelKey)}
                      className={cn(toolButton, active && "bg-sidebar-accent text-foreground")}
                    />
                  }
                >
                  {active && <span aria-hidden className="absolute -left-1.5 bottom-1.5 top-1.5 w-0.5 rounded-full bg-primary" />}
                  <Icon className="size-[18px]" />
                </TooltipTrigger>
                <TooltipContent side="right">
                  {t(labelKey)} ({modifierLabel(isMac)}
                  {index + 1})
                </TooltipContent>
              </Tooltip>
            );
          })}
          <div className="mt-auto">
            <Tooltip>
              <TooltipTrigger
                render={<button type="button" aria-label={t("shell.toggleTheme")} onClick={() => setTheme(dark ? "light" : "dark")} className={toolButton} />}
              >
                {dark ? <Sun className="size-[18px]" /> : <Moon className="size-[18px]" />}
              </TooltipTrigger>
              <TooltipContent side="right">{t("shell.toggleTheme")}</TooltipContent>
            </Tooltip>
          </div>
        </nav>
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-9 shrink-0 items-center gap-3 border-b bg-panel-header px-3">
            <span className="text-xs font-semibold uppercase tracking-wide">{current ? t(current.labelKey) : t("app.name")}</span>
          </header>
          <main className="min-h-0 flex-1 overflow-y-auto bg-canvas">{children}</main>
          <ImageTagDialog />
        </div>
      </div>
      <StatusBar />
    </div>
  );
}
