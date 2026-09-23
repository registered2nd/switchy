/**
 * Codex config template
 * Default config for a new custom provider
 */

export interface CodexTemplate {
  auth: Record<string, any>;
  config: string;
}

/**
 * Get the Codex custom template
 * @returns Codex template config
 */
export function getCodexCustomTemplate(): CodexTemplate {
  const config = `model_provider = "custom"
model = "gpt-5.4"
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true`;

  return {
    auth: { OPENAI_API_KEY: "" },
    config,
  };
}
