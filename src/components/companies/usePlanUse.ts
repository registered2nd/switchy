import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { Provider } from "@/types";
import { providersApi, type AppId } from "@/lib/api";
import { accountsApi, type Account } from "@/lib/api/accounts";
import { accountKeys } from "@/lib/query/accounts";
import { useProvidersQuery } from "@/lib/query";
import { extractErrorMessage } from "@/utils/errorUtils";
import {
  companyById,
  planPrograms,
  planRegion,
  PROGRAM_NAMES,
} from "@/config/companies";

export interface UsedInChoice {
  app: AppId;
  inUse: boolean;
  /** The provider that puts the account into this program, once made. */
  provider?: Provider;
}

/** The provider an account made in a program. */
function linkedProvider(
  providers: Record<string, Provider> | undefined,
  accountId: string,
): Provider | undefined {
  return Object.values(providers ?? {}).find(
    (provider) => provider.meta?.accountId === accountId,
  );
}

/**
 * Where a plan account can run and where it is in use. Putting it into a
 * program and taking it out are the backend's `use_account_in` and
 * `remove_account_from`, the same commands an agent calls.
 */
export function usePlanUse(account: Account) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const company = companyById(account.company);
  const region = planRegion(company, account.plan, account.region);
  const apps = planPrograms(region);

  const claude = useProvidersQuery("claude");
  const opencode = useProvidersQuery("opencode");
  const { data: opencodeLive } = useQuery({
    queryKey: ["opencodeLiveProviderIds"],
    queryFn: () => providersApi.getOpenCodeLiveProviderIds(),
    enabled: apps.includes("opencode"),
  });

  const choices: UsedInChoice[] = apps.map((app) => {
    const data = app === "claude" ? claude.data : opencode.data;
    const provider = linkedProvider(data?.providers, account.id);
    const inUse = provider
      ? app === "opencode"
        ? (opencodeLive ?? []).includes(provider.id)
        : data?.currentProviderId === provider.id
      : false;
    return { app, inUse, provider };
  });

  const refresh = async (app: AppId) => {
    await queryClient.invalidateQueries({ queryKey: ["providers", app] });
    if (app === "opencode") {
      await queryClient.invalidateQueries({
        queryKey: ["opencodeLiveProviderIds"],
      });
    }
    try {
      await providersApi.updateTrayMenu();
    } catch (error) {
      console.error("[accounts] tray refresh failed", error);
    }
  };

  /** Puts the account into the program and switches the program to it. */
  const runIn = async (app: AppId) => {
    try {
      await accountsApi.useIn(account.id, app);
      toast.success(
        t("account.nowUsedIn", {
          name: account.name,
          program: PROGRAM_NAMES[app],
          defaultValue: "{{name}} is now used in {{program}}",
        }),
        { closeButton: true },
      );
    } catch (error) {
      toast.error(extractErrorMessage(error));
    } finally {
      await refresh(app);
    }
  };

  /** Takes the account out of a program that holds several (OpenCode). */
  const stopIn = async (app: AppId) => {
    try {
      await accountsApi.removeFrom(account.id, app);
    } catch (error) {
      toast.error(extractErrorMessage(error));
    } finally {
      await refresh(app);
    }
  };

  /** Deletes the account and what it made in programs. */
  const remove = async () => {
    try {
      await accountsApi.delete(account.id);
    } catch (error) {
      toast.error(extractErrorMessage(error));
    } finally {
      await queryClient.invalidateQueries({ queryKey: accountKeys.all });
      for (const app of apps) await refresh(app);
    }
  };

  return { company, region, choices, runIn, stopIn, remove };
}
