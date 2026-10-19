//! Complete usage declarations. Sources are added only with their readers.
use serde::{Deserialize, Serialize};

pub const SHIPPED_USAGE_LAYER: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "support", content = "detail", rename_all = "snake_case")]
pub enum Support<S, A = Absence> {
    Supported(S),
    Unsupported(A),
}

impl<S: Copy, A: Copy> Support<S, A> {
    /// Presence and fallback queries also used by existing optional-capability callers.
    pub const fn is_some(self) -> bool {
        matches!(self, Self::Supported(_))
    }

    pub const fn supported(self) -> Option<S> {
        match self {
            Self::Supported(value) => Some(value),
            Self::Unsupported(_) => None,
        }
    }

    pub const fn or(self, fallback: Option<S>) -> Option<S> {
        match self {
            Self::Supported(value) => Some(value),
            Self::Unsupported(_) => fallback,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Absence {
    pub kind: AbsenceKind,
    pub reason: &'static str,
    pub would_change: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AbsenceKind {
    NoSuchData,
    Excluded,
    NotBuilt { layer: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitSource {
    ClaudeUsageText,
    CodexUsage,
    /// `agy -p "/usage" --output-format json`, which asks agy's server for
    /// its weekly buckets without a model call.
    AntigravityUsage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHour,
    Weekly,
    Monthly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Always,
    PlanDependent,
}

impl LimitSource {
    pub const ALL: [Self; 3] = [
        Self::ClaudeUsageText,
        Self::CodexUsage,
        Self::AntigravityUsage,
    ];

    pub const fn presence(self, window: WindowKind) -> Presence {
        match (self, window) {
            (Self::ClaudeUsageText, WindowKind::FiveHour | WindowKind::Weekly)
            | (Self::CodexUsage | Self::AntigravityUsage, WindowKind::Weekly) => Presence::Always,
            (Self::CodexUsage | Self::AntigravityUsage, WindowKind::FiveHour)
            | (
                Self::ClaudeUsageText | Self::CodexUsage | Self::AntigravityUsage,
                WindowKind::Monthly,
            ) => Presence::PlanDependent,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenSource {
    ClaudeTranscript,
    CodexRollout,
    GrokUpdates,
    OpenCodeParts,
    PiSession,
    OmpSession,
    /// The per-call token counts of agy's `transcript_full.jsonl`.
    AntigravityTranscript,
}
impl TokenSource {
    pub const fn format(self) -> crate::SessionFormat {
        match self {
            Self::ClaudeTranscript => crate::SessionFormat::Claude,
            Self::CodexRollout => crate::SessionFormat::Codex,
            Self::GrokUpdates => crate::SessionFormat::Grok,
            Self::OpenCodeParts => crate::SessionFormat::OpenCode,
            Self::PiSession => crate::SessionFormat::Pi,
            Self::OmpSession => crate::SessionFormat::Omp,
            Self::AntigravityTranscript => crate::SessionFormat::Antigravity,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostBasis {
    Recorded,
    PublicTable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFrom {
    UsageRecord,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionFrom {
    UsageRecord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct UsageDeclaration {
    pub limits: LimitDeclaration,
    pub tokens: Support<TokenSource>,
    pub cost: Support<CostBasis>,
    pub models: Support<ModelFrom>,
    pub attribution: Attribution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LimitDeclaration {
    pub five_hour: Support<LimitSource>,
    pub weekly: Support<LimitSource>,
    pub scoped: Support<LimitSource>,
    pub monthly: Support<LimitSource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Attribution {
    pub session: Support<AttributionFrom>,
    pub project: Support<AttributionFrom>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "source", rename_all = "snake_case")]
pub enum UsageSource {
    Limit(LimitSource),
    Tokens(TokenSource),
    Cost(CostBasis),
    Models(ModelFrom),
    Attribution(AttributionFrom),
}

impl<S: Copy> Support<S> {
    fn map(self, source: impl FnOnce(S) -> UsageSource) -> Support<UsageSource> {
        match self {
            Self::Supported(value) => Support::Supported(source(value)),
            Self::Unsupported(absence) => Support::Unsupported(absence),
        }
    }
}

impl UsageDeclaration {
    /// Exhaustive destructuring makes every added capability join the
    /// exporter, scheduler and contract validation in the same change.
    pub fn cells(self) -> [(&'static str, Support<UsageSource>); 9] {
        let Self {
            limits,
            tokens,
            cost,
            models,
            attribution,
        } = self;
        let LimitDeclaration {
            five_hour,
            weekly,
            scoped,
            monthly,
        } = limits;
        let Attribution { session, project } = attribution;
        [
            ("five_hour", five_hour.map(UsageSource::Limit)),
            ("weekly", weekly.map(UsageSource::Limit)),
            ("scoped", scoped.map(UsageSource::Limit)),
            ("monthly", monthly.map(UsageSource::Limit)),
            ("tokens", tokens.map(UsageSource::Tokens)),
            ("cost", cost.map(UsageSource::Cost)),
            ("models", models.map(UsageSource::Models)),
            ("session", session.map(UsageSource::Attribution)),
            ("project", project.map(UsageSource::Attribution)),
        ]
    }

    pub fn limit_sources(self) -> impl Iterator<Item = LimitSource> {
        LimitSource::ALL.into_iter().filter(move |source| {
            self.cells()
                .into_iter()
                .any(|(_, cell)| cell == Support::Supported(UsageSource::Limit(*source)))
        })
    }

    pub const fn window(self, kind: WindowKind) -> Support<LimitSource> {
        match kind {
            WindowKind::FiveHour => self.limits.five_hour,
            WindowKind::Weekly => self.limits.weekly,
            WindowKind::Monthly => self.limits.monthly,
        }
    }
}

pub const fn absent<S>(
    kind: AbsenceKind,
    reason: &'static str,
    would_change: &'static str,
) -> Support<S> {
    Support::Unsupported(Absence {
        kind,
        reason,
        would_change,
    })
}

pub const fn not_built<S>(layer: u8) -> Support<S> {
    absent(
        AbsenceKind::NotBuilt { layer },
        "The usage reader is not built yet.",
        "Ship the declared usage layer and its reader.",
    )
}

/// The documentation table shares the exhaustive coverage walk with JSON.
pub fn coverage_markdown() -> String {
    let mut out = String::from(
        "| Agent | 5h | Weekly | Scoped | Monthly | Tokens | Cost | Models | Session | Project |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for row in crate::ADAPTERS {
        out.push_str(&format!("| {} |", row.label));
        for (_, cell) in row.usage.cells() {
            let value = match cell {
                Support::Supported(_) => "Supported".into(),
                Support::Unsupported(absence) => match absence.kind {
                    AbsenceKind::NoSuchData => "No such data".into(),
                    AbsenceKind::Excluded => "Excluded".into(),
                    AbsenceKind::NotBuilt { layer } => format!("Not built (L{layer})"),
                },
            };
            out.push_str(&format!(" {value} |"));
        }
        out.push('\n');
    }
    out
}
