import { settingsApi } from "@/lib/api";

/**
 * Shared post-change sync: writes the current provider back to each app's live config.
 * Never throws; the caller decides how to notify from the return value.
 */
export async function syncCurrentProvidersLiveSafe(): Promise<{
  ok: boolean;
  error?: Error;
}> {
  try {
    await settingsApi.syncCurrentProvidersLive();
    return { ok: true };
  } catch (err) {
    const error = err instanceof Error ? err : new Error(String(err ?? ""));
    return { ok: false, error };
  }
}
