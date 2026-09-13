import { useCallback, useState } from "react";
import { Loader2, Search, Wrench, FileWarning } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  sessionsApi,
  type BrokenSessionInfo,
  type RepairResult,
} from "@/lib/api/sessions";

function formatRelative(ms: number): string {
  if (!ms) return "";
  const ago = Date.now() - ms;
  const min = Math.floor(ago / 60000);
  if (min < 1) return "just now";
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const days = Math.floor(hr / 24);
  return `${days}d ago`;
}

export function SessionRecoveryPanel() {
  const [scanning, setScanning] = useState(false);
  const [repairingPath, setRepairingPath] = useState<string | null>(null);
  const [sessions, setSessions] = useState<BrokenSessionInfo[] | null>(null);
  const [lastRepair, setLastRepair] = useState<RepairResult | null>(null);

  const handleScan = useCallback(async () => {
    setScanning(true);
    setLastRepair(null);
    try {
      const result = await sessionsApi.scanBroken();
      setSessions(result);
      if (result.length === 0) {
        toast.success("No broken sessions found.");
      } else {
        toast.info(`Found ${result.length} session(s) needing repair.`);
      }
    } catch (e) {
      toast.error(`Scan failed: ${e}`);
    } finally {
      setScanning(false);
    }
  }, []);

  const handleRepair = useCallback(async (path: string) => {
    setRepairingPath(path);
    try {
      const result = await sessionsApi.repairBroken(path);
      setLastRepair(result);
      const droppedTotal =
        result.thinkingDropped + result.redactedThinkingDropped;
      toast.success(
        `Repaired: ${droppedTotal} block(s) stripped, ${result.linesBefore} → ${result.linesAfter} lines, ${result.parentUuidRewrites} chain rewrite(s)`,
      );
      const fresh = await sessionsApi.scanBroken();
      setSessions(fresh);
    } catch (e) {
      toast.error(`Repair failed: ${e}`);
    } finally {
      setRepairingPath(null);
    }
  }, []);

  const handleRepairAll = useCallback(async () => {
    if (!sessions || sessions.length === 0) return;
    for (const s of sessions) {
      setRepairingPath(s.sourcePath);
      try {
        await sessionsApi.repairBroken(s.sourcePath);
      } catch (e) {
        toast.error(`Repair of ${s.sessionId.slice(0, 8)} failed: ${e}`);
      }
    }
    setRepairingPath(null);
    try {
      const fresh = await sessionsApi.scanBroken();
      setSessions(fresh);
      toast.success(`Repair-all complete. ${fresh.length} session(s) remain.`);
    } catch (e) {
      toast.error(`Final scan failed: ${e}`);
    }
  }, [sessions]);

  return (
    <section className="space-y-3">
      <header className="space-y-1">
        <div className="flex items-center gap-2">
          <FileWarning className="h-4 w-4 text-amber-500" />
          <h3 className="text-sm font-medium">
            Repair broken Claude Code sessions
          </h3>
        </div>
        <p className="text-xs text-muted-foreground">
          Scans <code>~/.claude/projects</code> for session transcripts
          containing thinking blocks with empty signatures or
          <code> redacted_thinking</code> blocks (produced by relays that don't
          sign properly, e.g. Aiberm-style wrapping relays). Stripping these
          allows the session to resume on Anthropic Official.
        </p>
      </header>

      <div className="flex items-center gap-2">
        <Button
          onClick={handleScan}
          disabled={scanning || repairingPath !== null}
          variant="secondary"
          size="sm"
        >
          {scanning ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />
          ) : (
            <Search className="h-3.5 w-3.5 mr-1.5" />
          )}
          Scan
        </Button>
        {sessions && sessions.length > 0 && (
          <Button
            onClick={handleRepairAll}
            disabled={repairingPath !== null}
            variant="default"
            size="sm"
          >
            {repairingPath !== null ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />
            ) : (
              <Wrench className="h-3.5 w-3.5 mr-1.5" />
            )}
            Repair all
          </Button>
        )}
        {sessions !== null && (
          <span className="text-xs text-muted-foreground ml-auto">
            {sessions.length} affected
          </span>
        )}
      </div>

      {sessions !== null && sessions.length === 0 && (
        <p className="text-xs text-muted-foreground italic">
          No broken sessions found.
        </p>
      )}

      {sessions !== null && sessions.length > 0 && (
        <div className="rounded-md border border-border-default divide-y divide-border-default text-xs overflow-hidden">
          {sessions.map((s) => (
            <div
              key={s.sourcePath}
              className="flex items-center justify-between gap-3 p-2.5"
            >
              <div className="min-w-0 flex-1">
                <p className="truncate font-mono text-[11px]">
                  <span className="text-muted-foreground">
                    {s.sessionId.slice(0, 8)}
                  </span>
                  {" · "}
                  {s.projectDir ?? (
                    <span className="text-muted-foreground italic">
                      (unknown cwd)
                    </span>
                  )}
                </p>
                <p className="text-muted-foreground mt-0.5">
                  {s.totalLines} lines · {s.thinkingBlocks} thinking{" "}
                  {s.emptySignatures > 0 && `(${s.emptySignatures} empty sig)`}
                  {" · "}
                  {s.redactedThinkingBlocks} redacted
                  {" · "}
                  {formatRelative(s.lastModifiedMs)}
                </p>
              </div>
              <Button
                size="sm"
                variant="outline"
                onClick={() => handleRepair(s.sourcePath)}
                disabled={repairingPath !== null}
              >
                {repairingPath === s.sourcePath ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Wrench className="h-3.5 w-3.5" />
                )}
                <span className="ml-1.5">Repair</span>
              </Button>
            </div>
          ))}
        </div>
      )}

      {lastRepair && (
        <div className="rounded-md bg-muted/40 px-3 py-2 text-[11px] text-muted-foreground space-y-0.5">
          <p>
            <span className="font-medium text-foreground">Last repair:</span>{" "}
            {lastRepair.sourcePath}
          </p>
          <p>
            Stripped{" "}
            {lastRepair.thinkingDropped + lastRepair.redactedThinkingDropped}{" "}
            block(s); {lastRepair.linesBefore} → {lastRepair.linesAfter} lines;{" "}
            {lastRepair.parentUuidRewrites} chain rewrite(s).
          </p>
          <p>
            Backup: <code>{lastRepair.backupPath}</code>
          </p>
        </div>
      )}

      <p className="text-[11px] text-muted-foreground italic">
        Quit Claude Code for a session before repairing it. Backups are saved as{" "}
        <code>&lt;name&gt;.bak.jsonl</code>; restore by copying back.
      </p>
    </section>
  );
}
