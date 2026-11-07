import { invoke } from "@tauri-apps/api/core";
import type { AppId } from "@/lib/api/types";

export interface LoginIdentity {
  email: string | null;
  account: string | null;
}

/** Cursor CLI and Antigravity CLI accounts, whose providers are only a login. */
export const LOGIN_ONLY_APPS: AppId[] = ["cursor", "antigravity"];

export const isLoginOnlyApp = (appId: AppId) => LOGIN_ONLY_APPS.includes(appId);

export const loginAccountApi = {
  async getIdentity(
    app: AppId,
    providerId: string,
  ): Promise<LoginIdentity | null> {
    return (await invoke("get_login_account_identity", {
      app,
      providerId,
    })) as LoginIdentity | null;
  },
};
