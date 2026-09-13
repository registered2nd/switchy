import { describe, expect, it } from "vitest";
import {
  extractKimiApiKey,
  extractKimiBaseUrl,
  extractKimiModelName,
  getKimiProviderId,
  isKimiOfficialConfig,
  setKimiApiKey,
  setKimiBaseUrl,
  setKimiModelName,
} from "@/utils/providerConfigUtils";
import { KIMI_OFFICIAL_CONFIG } from "@/config/kimiProviderPresets";
import { getKimiCustomTemplate } from "@/config/kimiTemplates";

const THIRD_PARTY = [
  'default_model = "openai/gpt-4o"',
  "",
  "[providers.openai]",
  'type = "openai"',
  'api_key = "sk-first"',
  'base_url = "https://api.example.com/v1"',
  "",
  '[models."openai/gpt-4o"]',
  'provider = "openai"',
  'model = "gpt-4o"',
  "max_context_size = 128000",
  "",
].join("\n");

describe("Kimi TOML utils", () => {
  it("resolves the active provider through the default_model alias", () => {
    expect(getKimiProviderId(THIRD_PARTY)).toBe("openai");
    expect(getKimiProviderId(KIMI_OFFICIAL_CONFIG)).toBe("managed:kimi-code");
  });

  it("reads api_key, base_url and model of the active provider", () => {
    expect(extractKimiApiKey(THIRD_PARTY)).toBe("sk-first");
    expect(extractKimiBaseUrl(THIRD_PARTY)).toBe("https://api.example.com/v1");
    expect(extractKimiModelName(THIRD_PARTY)).toBe("gpt-4o");
    expect(extractKimiModelName(KIMI_OFFICIAL_CONFIG)).toBe("kimi-for-coding");
  });

  it("round-trips api_key inside the provider table only", () => {
    const updated = setKimiApiKey(THIRD_PARTY, "sk-second");
    expect(extractKimiApiKey(updated)).toBe("sk-second");
    expect(updated).not.toContain("sk-first");
    // the model table is untouched
    expect(updated).toMatch(
      /\[models\."openai\/gpt-4o"\]\nprovider = "openai"/,
    );
  });

  it("removes base_url when set to empty and re-adds it inside the provider table", () => {
    const removed = setKimiBaseUrl(THIRD_PARTY, "");
    expect(removed).not.toMatch(/^\s*base_url\s*=/m);
    expect(extractKimiBaseUrl(removed)).toBeUndefined();

    const readded = setKimiBaseUrl(removed, "https://relay.example.com/v1/");
    expect(extractKimiBaseUrl(readded)).toBe("https://relay.example.com/v1/");
    const providerIndex = readded.indexOf("[providers.openai]");
    const modelsIndex = readded.indexOf('[models."openai/gpt-4o"]');
    const baseUrlIndex = readded.indexOf("base_url =");
    expect(baseUrlIndex).toBeGreaterThan(providerIndex);
    expect(baseUrlIndex).toBeLessThan(modelsIndex);
  });

  it("switching the model rewrites default_model and adds a model alias table", () => {
    const updated = setKimiModelName(THIRD_PARTY, "gpt-4.1-mini");
    expect(updated).toMatch(/^default_model = "openai\/gpt-4\.1-mini"$/m);
    expect(updated).toContain('[models."openai/gpt-4.1-mini"]');
    expect(extractKimiModelName(updated)).toBe("gpt-4.1-mini");
    // existing alias table stays available
    expect(updated).toContain('[models."openai/gpt-4o"]');
  });

  it("keeps an existing alias table when switching back to it", () => {
    const away = setKimiModelName(THIRD_PARTY, "gpt-4.1-mini");
    const back = setKimiModelName(away, "gpt-4o");
    expect(back.match(/\[models\."openai\/gpt-4o"\]/g)).toHaveLength(1);
    expect(extractKimiModelName(back)).toBe("gpt-4o");
  });

  it("custom template fields are editable through the helpers", () => {
    const { config } = getKimiCustomTemplate();
    let next = setKimiApiKey(config, "sk-custom");
    next = setKimiBaseUrl(next, "https://api.custom.io/v1");
    next = setKimiModelName(next, "deepseek-chat");
    expect(extractKimiApiKey(next)).toBe("sk-custom");
    expect(extractKimiBaseUrl(next)).toBe("https://api.custom.io/v1");
    expect(extractKimiModelName(next)).toBe("deepseek-chat");
    expect(next).toMatch(/^default_model = "custom\/deepseek-chat"$/m);
  });

  it("detects the managed login config as official", () => {
    expect(isKimiOfficialConfig(KIMI_OFFICIAL_CONFIG)).toBe(true);
    expect(isKimiOfficialConfig(THIRD_PARTY)).toBe(false);
    expect(isKimiOfficialConfig("")).toBe(false);
  });

  it("handles quoted provider ids in table headers", () => {
    const config = [
      'default_model = "managed:kimi-code/k3"',
      "",
      '[providers."managed:kimi-code"]',
      'type = "kimi"',
      'api_key = ""',
      "",
      '[models."managed:kimi-code/k3"]',
      'provider = "managed:kimi-code"',
      'model = "k3"',
    ].join("\n");
    expect(getKimiProviderId(config)).toBe("managed:kimi-code");
    expect(extractKimiModelName(config)).toBe("k3");
    const withUrl = setKimiBaseUrl(config, "https://api.kimi.com/coding/v1");
    expect(extractKimiBaseUrl(withUrl)).toBe("https://api.kimi.com/coding/v1");
  });
});
