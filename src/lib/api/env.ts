import { invoke } from "@tauri-apps/api/core";
import type { EnvConflict, BackupInfo } from "@/types/env";

/**
 * Environment variable API
 */

/**
 * Check an app's environment variable conflicts
 * @param appType app type ("claude" | "codex" | "gemini")
 * @returns environment variable conflicts
 */
export async function checkEnvConflicts(
  appType: string,
): Promise<EnvConflict[]> {
  return invoke<EnvConflict[]>("check_env_conflicts", { app: appType });
}

/**
 * Delete the given environment variables (backed up automatically)
 * @param conflicts conflicts to delete
 * @returns backup info
 */
export async function deleteEnvVars(
  conflicts: EnvConflict[],
): Promise<BackupInfo> {
  return invoke<BackupInfo>("delete_env_vars", { conflicts });
}

/**
 * Restore environment variables from a backup file
 * @param backupPath backup file path
 */
export async function restoreEnvBackup(backupPath: string): Promise<void> {
  return invoke<void>("restore_env_backup", { backupPath });
}

/**
 * Check environment variable conflicts for every app
 * @returns conflicts grouped by app type
 */
export async function checkAllEnvConflicts(): Promise<
  Record<string, EnvConflict[]>
> {
  const apps = ["claude", "codex", "gemini"];
  const results: Record<string, EnvConflict[]> = {};

  await Promise.all(
    apps.map(async (app) => {
      try {
        results[app] = await checkEnvConflicts(app);
      } catch (error) {
        console.error(`Failed to check environment variables for ${app}:`, error);
        results[app] = [];
      }
    }),
  );

  return results;
}
