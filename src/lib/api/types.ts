// The frontend uses AppId as the app identifier everywhere (matches the backend command parameter `app`)
export type AppId =
  | "claude"
  | "codex"
  | "gemini"
  | "kimi"
  | "opencode"
  | "openclaw";
