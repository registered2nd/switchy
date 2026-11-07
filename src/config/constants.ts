// Provider type constants
export const PROVIDER_TYPES = {
  GITHUB_COPILOT: "github_copilot",
} as const;

// Usage script template type constants
export const TEMPLATE_TYPES = {
  CUSTOM: "custom",
  GENERAL: "general",
  NEW_API: "newapi",
  GITHUB_COPILOT: "github_copilot",
  TOKEN_PLAN: "token_plan",
} as const;

export type TemplateType = (typeof TEMPLATE_TYPES)[keyof typeof TEMPLATE_TYPES];
