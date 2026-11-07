import { invoke } from "@tauri-apps/api/core";

export interface CodexIdentity {
  accountId: string | null;
  email: string | null;
  planType: string | null;
  /** `last_refresh` as unix seconds; 0 when unknown. */
  lastRefresh: number;
}

export const codexAccountApi = {
  async getIdentity(providerId: string): Promise<CodexIdentity | null> {
    return (await invoke("get_codex_account_identity", {
      providerId,
    })) as CodexIdentity | null;
  },
};
