import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import UsageFooter from "@/components/UsageFooter";
import type { Provider } from "@/types";
import { server } from "../msw/server";

const planCard: Provider = {
  id: "zai-plan",
  name: "Z.ai GLM Coding Plan",
  settingsConfig: {
    env: {
      ANTHROPIC_BASE_URL: "https://api.z.ai/api/anthropic",
      ANTHROPIC_AUTH_TOKEN: "",
    },
  },
  meta: {
    usage_script: {
      enabled: true,
      language: "javascript",
      code: "",
      templateType: "token_plan",
      codingPlanProvider: "zhipu",
    },
  },
} as Provider;

describe("UsageFooter on a coding-plan card", () => {
  it("says it is loading, then shows why the query failed", async () => {
    let answer: () => void = () => {};
    const answered = new Promise<void>((resolve) => {
      answer = resolve;
    });
    server.use(
      http.post("http://tauri.local/queryProviderUsage", async () => {
        await answered;
        return HttpResponse.json({ success: false, error: "No key" });
      }),
    );

    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    render(
      <QueryClientProvider client={client}>
        <UsageFooter
          provider={planCard}
          providerId={planCard.id}
          appId="claude"
          usageEnabled={true}
          isCurrent={true}
          inline={true}
        />
      </QueryClientProvider>,
    );

    expect(await screen.findByText("common.loading")).toBeInTheDocument();

    answer();

    expect(await screen.findByText("No key")).toBeInTheDocument();
    expect(screen.queryByText("common.loading")).not.toBeInTheDocument();
  });
});
