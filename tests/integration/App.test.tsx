import { Suspense, type ComponentType } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { providersApi } from "@/lib/api/providers";
import {
  resetProviderState,
  setCurrentProviderId,
  setLiveProviderIds,
  setProviders,
  setSettings,
} from "../msw/state";
import { emitTauriEvent } from "../msw/tauriMocks";

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
  AppRail: ({ activeApp, onSelectApp }: any) => (
    <div data-testid="app-rail">
      <span>{activeApp}</span>
      <button onClick={() => onSelectApp("claude")}>switch-claude</button>
      <button onClick={() => onSelectApp("codex")}>switch-codex</button>
      <button onClick={() => onSelectApp("openclaw")}>switch-openclaw</button>
    </div>
  ),
}));



const renderApp = (AppComponent: ComponentType) => {
  const client = new QueryClient();
  return render(
    <QueryClientProvider client={client}>
      <Suspense fallback={<div data-testid="loading">loading</div>}>
        <AppComponent />
      </Suspense>
    </QueryClientProvider>,
  );
};

describe("App integration with MSW", () => {
  beforeEach(() => {
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

      fireEvent.click(screen.getByText("switch-codex"));
      await waitFor(() =>
        expect(screen.getByTestId("provider-list").textContent).toContain(
          "codex-1",
        ),
      );

      fireEvent.click(screen.getByText("usage"));
      expect(screen.getByTestId("usage-modal")).toBeInTheDocument();
      fireEvent.click(screen.getByText("save-script"));
      fireEvent.click(screen.getByText("close-usage"));

      fireEvent.click(screen.getByText("create"));
      expect(screen.getByTestId("add-provider-dialog")).toBeInTheDocument();
      fireEvent.click(screen.getByText("confirm-add"));
      await waitFor(() =>
        expect(screen.getByTestId("provider-list").textContent).toMatch(
          /New codex Provider/,
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

  it("duplicates openclaw providers with a generated key that avoids live-only ids", async () => {
    setSettings({
      visibleApps: {
        claude: true,
        codex: true,
        gemini: true,
        kimi: true,
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

    const { default: App } = await import("@/App");
    renderApp(App);

    fireEvent.click(screen.getByText("switch-openclaw"));

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

    const { default: App } = await import("@/App");
    renderApp(App);

    fireEvent.click(screen.getByText("switch-openclaw"));

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
