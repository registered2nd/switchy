import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, it, expect } from "vitest";
import type { ReactElement } from "react";
import type { AppId } from "@/lib/api";
import type { Provider } from "@/types";
import { ProviderCard } from "@/components/providers/ProviderCard";
import {
  getAccountLocks,
  lockAccountOn,
  setRelayBalance,
} from "../msw/state";

const noop = () => {};

function createProvider(overrides: Partial<Provider> = {}): Provider {
  return {
    id: "provider-1",
    name: "Test Provider",
    settingsConfig: {},
    ...overrides,
  } as Provider;
}

function renderCard(provider: Provider, appId: AppId, isCurrent = true) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const ui: ReactElement = (
    <ProviderCard
      provider={provider}
      isCurrent={isCurrent}
      appId={appId}
      onSwitch={noop}
      onEdit={noop}
      onDelete={noop}
      onConfigureUsage={noop}
      onOpenWebsite={noop}
      onDuplicate={noop}
      isProxyRunning={false}
    />
  );
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

const lockButton = () => screen.findByRole("button", { name: "Lock in" });
const unlockButton = () => screen.findByRole("button", { name: "Unlock" });

describe("ProviderCard lock control", () => {
  it("locks a Claude relay card, which is not an official login", async () => {
    const provider = createProvider({
      id: "aiberm-claude",
      name: "Aiberm",
      settingsConfig: {
        env: {
          ANTHROPIC_BASE_URL: "https://relay.example.com",
          ANTHROPIC_AUTH_TOKEN: "sk-relay",
        },
      },
    });
    renderCard(provider, "claude");

    await userEvent.click(await lockButton());

    await waitFor(() =>
      expect(getAccountLocks()).toEqual({ claude: "aiberm-claude" }),
    );
    expect(await unlockButton()).toBeInTheDocument();
  });

  it("shows a relay card's existing lock and clears it", async () => {
    lockAccountOn("codex", "relay-codex");
    const provider = createProvider({
      id: "relay-codex",
      name: "Relay Codex",
      settingsConfig: { auth: { OPENAI_API_KEY: "sk-relay" } },
    });
    renderCard(provider, "codex");

    await userEvent.click(await unlockButton());

    await waitFor(() => expect(getAccountLocks()).toEqual({}));
    expect(await lockButton()).toBeInTheDocument();
  });

  it("keeps the control on an official Claude login", async () => {
    renderCard(createProvider({ id: "claude-1" }), "claude");

    expect(await lockButton()).toBeInTheDocument();
  });

  it("offers no lock on an app the pool does not rotate", async () => {
    renderCard(
      createProvider({
        id: "gemini-1",
        settingsConfig: { env: { GEMINI_API_KEY: "key" } },
      }),
      "gemini",
    );

    expect(await screen.findByLabelText("common.edit")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Lock in" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Unlock" }),
    ).not.toBeInTheDocument();
  });
});

describe("ProviderCard relay balance", () => {
  const relayCard = (id: string) =>
    createProvider({
      id,
      name: id,
      category: "third_party",
      settingsConfig: {
        env: {
          ANTHROPIC_BASE_URL: "https://relay.example.com",
          ANTHROPIC_AUTH_TOKEN: "sk-relay",
        },
      },
    });

  it("shows the relay's balance in dollars, its own units in the hover", async () => {
    setRelayBalance("kuaiapi", {
      success: true,
      error: null,
      balance: 45.53,
      units: 295.92,
      unitsPerDollar: 6.5,
      source: "usage",
      queriedAt: Date.now(),
    });
    renderCard(relayCard("kuaiapi"), "claude", false);

    const amount = await screen.findByText(/45\.53/);
    expect(screen.getByText("Balance")).toBeInTheDocument();
    expect(amount.parentElement).toHaveAttribute(
      "title",
      expect.stringMatching(/295\.92 of the relay's units, 6\.5 to the dollar/),
    );
  });

  it("says why a relay shows no balance to its key", async () => {
    setRelayBalance("aiberm", {
      success: false,
      error: "This key has no limit of its own",
      balance: null,
      units: null,
      unitsPerDollar: 1,
      source: null,
      queriedAt: Date.now(),
    });
    renderCard(relayCard("aiberm"), "claude", false);

    expect(
      await screen.findByText("This key has no limit of its own"),
    ).toBeInTheDocument();
  });

  it("shows nothing for a relay without a balance call", async () => {
    renderCard(relayCard("plain-relay"), "claude", false);

    expect(await screen.findByLabelText("common.edit")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByText("Balance")).not.toBeInTheDocument(),
    );
  });
});
