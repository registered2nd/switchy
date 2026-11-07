import { Suspense, type ComponentType } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { providersApi } from "@/lib/api/providers";
import { ThemeProvider } from "@/components/theme-provider";
import {
  getCurrentProviderId,
  listProviders,
  resetProviderState,
  setAccounts,
  setCurrentProviderId,
  setLiveProviderIds,
  setProviders,
  setSettings,
} from "../msw/state";
import { emitTauriEvent } from "../msw/tauriMocks";

// The theme provider follows the system theme through matchMedia, which jsdom lacks.
if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => undefined,
    removeListener: () => undefined,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

const toastSuccessMock = vi.fn();
const toastErrorMock = vi.fn();

vi.mock("sonner", () => ({
  toast: {
    success: (...args: unknown[]) => toastSuccessMock(...args),
    error: (...args: unknown[]) => toastErrorMock(...args),
  },
}));

vi.mock("@/components/providers/ProviderList", () => ({
  ProviderList: ({
    providers,
    currentProviderId,
    onSwitch,
    onEdit,
    onDuplicate,
    onConfigureUsage,
    onOpenWebsite,
    onCreate,
  }: any) => (
    <div>
      <div data-testid="provider-list">{JSON.stringify(providers)}</div>
      <div data-testid="current-provider">{currentProviderId}</div>
      <button onClick={() => onSwitch(providers[currentProviderId])}>
        switch
      </button>
      <button onClick={() => onEdit(providers[currentProviderId])}>edit</button>
      <button onClick={() => onDuplicate(providers[currentProviderId])}>
        duplicate
      </button>
      <button onClick={() => onConfigureUsage(providers[currentProviderId])}>
        usage
      </button>
      <button onClick={() => onOpenWebsite("https://example.com")}>
        open-website
      </button>
      <button onClick={() => onCreate?.()}>create</button>
    </div>
  ),
}));

vi.mock("@/components/providers/AddProviderDialog", () => ({
  AddProviderDialog: ({ open, onOpenChange, onSubmit, appId }: any) =>
    open ? (
      <div data-testid="add-provider-dialog">
        <button
          onClick={() =>
            onSubmit({
              name: `New ${appId} Provider`,
              settingsConfig: {},
              category: "custom",
              sortIndex: 99,
            })
          }
        >
          confirm-add
        </button>
        <button onClick={() => onOpenChange(false)}>close-add</button>
      </div>
    ) : null,
}));

vi.mock("@/components/providers/EditProviderDialog", () => ({
  EditProviderDialog: ({ open, provider, onSubmit, onOpenChange }: any) =>
    open ? (
      <div data-testid="edit-provider-dialog">
        <button
          onClick={() =>
            onSubmit({
              provider: {
                ...provider,
                name: `${provider.name}-edited`,
              },
              originalId: provider.id,
            })
          }
        >
          confirm-edit
        </button>
        <button onClick={() => onOpenChange(false)}>close-edit</button>
      </div>
    ) : null,
}));

vi.mock("@/components/UsageScriptModal", () => ({
  default: ({ isOpen, provider, onSave, onClose }: any) =>
    isOpen ? (
      <div data-testid="usage-modal">
        <span data-testid="usage-provider">{provider?.id}</span>
        <button onClick={() => onSave("script-code")}>save-script</button>
        <button onClick={() => onClose()}>close-usage</button>
      </div>
    ) : null,
}));

vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({ isOpen, onConfirm, onCancel }: any) =>
    isOpen ? (
      <div data-testid="confirm-dialog">
        <button onClick={() => onConfirm()}>confirm-delete</button>
        <button onClick={() => onCancel()}>cancel-delete</button>
      </div>
    ) : null,
}));

vi.mock("@/components/layout/AppRail", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/layout/AppRail")>()),
  AppRail: ({ entries, activeEntry, onSelectEntry }: any) => (
    <div data-testid="app-rail">
      <span data-testid="active-entry">{activeEntry}</span>
      <span data-testid="entries">
        {entries.map((entry: any) => entry.id).join(",")}
      </span>
      {["anthropic", "openai", "zai", "deepseek", "other"].map((id) => (
        <button key={id} onClick={() => onSelectEntry(id)}>
          switch-{id}
        </button>
      ))}
    </div>
  ),
}));

// Menus open on pointer events jsdom does not send: render their items as buttons.
vi.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: any) => <div>{children}</div>,
  DropdownMenuTrigger: ({ children }: any) => <div>{children}</div>,
  DropdownMenuContent: ({ children }: any) => <div>{children}</div>,
  DropdownMenuItem: ({ children, onClick }: any) => (
    <button onClick={onClick}>{children}</button>
  ),
  DropdownMenuLabel: ({ children }: any) => <span>{children}</span>,
  DropdownMenuSeparator: () => null,
}));

const renderApp = (AppComponent: ComponentType) => {
  const client = new QueryClient();
  return render(
    <QueryClientProvider client={client}>
      <ThemeProvider defaultTheme="system" storageKey="switchy-theme">
        <Suspense fallback={<div data-testid="loading">loading</div>}>
          <AppComponent />
        </Suspense>
      </ThemeProvider>
    </QueryClientProvider>,
  );
};

describe("App integration with MSW", () => {
  beforeEach(() => {
    localStorage.clear();
    resetProviderState();
    toastSuccessMock.mockReset();
    toastErrorMock.mockReset();
  });

  it(
    "covers basic provider flows via real hooks",
    { timeout: 15000 },
    async () => {
      const { default: App } = await import("@/App");
      renderApp(App);

      await waitFor(() =>
        expect(screen.getByTestId("provider-list").textContent).toContain(
          "claude-1",
        ),
      );

      fireEvent.click(screen.getByText("switch-openai"));
      await waitFor(() =>
        expect(screen.getByTestId("provider-list").textContent).toContain(
          "codex-1",
        ),
      );

      fireEvent.click(screen.getByText("usage"));
      expect(screen.getByTestId("usage-modal")).toBeInTheDocument();
      fireEvent.click(screen.getByText("save-script"));
      fireEvent.click(screen.getByText("close-usage"));

      fireEvent.click(screen.getByText("ChatGPT subscription (Codex)"));
      await waitFor(() =>
        expect(screen.getByTestId("provider-list").textContent).toMatch(
          /"name":"ChatGPT"/,
        ),
      );

      fireEvent.click(screen.getByText("edit"));
      expect(screen.getByTestId("edit-provider-dialog")).toBeInTheDocument();
      fireEvent.click(screen.getByText("confirm-edit"));
      await waitFor(() =>
        expect(screen.getByTestId("provider-list").textContent).toMatch(
          /-edited/,
        ),
      );

      fireEvent.click(screen.getByText("switch"));
      fireEvent.click(screen.getByText("duplicate"));
      await waitFor(() =>
        expect(screen.getByTestId("provider-list").textContent).toMatch(/copy/),
      );

      fireEvent.click(screen.getByText("open-website"));

      emitTauriEvent("provider-switched", {
        appType: "codex",
        providerId: "codex-2",
      });

      expect(toastErrorMock).not.toHaveBeenCalled();
      expect(toastSuccessMock).toHaveBeenCalled();
    },
  );

  it("puts a plan account into Claude Code from its card", async () => {
    setAccounts([
      {
        id: "acc-1",
        company: "zai",
        kind: "plan",
        plan: "coding",
        region: "intl",
        name: "work",
        secret: "zk-test",
        createdAt: Date.now(),
      },
    ]);

    const { default: App } = await import("@/App");
    renderApp(App);

    fireEvent.click(screen.getByText("switch-zai"));
    await waitFor(() => expect(screen.getByText("work")).toBeInTheDocument());

    fireEvent.click(screen.getByTitle("Use in Claude Code"));

    await waitFor(() => {
      const linked = Object.values(listProviders("claude")).find(
        (provider) => provider.meta?.accountId === "acc-1",
      );
      expect(linked).toBeDefined();
      expect(getCurrentProviderId("claude")).toBe(linked!.id);
    });
    expect(toastErrorMock).not.toHaveBeenCalled();
  });

  it("lists a pay-as-you-go key run in a program under Other, and a balance monitor under its company", async () => {
    setProviders("claude", {
      "claude-1": {
        id: "claude-1",
        name: "Claude",
        settingsConfig: { env: {} },
        category: "official",
        sortIndex: 0,
        createdAt: Date.now(),
      },
      ds: {
        id: "ds",
        name: "DeepSeek in Claude Code",
        settingsConfig: {
          env: {
            ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic",
            ANTHROPIC_AUTH_TOKEN: "dk",
          },
        },
        category: "cn_official",
        sortIndex: 1,
        createdAt: Date.now(),
      },
    });
    setAccounts([
      {
        id: "mon-1",
        company: "deepseek",
        kind: "monitor",
        region: "intl",
        name: "DeepSeek main",
        secret: "dk",
        createdAt: Date.now(),
      },
    ]);

    const { default: App } = await import("@/App");
    renderApp(App);

    await waitFor(() =>
      expect(screen.getByTestId("entries").textContent).toContain("other"),
    );
    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).not.toContain(
        "DeepSeek in Claude Code",
      ),
    );

    fireEvent.click(screen.getByText("switch-other"));
    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "DeepSeek in Claude Code",
      ),
    );

    fireEvent.click(screen.getByText("switch-deepseek"));
    await waitFor(() =>
      expect(screen.getByText("DeepSeek main")).toBeInTheDocument(),
    );
    await waitFor(() => expect(screen.getByText(/12\.50/)).toBeInTheDocument());
  });

  it("duplicates openclaw providers with a generated key that avoids live-only ids", async () => {
    setSettings({
      visibleApps: {
        claude: true,
        codex: true,
        gemini: true,
        kimi: true,
        cursor: true,
        antigravity: true,
        opencode: true,
        openclaw: true,
      },
    });
    setProviders("openclaw", {
      deepseek: {
        id: "deepseek",
        name: "DeepSeek",
        settingsConfig: {
          baseUrl: "https://api.deepseek.com",
          apiKey: "test-key",
          api: "openai-completions",
          models: [],
        },
        category: "custom",
        sortIndex: 0,
        createdAt: Date.now(),
      },
    });
    setCurrentProviderId("openclaw", "deepseek");
    setLiveProviderIds("openclaw", ["deepseek-copy"]);

    localStorage.setItem("switchy-last-view", "openclaw");
    const { default: App } = await import("@/App");
    renderApp(App);

    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "deepseek",
      ),
    );

    fireEvent.click(screen.getByText("duplicate"));

    await waitFor(() => {
      const providerList = screen.getByTestId("provider-list").textContent;
      expect(providerList).toContain("deepseek-copy-2");
      expect(providerList).toContain("DeepSeek copy");
    });

    expect(toastErrorMock).not.toHaveBeenCalledWith(
      expect.stringContaining("Provider key is required for openclaw"),
    );
  });

  it("shows toast when duplicate cannot load live provider ids", async () => {
    setSettings({
      visibleApps: {
        claude: true,
        codex: true,
        gemini: true,
        kimi: true,
        cursor: true,
        antigravity: true,
        opencode: true,
        openclaw: true,
      },
    });
    setProviders("openclaw", {
      deepseek: {
        id: "deepseek",
        name: "DeepSeek",
        settingsConfig: {
          baseUrl: "https://api.deepseek.com",
          apiKey: "test-key",
          api: "openai-completions",
          models: [],
        },
        category: "custom",
        sortIndex: 0,
        createdAt: Date.now(),
      },
    });
    setCurrentProviderId("openclaw", "deepseek");

    const liveIdsSpy = vi
      .spyOn(providersApi, "getOpenClawLiveProviderIds")
      .mockRejectedValueOnce(new Error("broken config"));

    localStorage.setItem("switchy-last-view", "openclaw");
    const { default: App } = await import("@/App");
    renderApp(App);

    await waitFor(() =>
      expect(screen.getByTestId("provider-list").textContent).toContain(
        "deepseek",
      ),
    );

    fireEvent.click(screen.getByText("duplicate"));

    await waitFor(() => {
      expect(toastErrorMock).toHaveBeenCalledWith(
        expect.stringContaining("Failed to read provider IDs from config"),
      );
    });

    expect(screen.getByTestId("provider-list").textContent).not.toContain(
      "deepseek-copy",
    );

    liveIdsSpy.mockRestore();
  });
});
