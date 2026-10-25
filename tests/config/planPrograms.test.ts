import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { COMPANIES, SIGN_IN_TEMPLATES } from "@/config/companies";
import { providerPresets } from "@/config/claudeProviderPresets";
import { opencodeProviderPresets } from "@/config/opencodeProviderPresets";

const FILE = path.resolve(__dirname, "../../src/config/planPrograms.json");
const SIGN_INS = path.resolve(
  __dirname,
  "../../src/config/signInTemplates.json",
);

/**
 * What each plan puts into each program, built from the catalog and the
 * programs' presets: the backend makes a plan account's providers from it.
 */
function buildPlanPrograms() {
  const out: Record<string, unknown> = {};
  for (const company of COMPANIES) {
    for (const plan of company.plans ?? []) {
      for (const [region, def] of Object.entries(plan.regions)) {
        const programs: Record<string, unknown> = {};
        for (const [app, program] of Object.entries(def!.programs)) {
          if (app === "claude") {
            const preset = providerPresets.find(
              (p) => p.name === program!.preset,
            )!;
            const settings = structuredClone(preset.settingsConfig) as any;
            delete settings.env.ANTHROPIC_AUTH_TOKEN;
            delete settings.env.ANTHROPIC_API_KEY;
            if (program!.baseUrl)
              settings.env.ANTHROPIC_BASE_URL = program!.baseUrl;
            programs.claude = {
              keyField: preset.apiKeyField ?? "ANTHROPIC_AUTH_TOKEN",
              apiFormat: preset.apiFormat ?? null,
              websiteUrl: preset.websiteUrl,
              category: preset.category ?? null,
              icon: preset.icon ?? null,
              iconColor: preset.iconColor ?? null,
              settingsConfig: settings,
            };
          } else if (app === "opencode") {
            const preset = opencodeProviderPresets.find(
              (p) => p.name === program!.preset,
            )!;
            const settings = structuredClone(preset.settingsConfig) as any;
            delete settings.options?.apiKey;
            if (program!.baseUrl) settings.options.baseURL = program!.baseUrl;
            programs.opencode = {
              keyField: "apiKey",
              apiFormat: null,
              websiteUrl: preset.websiteUrl,
              category: preset.category ?? null,
              icon: preset.icon ?? null,
              iconColor: preset.iconColor ?? null,
              settingsConfig: settings,
            };
          }
        }
        out[`${company.id}/${plan.id}/${region}`] = {
          companyName: company.name,
          quotaUrl: def!.quotaUrl ?? null,
          programs,
        };
      }
    }
  }
  return out;
}

describe("planPrograms.json", () => {
  it("matches the catalog and the presets", () => {
    const built = buildPlanPrograms();
    if (process.env.UPDATE_PLAN_PROGRAMS) {
      writeFileSync(FILE, JSON.stringify(built, null, 2) + "\n");
    }
    // Out of step: run with UPDATE_PLAN_PROGRAMS=1 to rewrite it.
    expect(JSON.parse(readFileSync(FILE, "utf8"))).toEqual(built);
  });

  it("signInTemplates.json matches the catalog", () => {
    if (process.env.UPDATE_PLAN_PROGRAMS) {
      writeFileSync(
        SIGN_INS,
        JSON.stringify(SIGN_IN_TEMPLATES, null, 2) + "\n",
      );
    }
    expect(JSON.parse(readFileSync(SIGN_INS, "utf8"))).toEqual(
      SIGN_IN_TEMPLATES,
    );
  });
});
