import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Provider } from "@/types";
import { providersApi, type AppId } from "@/lib/api";
import { useProvidersQuery, useSettingsQuery } from "@/lib/query";
import { useAccountsQuery } from "@/lib/query/accounts";
import {
  COMPANIES,
  OTHER_COMPANY,
  sidebarEntryOf,
  type SidebarEntryId,
} from "@/config/companies";

export interface SidebarEntry {
  id: SidebarEntryId;
  name: string;
  icon: string;
  /** One of its accounts is in use in a program. */
  inUse: boolean;
}

const APPS = [
  "claude",
  "codex",
  "gemini",
  "kimi",
  "cursor",
  "antigravity",
  "opencode",
] as const satisfies readonly AppId[];

/**
 * The sidebar: the companies shown by default and every other one that holds
 * an account, in catalog order, less the ones hidden in Settings; then
 * "Other" when a card belongs to no company.
 */
export function useSidebarEntries(otherName: string): SidebarEntry[] {
  const { data: settings } = useSettingsQuery();
  const { data: accounts = [] } = useAccountsQuery();
  const queries = {
    claude: useProvidersQuery("claude"),
    codex: useProvidersQuery("codex"),
    gemini: useProvidersQuery("gemini"),
    kimi: useProvidersQuery("kimi"),
    cursor: useProvidersQuery("cursor"),
    antigravity: useProvidersQuery("antigravity"),
    opencode: useProvidersQuery("opencode"),
  } as const;
  const { data: opencodeLive = [] } = useQuery({
    queryKey: ["opencodeLiveProviderIds"],
    queryFn: () => providersApi.getOpenCodeLiveProviderIds(),
  });
  const hidden = settings?.hiddenCompanies ?? [];

  const signature = APPS.map(
    (app) =>
      `${app}:${queries[app].data?.currentProviderId}:${Object.keys(queries[app].data?.providers ?? {}).join(",")}`,
  ).join("|");

  return useMemo(() => {
    const holding = new Set<string>();
    const inUse = new Set<string>();
    const accountCompany = new Map(accounts.map((a) => [a.id, a.company]));
    for (const account of accounts) holding.add(account.company);

    for (const app of APPS) {
      const data = queries[app].data;
      for (const provider of Object.values(
        data?.providers ?? {},
      ) as Provider[]) {
        const entry = provider.meta?.accountId
          ? accountCompany.get(provider.meta.accountId)
          : sidebarEntryOf(app, provider);
        if (!entry) continue;
        holding.add(entry);
        const live =
          app === "opencode"
            ? opencodeLive.includes(provider.id)
            : data?.currentProviderId === provider.id;
        if (live) inUse.add(entry);
      }
    }

    const entries: SidebarEntry[] = COMPANIES.filter(
      (company) =>
        (company.shownByDefault || holding.has(company.id)) &&
        !hidden.includes(company.id),
    ).map((company) => ({
      id: company.id,
      name: company.name,
      icon: company.icon,
      inUse: inUse.has(company.id),
    }));
    if (holding.has(OTHER_COMPANY)) {
      entries.push({
        id: OTHER_COMPANY,
        name: otherName,
        icon: "",
        inUse: inUse.has(OTHER_COMPANY),
      });
    }
    return entries;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    signature,
    accounts,
    hidden.join(","),
    opencodeLive.join(","),
    otherName,
  ]);
}
