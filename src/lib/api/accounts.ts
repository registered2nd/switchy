import { invoke } from "@tauri-apps/api/core";
import type { SubscriptionQuota } from "@/types/subscription";
import type { AppId } from "./types";

/** A company's plan account or balance monitor (accounts table). */
export interface Account {
  id: string;
  /** Catalog company id. */
  company: string;
  /** A plan runs in programs; a monitor only reads a balance. */
  kind: "plan" | "monitor";
  plan?: string | null;
  region?: string | null;
  name: string;
  /** The key the company issued for the plan or the balance. */
  secret: string;
  /** For a Kimi plan key: the Kimi Code sign-in it belongs to. */
  loginProviderId?: string | null;
  sortIndex?: number | null;
  createdAt: number;
  meta?: Record<string, unknown>;
}

export interface BalancePart {
  name: "granted" | "toppedUp" | "cash" | "voucher" | string;
  amount: number;
}

export interface BalanceReport {
  success: boolean;
  error: string | null;
  currency: string | null;
  balance: number | null;
  parts: BalancePart[];
  spentToday: number | null;
  spentMonth: number | null;
  trackingSince: number | null;
  queriedAt: number;
}

export const accountsApi = {
  async list(): Promise<Account[]> {
    return await invoke("get_accounts");
  },

  async save(account: Account): Promise<void> {
    await invoke("save_account", { account });
  },

  async delete(id: string): Promise<void> {
    await invoke("delete_account", { id });
  },

  async balance(id: string): Promise<BalanceReport> {
    return await invoke("get_account_balance", { id });
  },

  /** A plan account's quota windows, read with its key. */
  async quota(id: string): Promise<SubscriptionQuota> {
    return await invoke("get_account_quota", { id });
  },

  /** "Used in": puts the account into the program and switches it there. */
  async useIn(id: string, app: AppId): Promise<string> {
    return await invoke("use_account_in", { id, app });
  },

  /** Adds an empty sign-in card to a program whose accounts are its own logins. */
  async addSignIn(app: AppId, name?: string): Promise<string> {
    return await invoke("add_sign_in_account", { app, name });
  },

  /** Takes the account out of OpenCode, which holds several at once. */
  async removeFrom(id: string, app: AppId): Promise<void> {
    await invoke("remove_account_from", { id, app });
  },
};
