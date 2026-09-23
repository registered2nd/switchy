use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::error::AppError;

/// A CLI tool whose providers Switchy switches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    Claude,
    Codex,
    Gemini,
    Kimi,
    OpenCode,
    OpenClaw,
}

impl AppType {
    pub fn as_str(&self) -> &str {
        match self {
            AppType::Claude => "claude",
            AppType::Codex => "codex",
            AppType::Gemini => "gemini",
            AppType::Kimi => "kimi",
            AppType::OpenCode => "opencode",
            AppType::OpenClaw => "openclaw",
        }
    }

    /// Additive apps keep every provider in their config file at once
    /// (OpenCode, OpenClaw); the others hold one current provider.
    pub fn is_additive_mode(&self) -> bool {
        matches!(self, AppType::OpenCode | AppType::OpenClaw)
    }

    /// Every app type.
    pub fn all() -> impl Iterator<Item = AppType> {
        [
            AppType::Claude,
            AppType::Codex,
            AppType::Gemini,
            AppType::Kimi,
            AppType::OpenCode,
            AppType::OpenClaw,
        ]
        .into_iter()
    }
}

impl FromStr for AppType {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_lowercase();
        match normalized.as_str() {
            "claude" => Ok(AppType::Claude),
            "codex" => Ok(AppType::Codex),
            "gemini" => Ok(AppType::Gemini),
            "kimi" => Ok(AppType::Kimi),
            "opencode" => Ok(AppType::OpenCode),
            "openclaw" => Ok(AppType::OpenClaw),
            other => Err(AppError::localized(
                "unsupported_app",
                format!("Unsupported app id: '{other}'. Allowed: claude, codex, gemini, kimi, opencode, openclaw."),
            )),
        }
    }
}
