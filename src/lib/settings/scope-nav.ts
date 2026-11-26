import type { SettingsScope } from "@/lib/tauri/settings";

export const TAB_IDS = ["engine", "storage", "assistant", "display", "other"] as const;
export type TabId = (typeof TAB_IDS)[number];

/** 中身のあるタブ(docs/52 §3)。エンジンは全体設定のみ。 */
export function tabsFor(scope: SettingsScope): TabId[] {
  return TAB_IDS.filter((id) => scope === "global" || id !== "engine");
}

/**
 * `?scope=&tab=` を解釈する。省略時は個別設定。個別設定に無いタブ(エンジン)を指すリンクは、全体設定を開く。
 */
export function parseSettingsLocation(
  scope: string | null,
  tab: string | null,
): { scope: SettingsScope; tab: TabId } {
  const wanted = TAB_IDS.find((id) => id === tab);
  const resolved: SettingsScope =
    scope === "global" || scope === "user" ? scope : wanted === "engine" ? "global" : "user";
  const tabs = tabsFor(resolved);
  return { scope: resolved, tab: wanted && tabs.includes(wanted) ? wanted : tabs[0] };
}

export const settingsHref = (scope: SettingsScope, tab: TabId) => `/settings?scope=${scope}&tab=${tab}`;
