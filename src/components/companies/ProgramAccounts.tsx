import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import type { Provider } from "@/types";
import { providersApi, settingsApi, type AppId } from "@/lib/api";
import { useProvidersQuery } from "@/lib/query";
import { useProviderActions } from "@/hooks/useProviderActions";
import { useProxyStatus } from "@/hooks/useProxyStatus";
import { useLastValidValue } from "@/hooks/useLastValidValue";
import { openclawKeys } from "@/hooks/useOpenClaw";
import { extractErrorMessage } from "@/utils/errorUtils";
import {
  useDisableCurrentOmo,
  useDisableCurrentOmoSlim,
} from "@/lib/query/omo";
import { ProviderList } from "@/components/providers/ProviderList";
import { EditProviderDialog } from "@/components/providers/EditProviderDialog";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import UsageScriptModal from "@/components/UsageScriptModal";
import { PROGRAM_NAMES } from "@/config/companies";

interface ProgramAccountsProps {
  appId: AppId;
  /** Which of the program's providers to list. */
  filter: (provider: Provider) => boolean;
  /** Label the cards with the program they run in. */
  showProgram?: boolean;
  renderExtraUsedIn?: (provider: Provider) => ReactNode;
}

/**
 * Some of one program's providers, with everything a card can do: use it,
 * edit, delete, set up its usage badge. A company page lists its accounts in
 * one of these per program.
 */
export function ProgramAccounts({
  appId,
  filter,
  showProgram = true,
  renderExtraUsedIn,
}: ProgramAccountsProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const {
    isRunning: isProxyRunning,
    takeoverStatus,
    status,
  } = useProxyStatus();
  const isTakeover = isProxyRunning && Boolean(takeoverStatus?.[appId]);
  const activeProviderId = useMemo(
    () =>
      status?.active_targets?.find((target) => target.app_type === appId)
        ?.provider_id,
    [status?.active_targets, appId],
  );
  const { data, isLoading } = useProvidersQuery(appId, { isProxyRunning });
  const {
    addProvider,
    updateProvider,
    switchProvider,
    deleteProvider,
    saveUsageScript,
    setAsDefaultModel,
  } = useProviderActions(appId, isProxyRunning);

  const providers = useMemo(() => {
    const all = data?.providers ?? {};
    return Object.fromEntries(
      Object.entries(all).filter(([, provider]) => filter(provider)),
    );
  }, [data, filter]);

  const [editing, setEditing] = useState<Provider | null>(null);
  const [usageProvider, setUsageProvider] = useState<Provider | null>(null);
  const [confirm, setConfirm] = useState<{
    provider: Provider;
    action: "remove" | "delete";
  } | null>(null);
  const disableOmo = useDisableCurrentOmo();
  const disableOmoSlim = useDisableCurrentOmoSlim();
  const shownEditing = useLastValidValue(editing);
  const shownUsage = useLastValidValue(usageProvider);
  const isAdditive = appId === "opencode" || appId === "openclaw";

  const openWebsite = async (url: string) => {
    try {
      await settingsApi.openExternal(url);
    } catch (error) {
      toast.error(
        extractErrorMessage(error) ||
          t("notifications.openLinkFailed", {
            defaultValue: "Failed to open link",
          }),
      );
    }
  };

  const openTerminal = async (provider: Provider) => {
    try {
      const cwd = await settingsApi.pickDirectory();
      if (!cwd) return;
      await providersApi.openTerminal(provider.id, appId, { cwd });
      toast.success(
        t("provider.terminalOpened", { defaultValue: "Terminal opened" }),
      );
    } catch (error) {
      toast.error(
        t("provider.terminalOpenFailed", {
          defaultValue: "Failed to open terminal",
        }) + `: ${extractErrorMessage(error)}`,
      );
    }
  };

  const duplicate = async (provider: Provider) => {
    const copy: Omit<Provider, "id" | "createdAt"> & {
      providerKey?: string;
      addToLive?: boolean;
    } = {
      name: `${provider.name} copy`,
      settingsConfig: structuredClone(provider.settingsConfig),
      websiteUrl: provider.websiteUrl,
      category: provider.category,
      sortIndex:
        provider.sortIndex !== undefined ? provider.sortIndex + 1 : undefined,
      meta: provider.meta ? structuredClone(provider.meta) : undefined,
      icon: provider.icon,
      iconColor: provider.iconColor,
    };
    if (isAdditive) {
      let liveIds: string[];
      try {
        liveIds =
          appId === "opencode"
            ? await queryClient.ensureQueryData({
                queryKey: ["opencodeLiveProviderIds"],
                queryFn: () => providersApi.getOpenCodeLiveProviderIds(),
              })
            : await queryClient.ensureQueryData({
                queryKey: openclawKeys.liveProviderIds,
                queryFn: () => providersApi.getOpenClawLiveProviderIds(),
              });
      } catch (error) {
        const detail = extractErrorMessage(error);
        toast.error(
          t("provider.duplicateLiveIdsLoadFailed", {
            defaultValue:
              "Failed to read provider IDs from config. Please fix the config and try again.",
          }) + (detail ? `: ${detail}` : ""),
        );
        return;
      }
      const taken = new Set([
        ...Object.keys(data?.providers ?? {}),
        ...liveIds,
      ]);
      let key = `${provider.id}-copy`;
      for (let n = 2; taken.has(key); n++) key = `${provider.id}-copy-${n}`;
      copy.providerKey = key;
      copy.addToLive = false;
    }
    await addProvider(copy);
  };

  const confirmAction = async () => {
    if (!confirm) return;
    const { provider, action } = confirm;
    if (action === "remove") {
      await providersApi.removeFromLiveConfig(provider.id, appId);
      await queryClient.invalidateQueries({
        queryKey:
          appId === "opencode"
            ? ["opencodeLiveProviderIds"]
            : openclawKeys.liveProviderIds,
      });
      toast.success(
        t("notifications.removeFromConfigSuccess", {
          defaultValue: "Removed from config",
        }),
        { closeButton: true },
      );
    } else {
      await deleteProvider(provider.id);
    }
    setConfirm(null);
  };

  if (!isLoading && Object.keys(providers).length === 0) return null;

  return (
    <>
      <ProviderList
        providers={providers}
        currentProviderId={data?.currentProviderId ?? ""}
        appId={appId}
        isLoading={isLoading}
        isProxyRunning={isProxyRunning}
        isProxyTakeover={isTakeover}
        activeProviderId={activeProviderId}
        onSwitch={switchProvider}
        onEdit={setEditing}
        onDelete={(provider) => setConfirm({ provider, action: "delete" })}
        onRemoveFromConfig={
          isAdditive
            ? (provider) => setConfirm({ provider, action: "remove" })
            : undefined
        }
        onDisableOmo={
          appId === "opencode" ? () => disableOmo.mutate(undefined) : undefined
        }
        onDisableOmoSlim={
          appId === "opencode"
            ? () => disableOmoSlim.mutate(undefined)
            : undefined
        }
        onDuplicate={duplicate}
        onConfigureUsage={setUsageProvider}
        onOpenWebsite={openWebsite}
        onOpenTerminal={appId === "claude" ? openTerminal : undefined}
        onSetAsDefault={appId === "openclaw" ? setAsDefaultModel : undefined}
        programLabel={showProgram ? PROGRAM_NAMES[appId] : undefined}
        renderExtraUsedIn={renderExtraUsedIn}
      />

      <EditProviderDialog
        open={Boolean(editing)}
        provider={shownEditing}
        onOpenChange={(open) => !open && setEditing(null)}
        onSubmit={async ({ provider, originalId }) => {
          await updateProvider(provider, originalId);
          setEditing(null);
        }}
        appId={appId}
        isProxyTakeover={isTakeover}
      />

      {shownUsage && (
        <UsageScriptModal
          key={shownUsage.id}
          provider={shownUsage}
          appId={appId}
          isOpen={Boolean(usageProvider)}
          onClose={() => setUsageProvider(null)}
          onSave={(script) => {
            if (usageProvider) void saveUsageScript(usageProvider, script);
          }}
        />
      )}

      <ConfirmDialog
        isOpen={Boolean(confirm)}
        title={
          confirm?.action === "remove"
            ? t("confirm.removeProvider")
            : t("confirm.deleteProvider")
        }
        message={
          confirm
            ? t(
                confirm.action === "remove"
                  ? "confirm.removeProviderMessage"
                  : "confirm.deleteProviderMessage",
                { name: confirm.provider.name },
              )
            : ""
        }
        onConfirm={() => void confirmAction()}
        onCancel={() => setConfirm(null)}
      />
    </>
  );
}
