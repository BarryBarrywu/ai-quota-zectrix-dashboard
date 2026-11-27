use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dashboard {
    pub now: DateTime<FixedOffset>,
    pub screen_updated_at: Option<DateTime<FixedOffset>>,
    pub codex: Source<CodexQuota>,
    pub antigravity: Source<AntigravityQuota>,
    pub codex_radar: Source<Radar>,
    pub antigravity_radar: Source<Radar>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum Source<T> {
    Current(T),
    Stale(T),
    #[default]
    Unavailable,
}
impl<T> Source<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Current(v) | Self::Stale(v) => Some(v),
            Self::Unavailable => None,
        }
    }
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Stale(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexQuota {
    pub weekly: QuotaWindow,
    pub reset_credits: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AntigravityQuota {
    pub gemini: Option<GroupReading>,
    pub claude_gpt: Option<GroupReading>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupReading {
    #[serde(flatten)]
    pub window: QuotaWindow,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub last_success: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub kind: WindowKind,
    pub remaining_percent: u8,
    pub resets_at: Option<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    Weekly,
    FiveHour,
    Tied,
    Unspecified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Radar {
    Scheduled {
        starts_at: i64,
        ends_at: i64,
        scope: Scope,
    },
    Confirmed {
        kind: ResetKind,
        announced_at: i64,
        scope: Scope,
    },
    NoSchedule,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    All,
    Gemini,
    ClaudeGpt,
    Named,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetKind {
    Global,
    Banked,
}
