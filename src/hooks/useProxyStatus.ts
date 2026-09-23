/**
 * Proxy service status hook
 */

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import type {
  ProxyStatus,
  ProxyServerInfo,
  ProxyTakeoverStatus,
} from "@/types/proxy";
import { extractErrorMessage } from "@/utils/errorUtils";

/**
 * Proxy service status
 */
export function useProxyStatus() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  // Query status (auto-polling)
  const { data: status, isLoading } = useQuery({
    queryKey: ["proxyStatus"],
    queryFn: () => invoke<ProxyStatus>("get_proxy_status"),
    // Poll only while the service is running
    refetchInterval: (query) => (query.state.data?.running ? 2000 : false),
    // Keep previous data to avoid flicker
    placeholderData: (previousData) => previousData,
  });

  // Query each app's takeover status
  const { data: takeoverStatus } = useQuery({
    queryKey: ["proxyTakeoverStatus"],
    queryFn: () => invoke<ProxyTakeoverStatus>("get_proxy_takeover_status"),
    placeholderData: (previousData) => previousData,
  });

  // Start the server (master switch: starts the service only, no takeover)
  const startProxyServerMutation = useMutation({
    mutationFn: () => invoke<ProxyServerInfo>("start_proxy_server"),
    onSuccess: (info) => {
      toast.success(
        t("proxy.server.started", {
          address: info.address,
          port: info.port,
          defaultValue: "Proxy service started - {{address}}:{{port}}",
        }),
        { closeButton: true },
      );
      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "Unknown" });
      toast.error(
        t("proxy.server.startFailed", {
          defaultValue: `Failed to start proxy service: ${detail}`,
        }),
      );
    },
  });

  // Stop the server (master switch off: force-restore every taken-over live config)
  const stopWithRestoreMutation = useMutation({
    mutationFn: () => invoke("stop_proxy_with_restore"),
    onSuccess: () => {
      toast.success(
        t("proxy.stoppedWithRestore", {
          defaultValue: "Proxy service stopped, all takeover configs restored",
        }),
        { closeButton: true },
      );
      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
      queryClient.invalidateQueries({ queryKey: ["proxyTakeoverStatus"] });
      // Drop all provider health caches (the backend has cleared the database records)
      queryClient.removeQueries({ queryKey: ["providerHealth"] });
      // Drop all circuit breaker stats caches (breaker state resets once the proxy stops)
      queryClient.removeQueries({ queryKey: ["circuitBreakerStats"] });
      // Note: the failover queue and toggle state are kept, no refresh needed
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "Unknown" });
      toast.error(
        t("proxy.stopWithRestoreFailed", {
          defaultValue: `Stop failed: ${detail}`,
        }),
      );
    },
  });

  // Turn takeover on/off per app
  const setTakeoverForAppMutation = useMutation({
    mutationFn: ({ appType, enabled }: { appType: string; enabled: boolean }) =>
      invoke("set_proxy_takeover_for_app", { appType, enabled }),
    onSuccess: (_data, variables) => {
      const appLabel =
        variables.appType === "claude"
          ? "Claude"
          : variables.appType === "codex"
            ? "Codex"
            : variables.appType === "gemini"
              ? "Gemini"
              : "OpenCode";

      toast.success(
        variables.enabled
          ? t("proxy.takeover.enabled", {
              app: appLabel,
              defaultValue: "{{app}} takeover enabled",
            })
          : t("proxy.takeover.disabled", {
              app: appLabel,
              defaultValue: "{{app}} takeover disabled",
            }),
        { closeButton: true },
      );

      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
      queryClient.invalidateQueries({ queryKey: ["proxyTakeoverStatus"] });
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "Unknown" });
      toast.error(t("proxy.takeover.failed"), {
        description: detail,
        duration: 12000,
        closeButton: true,
      });
    },
  });

  // Switch provider in proxy mode (hot switch)
  const switchProxyProviderMutation = useMutation({
    mutationFn: ({
      appType,
      providerId,
    }: {
      appType: string;
      providerId: string;
    }) => invoke("switch_proxy_provider", { appType, providerId }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "Unknown" });
      toast.error(
        t("proxy.switchFailed", {
          error: detail,
          defaultValue: "Switch failed: {{error}}",
        }),
      );
    },
  });

  // Whether the service is running
  const checkRunning = async () => {
    try {
      return await invoke<boolean>("is_proxy_running");
    } catch {
      return false;
    }
  };

  // Check takeover status
  const checkTakeoverActive = async () => {
    try {
      return await invoke<boolean>("is_live_takeover_active");
    } catch {
      return false;
    }
  };

  return {
    status,
    isLoading,
    isRunning: status?.running || false,
    takeoverStatus,
    isTakeoverActive:
      takeoverStatus?.claude ||
      takeoverStatus?.codex ||
      takeoverStatus?.gemini ||
      false,

    // Start/stop (master switch)
    startProxyServer: startProxyServerMutation.mutateAsync,
    stopWithRestore: stopWithRestoreMutation.mutateAsync,

    // Per-app takeover switch
    setTakeoverForApp: setTakeoverForAppMutation.mutateAsync,

    // Switch provider in proxy mode
    switchProxyProvider: switchProxyProviderMutation.mutateAsync,

    // Status checks
    checkRunning,
    checkTakeoverActive,

    // Loading state
    isStarting: startProxyServerMutation.isPending,
    isStopping: stopWithRestoreMutation.isPending,
    isPending:
      startProxyServerMutation.isPending ||
      stopWithRestoreMutation.isPending ||
      setTakeoverForAppMutation.isPending,
  };
}
