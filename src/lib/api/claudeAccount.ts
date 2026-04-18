import { invoke } from "@tauri-apps/api/core";

export interface CapturedIdentity {
  accountUuid: string;
  emailAddress: string;
  capturedAt: number;
}

export type CaptureOutcome =
  | { kind: "captured"; identity: CapturedIdentity }
  | {
      kind: "needsConfirmation";
      existing: CapturedIdentity;
      incoming: CapturedIdentity;
    };

export const claudeAccountApi = {
  async capture(providerId: string, force = false): Promise<CaptureOutcome> {
    return await invoke("capture_claude_account", { providerId, force });
  },
  async clear(providerId: string): Promise<void> {
    await invoke("clear_claude_account", { providerId });
  },
  async getIdentity(providerId: string): Promise<CapturedIdentity | null> {
    return (await invoke("get_captured_claude_identity", { providerId })) as
      | CapturedIdentity
      | null;
  },
};
