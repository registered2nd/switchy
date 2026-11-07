import { invoke } from "@tauri-apps/api/core";

export interface BrokenSessionInfo {
  sessionId: string;
  sourcePath: string;
  projectDir: string | null;
  lastModifiedMs: number;
  totalLines: number;
  thinkingBlocks: number;
  emptySignatures: number;
  redactedThinkingBlocks: number;
}

export interface RepairResult {
  sourcePath: string;
  backupPath: string;
  linesBefore: number;
  linesAfter: number;
  thinkingDropped: number;
  redactedThinkingDropped: number;
  parentUuidRewrites: number;
}

export const sessionsApi = {
  async scanBroken(): Promise<BrokenSessionInfo[]> {
    return await invoke("scan_broken_sessions");
  },

  async repairBroken(sourcePath: string): Promise<RepairResult> {
    return await invoke("repair_broken_session", { sourcePath });
  },
};
