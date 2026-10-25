import { describe, expect, it } from "vitest";
import type { Provider } from "@/types";
import type { AppId } from "@/lib/api";
import {
  COMPANIES,
  relayPresetNames,
  sidebarEntryOf,
} from "@/config/companies";
import { providerPresets } from "@/config/claudeProviderPresets";
import { codexProviderPresets } from "@/config/codexProviderPresets";
import { geminiProviderPresets } from "@/config/geminiProviderPresets";
import { opencodeProviderPresets } from "@/config/opencodeProviderPresets";
import { kimiProviderPresets } from "@/config/kimiProviderPresets";

const provider = (
  settingsConfig: Record<string, unknown>,
  extra: Partial<Provider> = {},
): Provider => ({
  id: "p",
  name: "p",
  settingsConfig,
  ...extra,
});

const PRESET_NAMES: Partial<Record<AppId, string[]>> = {
  claude: providerPresets.map((p) => p.name),
  codex: codexProviderPresets.map((p) => p.name),
  gemini: geminiProviderPresets.map((p) => p.name),
  opencode: opencodeProviderPresets.map((p) => p.name),
  kimi: kimiProviderPresets.map((p) => p.name),
};

describe("companies catalog", () => {
  it("names only presets that exist", () => {
    for (const company of COMPANIES) {
      for (const plan of company.plans ?? []) {
        for (const region of Object.values(plan.regions)) {
          for (const [app, program] of Object.entries(region!.programs)) {
            expect(
              PRESET_NAMES[app as AppId],
              `${company.id}/${plan.id}`,
            ).toContain(program!.preset);
          }
        }
      }
    }
    for (const app of [
      "claude",
      "codex",
      "gemini",
      "opencode",
      "kimi",
    ] as AppId[]) {
      for (const name of relayPresetNames(app)) {
        expect(PRESET_NAMES[app], app).toContain(name);
      }
    }
  });

  it("lists each card under the company whose models it serves", () => {
    expect(sidebarEntryOf("claude", provider({ env: {} }))).toBe("anthropic");
    expect(
      sidebarEntryOf(
        "claude",
        provider({ env: { ANTHROPIC_BASE_URL: "https://www.packyapi.com" } }),
      ),
    ).toBe("anthropic");
    expect(
      sidebarEntryOf(
        "claude",
        provider({
          env: { ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic" },
        }),
      ),
    ).toBe("other");
    expect(
      sidebarEntryOf(
        "claude",
        provider({ env: {} }, { meta: { accountId: "a" } }),
      ),
    ).toBeNull();
    expect(sidebarEntryOf("codex", provider({ auth: {}, config: "" }))).toBe(
      "openai",
    );
    expect(
      sidebarEntryOf(
        "codex",
        provider({
          auth: { OPENAI_API_KEY: "k" },
          config: 'base_url = "https://www.packyapi.com/v1"',
        }),
      ),
    ).toBe("openai");
    expect(sidebarEntryOf("antigravity", provider({ credential: null }))).toBe(
      "google",
    );
    expect(sidebarEntryOf("gemini", provider({ env: {} }))).toBe("google");
    expect(sidebarEntryOf("cursor", provider({ auth: null }))).toBe("cursor");
    expect(
      sidebarEntryOf(
        "opencode",
        provider({ options: { baseURL: "https://openrouter.ai/api/v1" } }),
      ),
    ).toBe("opencode");
    expect(
      sidebarEntryOf("opencode", provider({}, { category: "omo" })),
    ).toBeNull();
    expect(sidebarEntryOf("openclaw", provider({ baseUrl: "x" }))).toBeNull();
  });
});
