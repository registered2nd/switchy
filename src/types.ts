export type ProviderCategory =
  | "official" // Official
  | "cn_official" // Open-source official (formerly "Chinese official")
  | "cloud_provider" // Cloud provider (AWS Bedrock etc.)
  | "aggregator" // Aggregator site
  | "third_party" // Third-party provider
  | "custom" // Custom
  | "omo" // Oh My OpenCode
  | "omo-slim"; // Oh My OpenCode Slim

export interface Provider {
  id: string;
  name: string;
  settingsConfig: Record<string, any>; // App config object: settings.json for Claude; { auth, config } for Codex
  websiteUrl?: string;
  // Provider category (drives category-specific hints and capability switches)
  category?: ProviderCategory;
  createdAt?: number; // Creation timestamp (ms)
  sortIndex?: number; // Sort index (for custom drag ordering)
  // Notes
  notes?: string;
  // Optional provider metadata (kept only in ~/.switchy/config.json, never written to the live config)
  meta?: ProviderMeta;
  // Icon settings
  icon?: string; // Icon name (e.g. "openai", "anthropic")
  iconColor?: string; // Icon color (hex, e.g. "#00A67E")
  // Whether the provider is in the failover queue
  inFailoverQueue?: boolean;
}

export interface AppConfig {
  providers: Record<string, Provider>;
  current: string;
}

// Custom endpoint
export interface CustomEndpoint {
  url: string;
  addedAt: number;
  lastUsed?: number;
}

// Endpoint candidate (for the endpoint speed-test dialog)
export interface EndpointCandidate {
  id?: string;
  url: string;
  isCustom?: boolean;
}

import type { TemplateType } from "./config/constants";

// Usage query script settings
export interface UsageScript {
  enabled: boolean; // Usage query enabled
  language: "javascript"; // Script language
  code: string; // Script code (JSON config)
  timeout?: number; // Timeout in seconds (default 10)
  templateType?: TemplateType; // Template type (the backend picks validation rules from it)
  apiKey?: string; // API key for usage queries only (general template)
  baseUrl?: string; // Base URL for usage queries only (general and NewAPI templates)
  accessToken?: string; // Access token (NewAPI template)
  userId?: string; // User ID (NewAPI template)
  codingPlanProvider?: string; // Coding Plan provider id (e.g. "kimi", "zhipu", "minimax")
  autoQueryInterval?: number; // Auto-query interval in minutes (0 disables)
  autoIntervalMinutes?: number; // Auto-query interval in minutes (alias)
  request?: {
    // Request settings
    url?: string; // Request URL
    method?: string; // HTTP method
    headers?: Record<string, string>; // Request headers
    body?: any; // Request body
  };
}

// Usage data for one plan
export interface UsageData {
  planName?: string; // Plan name (optional)
  extra?: string; // Extra field for any text to display (optional)
  isValid?: boolean; // Whether the plan is valid (optional)
  invalidMessage?: string; // Why the plan is invalid (optional, shown when isValid is false)
  total?: number; // Total quota (optional)
  used?: number; // Used quota (optional)
  remaining?: number; // Remaining quota (optional)
  unit?: string; // Unit (optional)
}

// Usage query result (supports multiple plans)
export interface UsageResult {
  success: boolean;
  data?: UsageData[]; // Array, so several plans can be returned
  error?: string;
}

// Per-provider model test settings
export interface ProviderTestConfig {
  // Use these settings instead of the global ones (false uses the global settings)
  enabled: boolean;
  // Model name to test (overrides the global setting)
  testModel?: string;
  // Timeout (seconds)
  timeoutSecs?: number;
  // Test prompt
  testPrompt?: string;
  // Degraded threshold (ms)
  degradedThresholdMs?: number;
  // Maximum retries
  maxRetries?: number;
}

// Per-provider proxy settings
export interface ProviderProxyConfig {
  // Use these settings instead of the global ones (false uses the global/system proxy)
  enabled: boolean;
  // Proxy type: http, https, socks5
  proxyType?: "http" | "https" | "socks5";
  // Proxy host
  proxyHost?: string;
  // Proxy port
  proxyPort?: number;
  // Proxy username (optional)
  proxyUsername?: string;
  // Proxy password (optional)
  proxyPassword?: string;
}

export type AuthBindingSource = "provider_config" | "managed_account";

export interface AuthBinding {
  source: AuthBindingSource;
  authProvider?: string;
  accountId?: string;
}

// Provider metadata (field names match the backend, so snake_case)
export interface ProviderMeta {
  // Custom endpoints: keyed by URL, value is the endpoint info
  custom_endpoints?: Record<string, CustomEndpoint>;
  // Whether to apply the common config snippet when switching/syncing to live
  commonConfigEnabled?: boolean;
  // Usage query script settings
  usage_script?: UsageScript;
  // Endpoint management: pick the fastest endpoint after a speed test
  endpointAutoSelect?: boolean;
  // Partner promotion key (lets the backend recognize PackyCode etc.)
  partnerPromotionKey?: string;
  // Per-provider model test settings
  testConfig?: ProviderTestConfig;
  // Per-provider proxy settings
  proxyConfig?: ProviderProxyConfig;
  // Provider cost multiplier
  costMultiplier?: string;
  // Provider billing mode source
  pricingModelSource?: string;
  // Claude API format (Claude providers only)
  // - "anthropic": native Anthropic Messages API, passed through as is
  // - "openai_chat": OpenAI Chat Completions, needs format conversion
  // - "openai_responses": OpenAI Responses API, needs format conversion
  apiFormat?: "anthropic" | "openai_chat" | "openai_responses";
  // Shared auth binding
  authBinding?: AuthBinding;
  // Claude auth field name
  apiKeyField?: ClaudeApiKeyField;
  // Treat base_url as the full API endpoint (the proxy uses this URL as is, without appending a path)
  isFullUrl?: boolean;
  // Prompt cache key for OpenAI-compatible endpoints (improves cache hit rate)
  promptCacheKey?: string;
  // Provider type (identifies special providers such as Copilot)
  providerType?: string;
  // Linked GitHub Copilot account ID (legacy field, still read for compatibility)
  githubAccountId?: string;
  // Captured Claude OAuth identity (Official/Claude providers only).
  // Presence implies a snapshot exists under ~/.switchy/accounts/{id}/.
  capturedClaudeAccount?: {
    accountUuid: string;
    emailAddress: string;
    capturedAt: number;
  };
}

// Claude API format type
// - "anthropic": native Anthropic Messages API, passed through as is
// - "openai_chat": OpenAI Chat Completions, needs format conversion
// - "openai_responses": OpenAI Responses API, needs format conversion
export type ClaudeApiFormat = "anthropic" | "openai_chat" | "openai_responses";

// Claude auth field type
export type ClaudeApiKeyField = "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_API_KEY";

// Apps shown on the main page
export interface VisibleApps {
  claude: boolean;
  codex: boolean;
  gemini: boolean;
  kimi: boolean;
  opencode: boolean;
  openclaw: boolean;
}

// App settings (for the settings dialog and the Tauri API)
// Stored locally in ~/.switchy/settings.json, not synced with the database
export interface Settings {
  // ===== Device-level UI settings =====
  // Show the icon in the system tray (macOS menu bar)
  showInTray: boolean;
  // Minimize to the tray instead of quitting when the close button is clicked
  minimizeToTrayOnClose: boolean;
  // Launch at login
  launchOnStartup?: boolean;
  // Silent start (do not show the main window at launch)
  silentStartup?: boolean;
  // Enable the local proxy on the main page (off by default)
  enableLocalProxy?: boolean;
  // User has confirmed the local proxy first-run notice
  proxyConfirmed?: boolean;
  // User has confirmed the usage query first-run notice
  usageConfirmed?: boolean;
  // User has confirmed the stream check first-run notice
  streamCheckConfirmed?: boolean;
  // Whether to show the failover toggle independently on the main page
  enableFailoverToggle?: boolean;
  // User has confirmed the failover toggle first-run notice
  failoverConfirmed?: boolean;
  // User has confirmed the auto-sync traffic warning
  autoSyncConfirmed?: boolean;
  // Preferred language (optional, defaults to Chinese)
  language?: "en" | "zh" | "ja";

  // Apps shown on the main page (all by default)
  visibleApps?: VisibleApps;

  // ===== Device-level directory overrides =====
  // Override the Claude Code config directory (optional)
  claudeConfigDir?: string;
  // Optional Claude Code mirror config directory (e.g. WSL)
  claudeMirrorConfigDir?: string;
  // Override the Codex config directory (optional)
  codexConfigDir?: string;
  // Optional Codex mirror config directory (e.g. WSL)
  codexMirrorConfigDir?: string;
  // Override the Gemini config directory (optional)
  geminiConfigDir?: string;
  // Override the Kimi Code config directory (optional)
  kimiConfigDir?: string;
  // Override the OpenCode config directory (optional)
  opencodeConfigDir?: string;
  // Override the OpenClaw config directory (optional)
  openclawConfigDir?: string;

  // ===== Current provider IDs (device-level) =====
  // Current Claude provider ID (takes precedence over the database is_current)
  currentProviderClaude?: string;
  // Current Codex provider ID (takes precedence over the database is_current)
  currentProviderCodex?: string;
  // Current Gemini provider ID (takes precedence over the database is_current)
  currentProviderGemini?: string;
  // Current Kimi provider ID (takes precedence over the database is_current)
  currentProviderKimi?: string;



  // ===== Backup policy =====
  // Auto-backup interval in hours (0=disabled, default 24)
  backupIntervalHours?: number;
  // Maximum backup files to retain (default 10)
  backupRetainCount?: number;

  // ===== Terminal =====
  // Preferred terminal app (optional, defaults to the system terminal)
  // macOS: "terminal" | "iterm2" | "warp" | "alacritty" | "kitty" | "ghostty"
  // Windows: "cmd" | "powershell" | "wt"
  // Linux: "gnome-terminal" | "konsole" | "xfce4-terminal" | "alacritty" | "kitty" | "ghostty"
  preferredTerminal?: string;
}

// MCP server connection parameters (loose: extra fields allowed)
export interface McpServerSpec {
  // Optional: stdio configs in common community .mcp.json files may omit type
  type?: "stdio" | "http" | "sse";
  // stdio fields
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string;
  // http and sse fields
  url?: string;
  headers?: Record<string, string>;
  // Shared fields
  [key: string]: any;
}

// v3.7.0: per-app enabled state of an MCP server
export interface McpApps {
  claude: boolean;
  codex: boolean;
  gemini: boolean;
  kimi: boolean;
  opencode: boolean;
  openclaw: boolean;
}

// MCP server entry (v3.7.0 unified shape)
export interface McpServer {
  id: string;
  name: string;
  server: McpServerSpec;
  apps: McpApps; // v3.7.0: which clients the server applies to
  description?: string;
  tags?: string[];
  homepage?: string;
  docs?: string;
  // Legacy fields (v3.6.x and earlier)
  enabled?: boolean; // Deprecated; v3.7.0 uses apps
  source?: string;
  [key: string]: any;
}

// MCP server map (id -> McpServer)
export type McpServersMap = Record<string, McpServer>;

// MCP config status
export interface McpStatus {
  userConfigPath: string;
  userConfigExists: boolean;
  serverCount: number;
}

// MCP list response from config.json
export interface McpConfigResponse {
  configPath: string;
  servers: Record<string, McpServer>;
}

// ============================================================================
// Universal provider: config shared across apps
// ============================================================================

// Per-app enabled state of a universal provider
export interface UniversalProviderApps {
  claude: boolean;
  codex: boolean;
  gemini: boolean;
}

// Claude model settings
export interface ClaudeModelConfig {
  model?: string;
  haikuModel?: string;
  sonnetModel?: string;
  opusModel?: string;
}

// Codex model settings
export interface CodexModelConfig {
  model?: string;
  reasoningEffort?: string;
}

// Gemini model settings
export interface GeminiModelConfig {
  model?: string;
}

// Model settings per app
export interface UniversalProviderModels {
  claude?: ClaudeModelConfig;
  codex?: CodexModelConfig;
  gemini?: GeminiModelConfig;
}

// Universal provider (config shared across apps)
export interface UniversalProvider {
  id: string;
  name: string;
  providerType: string; // "newapi" | "custom" etc.
  apps: UniversalProviderApps;
  baseUrl: string;
  apiKey: string;
  models: UniversalProviderModels;
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  meta?: ProviderMeta;
  createdAt?: number;
  sortIndex?: number;
}

// Universal provider map (id -> UniversalProvider)
export type UniversalProvidersMap = Record<string, UniversalProvider>;

// ============================================================================
// OpenCode-specific config (v3.9.2+)
// ============================================================================

// OpenCode model settings
export interface OpenCodeModel {
  name: string;
  limit?: {
    context?: number;
    output?: number;
  };
  options?: Record<string, unknown>; // Extra per-model options (provider routing etc.)
  // Any extra fields allowed (cost, modalities, thinking, variants etc.)
  [key: string]: unknown;
}

// OpenCode provider options
export interface OpenCodeProviderOptions {
  baseURL?: string;
  apiKey?: string;
  headers?: Record<string, string>;
  // Extra options allowed (timeout, setCacheKey etc.)
  [key: string]: unknown;
}

// OpenCode provider config (settings_config shape)
export interface OpenCodeProviderConfig {
  npm: string; // AI SDK package name, e.g. "@ai-sdk/openai-compatible"
  name?: string; // Provider display name
  options: OpenCodeProviderOptions;
  models: Record<string, OpenCodeModel>;
}

// OpenCode MCP server config (differs from the unified format)
export interface OpenCodeMcpServerSpec {
  type: "local" | "remote";
  // local type fields
  command?: string[]; // Unlike the unified format, command and args are one array
  environment?: Record<string, string>; // Unlike the unified format, uses environment instead of env
  // remote type fields
  url?: string;
  headers?: Record<string, string>;
  // Shared fields
  enabled?: boolean;
}

// ============================================================================
// OpenClaw-specific config (v3.11.0+)
// ============================================================================

// OpenClaw model settings
export interface OpenClawModel {
  id: string;
  name: string;
  alias?: string;
  reasoning?: boolean; // Supports reasoning mode (e.g. o1, DeepSeek R1)
  input?: string[]; // Supported input types (e.g. ["text"], ["text", "image"])
  cost?: {
    input: number;
    output: number;
    cacheRead?: number; // Cache read price
    cacheWrite?: number; // Cache write price
  };
  contextWindow?: number;
  maxTokens?: number; // Maximum output tokens
}

// OpenClaw default model config (agents.defaults.model)
export interface OpenClawDefaultModel {
  primary: string;
  fallbacks?: string[];
}

// OpenClaw model catalog entry (values in agents.defaults.models)
export interface OpenClawModelCatalogEntry {
  alias?: string;
}

export interface OpenClawHealthWarning {
  code: string;
  message: string;
  path?: string;
}

export interface OpenClawWriteOutcome {
  backupPath?: string;
  warnings: OpenClawHealthWarning[];
}

export type OpenClawToolsProfile = "minimal" | "coding" | "messaging" | "full";

// OpenClaw provider config (settings_config shape)
// Maps to OpenClaw models.providers.<provider-id>
export interface OpenClawProviderConfig {
  baseUrl?: string; // API endpoint
  apiKey?: string; // API key
  api?: string; // API protocol type (e.g. "openai-completions", "anthropic")
  models?: OpenClawModel[]; // Available models
  headers?: Record<string, string>; // Custom request headers (e.g. User-Agent)
  authHeader?: boolean; // Provider-specific auth switch (e.g. Longcat)
}

// Full OpenClaw agents.defaults config
export interface OpenClawAgentsDefaults {
  model?: OpenClawDefaultModel;
  models?: Record<string, OpenClawModelCatalogEntry>;
  timeoutSeconds?: number;
  timeout?: number;
  [key: string]: unknown; // preserve unknown fields
}

// OpenClaw env config (the env node of openclaw.json)
export interface OpenClawEnvConfig {
  [key: string]: unknown;
}

// OpenClaw tools config (the tools node of openclaw.json)
export interface OpenClawToolsConfig {
  profile?: OpenClawToolsProfile | string;
  allow?: string[];
  deny?: string[];
  [key: string]: unknown; // preserve unknown fields
}
