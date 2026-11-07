import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  ProviderForm,
  type ProviderFormValues,
} from "@/components/providers/forms/ProviderForm";

vi.mock("@/lib/api/config", () => ({
  getCommonConfigSnippet: vi.fn().mockResolvedValue(""),
  setCommonConfigSnippet: vi.fn().mockResolvedValue(undefined),
  extractCommonConfigSnippet: vi.fn().mockResolvedValue(""),
  getClaudeCommonConfigSnippet: vi.fn().mockResolvedValue(""),
  setClaudeCommonConfigSnippet: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@/components/JsonEditor", () => ({
  default: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      value={value}
      onChange={(event) => onChange(event.target.value)}
      aria-label="mock-editor"
    />
  ),
}));

vi.mock("@/components/common/FullScreenPanel", () => ({
  FullScreenPanel: ({ isOpen, children }: any) =>
    isOpen ? <div>{children}</div> : null,
}));

const renderForm = (onSubmit: (values: ProviderFormValues) => void) => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ProviderForm
        appId="kimi"
        submitLabel="submit-kimi"
        onSubmit={onSubmit}
        onCancel={() => {}}
      />
    </QueryClientProvider>,
  );
};

describe("ProviderForm (kimi)", () => {
  it("submits the official preset as { config, credentials: null }", async () => {
    const onSubmit = vi.fn();
    renderForm(onSubmit);

    fireEvent.click(await screen.findByRole("button", { name: /Kimi Code/ }));

    await waitFor(() =>
      expect(screen.getByDisplayValue("Kimi Code")).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("button", { name: "submit-kimi" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const values = onSubmit.mock.calls[0][0] as ProviderFormValues;
    const settings = JSON.parse(values.settingsConfig);
    expect(settings.credentials).toBeNull();
    expect(settings.config).toContain(
      'default_model = "kimi-code/kimi-for-coding"',
    );
    expect(settings.config).toContain('[providers."managed:kimi-code"]');
    expect(values.presetCategory).toBe("official");
    expect(typeof values.meta?.commonConfigEnabled).toBe("boolean");
  });

  it("writes API key, base URL and model into config.toml for a custom provider", async () => {
    const onSubmit = vi.fn();
    renderForm(onSubmit);

    const apiKeyInput =
      (await screen.findByPlaceholderText("provider.namePlaceholder")) &&
      document.getElementById("kimiApiKey");
    fireEvent.change(apiKeyInput as HTMLInputElement, {
      target: { value: "sk-test-123" },
    });
    fireEvent.change(document.getElementById("kimiBaseUrl") as HTMLElement, {
      target: { value: "https://api.example.com/v1" },
    });
    fireEvent.change(screen.getByPlaceholderText("provider.namePlaceholder"), {
      target: { value: "My Relay" },
    });

    fireEvent.click(screen.getByRole("button", { name: "submit-kimi" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const values = onSubmit.mock.calls[0][0] as ProviderFormValues;
    const settings = JSON.parse(values.settingsConfig);
    expect(settings.credentials).toBeNull();
    expect(settings.config).toContain('api_key = "sk-test-123"');
    expect(settings.config).toContain(
      'base_url = "https://api.example.com/v1"',
    );
    expect(settings.config).toMatch(/^default_model = "custom\//m);
  });
});
