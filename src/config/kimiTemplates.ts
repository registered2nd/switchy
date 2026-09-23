/**
 * Kimi Code config template
 * Default config for a new custom provider
 */

export interface KimiTemplate {
  credentials: Record<string, any> | null;
  config: string;
}

/**
 * Get the Kimi custom template
 *
 * A custom provider uses an API key: `[providers.custom]` plus one model alias,
 * with `default_model` pointing at that alias. The form writes its API key, endpoint and model name back here.
 */
export function getKimiCustomTemplate(): KimiTemplate {
  const config = `default_model = "custom/gpt-4o"

[providers.custom]
type = "openai"
api_key = ""

[models."custom/gpt-4o"]
provider = "custom"
model = "gpt-4o"`;

  return {
    credentials: null,
    config,
  };
}
