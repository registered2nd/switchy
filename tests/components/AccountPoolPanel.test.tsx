import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { AccountPoolPanel } from "@/components/proxy/AccountPoolPanel";
import type { CodexTerminalUpdate } from "@/lib/api/settings";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const api = vi.hoisted(() => ({
  getAccountPoolConfig: vi.fn(),
  getProxyRouteStatus: vi.fn(),
  getCodexTerminalUpdates: vi.fn(),
  refreshCodexTerminal: vi.fn(),
  cancelCodexTerminalRefresh: vi.fn(),
  setAccountPoolConfig: vi.fn(),
}));

vi.mock("@/lib/api/settings", () => ({ settingsApi: api }));

const terminal = (
  overrides: Partial<CodexTerminalUpdate>,
): CodexTerminalUpdate => ({
  platform: "Windows",
  socket: "ws://127.0.0.1:5000",
  cwd: "C:\\Projects\\demo",
  serverVersion: "codex-cli 0.162.1",
  installedVersion: "codex-cli 0.163.0",
  cliChanged: true,
  catalogChanged: false,
  refreshOnExit: true,
  managed: true,
  queued: false,
  waiting: false,
  ...overrides,
});

describe("AccountPoolPanel Codex terminals", () => {
  beforeEach(() => {
    api.getAccountPoolConfig.mockResolvedValue({
      enabled: false,
      thresholdPercent: 98,
      requireProxy: false,
      blockedExitCountries: [],
      keepWarmEnabled: false,
      keepWarmIntervalMinutes: 60,
      codexSharedSession: true,
    });
    api.getProxyRouteStatus.mockResolvedValue([]);
    api.refreshCodexTerminal.mockResolvedValue(undefined);
    api.cancelCodexTerminalRefresh.mockResolvedValue(undefined);
  });

  it("refreshes a terminal with one click and shows it waiting for the reply", async () => {
    api.getCodexTerminalUpdates
      .mockResolvedValueOnce([terminal({})])
      .mockResolvedValue([terminal({ queued: true, waiting: true })]);
    render(<AccountPoolPanel />);

    fireEvent.click(
      await screen.findByRole("button", {
        name: "proxy.accountPool.codexRefresh",
      }),
    );

    await waitFor(() =>
      expect(api.refreshCodexTerminal).toHaveBeenCalledWith(
        "ws://127.0.0.1:5000",
      ),
    );
    expect(
      await screen.findByText("proxy.accountPool.codexRefreshQueued"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("proxy.accountPool.codexRefreshWaiting"),
    ).toBeInTheDocument();
    expect(api.cancelCodexTerminalRefresh).not.toHaveBeenCalled();
  });

  it("cancels a requested refresh", async () => {
    api.getCodexTerminalUpdates.mockResolvedValue([
      terminal({ queued: true }),
    ]);
    render(<AccountPoolPanel />);

    fireEvent.click(
      await screen.findByRole("button", {
        name: "proxy.accountPool.codexCancelRefresh",
      }),
    );

    await waitFor(() =>
      expect(api.cancelCodexTerminalRefresh).toHaveBeenCalledWith(
        "ws://127.0.0.1:5000",
      ),
    );
    expect(api.refreshCodexTerminal).not.toHaveBeenCalled();
  });

  it("keeps refresh on exit and the manual instruction for older terminals", async () => {
    api.getCodexTerminalUpdates.mockResolvedValue([
      terminal({ socket: "ws://127.0.0.1:1", managed: false, queued: true }),
      terminal({ socket: "ws://127.0.0.1:2", refreshOnExit: false }),
    ]);
    render(<AccountPoolPanel />);

    expect(
      await screen.findByText("proxy.accountPool.codexRefreshOnExitQueued"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("proxy.accountPool.codexLegacyRefresh"),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("button")).toHaveLength(1);
    expect(
      screen.queryByText("proxy.accountPool.codexRefreshQueued"),
    ).not.toBeInTheDocument();
  });
});
