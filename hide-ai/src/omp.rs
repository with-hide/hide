//! OMP's public SDK in a short-lived Bun child, never its ambient CLI harness.
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::program::Program;
use crate::runner::{self, Environment, PrivateFile, Spec};
use crate::text_cli::{TextCliConfig, system_with_schema};
use crate::{
    AiBackend, AiError, AiRequest, AiResponse, AiUsage, Availability, CancelToken, ModelCatalog,
    ProviderId,
};

pub const DEFAULT_MODEL: &str = "anthropic/claude-haiku-4-5";
const BRIDGE: &str = include_str!("omp_bridge.mjs");
const PROBE_DEADLINE: Duration = Duration::from_secs(20);

#[derive(Clone, Debug)]
pub struct OmpConfig {
    pub cli: TextCliConfig,
    /// An explicitly selected, user-installed SDK; otherwise found beside omp.
    pub package: Option<PathBuf>,
    /// An explicitly selected Bun; otherwise resolved on the account's PATH.
    pub runtime: Option<PathBuf>,
}

impl OmpConfig {
    pub fn new(model: &str) -> Self {
        Self {
            cli: TextCliConfig::new(model),
            package: None,
            runtime: None,
        }
    }
}

pub struct OmpSdkBackend {
    config: OmpConfig,
}

impl OmpSdkBackend {
    pub fn new(config: OmpConfig) -> Self {
        Self { config }
    }

    fn installation(&self) -> Result<(Program, PathBuf), AiError> {
        let cli = self
            .config
            .cli
            .resolved("omp")
            .ok_or_else(|| unavailable("omp_not_installed"))?;
        let package = match &self.config.package {
            Some(path) if is_package(path) => path.clone(),
            Some(_) => return Err(unavailable("omp_sdk_package_identity_invalid")),
            None => sdk_beside(cli.path()).ok_or_else(|| unavailable(
                "omp_sdk_package_required_install_official_npm_package_standalone_not_supported"))?,
        };
        let runtime = Program::resolve(
            self.config.runtime.as_deref().unwrap_or(Path::new("bun")),
            self.config.cli.search_path.as_deref(),
        )
        .ok_or_else(|| unavailable("omp_bun_required_install_runtime_matching_package_engines"))?;
        Ok((runtime, package))
    }

    fn call(
        &self,
        mut body: Value,
        deadline: Duration,
        cancel: &CancelToken,
    ) -> Result<Frame, AiError> {
        if cancel.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let (runtime, package) = self.installation()?;
        let bridge = PrivateFile::create("bridge.mjs", BRIDGE)
            .map_err(|_| unavailable("omp_bridge_file_failed"))?;
        let cache = PrivateFile::create("auth-cache.enc", "")
            .map_err(|_| unavailable("omp_cache_file_failed"))?;
        body["auth_cache"] = json!(cache.path());
        if body.to_string().len() > 1024 * 1024 {
            return Err(AiError::Rejected("omp_request_too_large".into()));
        }
        let request = PrivateFile::create("request.json", &body.to_string())
            .map_err(|_| unavailable("omp_request_file_failed"))?;
        let mut args = vec!["--no-env-file".into(), "--no-install".into()];
        // Bun watches the owner even when a crash skips Rust destructors.
        // Windows already ties OwnedChild to a kill-on-close job.
        #[cfg(unix)]
        args.push("--no-orphans".into());
        args.extend([
            bridge.path().to_string_lossy().into_owned(),
            package.to_string_lossy().into_owned(),
            request.path().to_string_lossy().into_owned(),
        ]);
        let run = runner::run(
            &Spec {
                binary: &runtime,
                args: &args,
                cwd: request.dir(),
                stdin: None,
                environment: Environment::Login,
                set: &[],
                deadline,
            },
            cancel,
        )
        .map_err(|error| error.into_error("omp"))?;
        // A crashed child may already have sent the request. Never infer a
        // pre-submission refusal from stderr or an incomplete protocol frame.
        if !run.succeeded() {
            return Err(AiError::CompletionUnknown("omp_child_failed".into()));
        }
        let frame: Frame = serde_json::from_str(&run.stdout)
            .map_err(|_| AiError::CompletionUnknown("omp_bridge_protocol_invalid".into()))?;
        match frame {
            Frame::Error {
                protocol: 1,
                code,
                reason,
            } => Err(match code.as_str() {
                "auth" => AiError::NotAuthenticated,
                "timeout" => AiError::Timeout,
                "quota" => AiError::UsageLimited { retry_after: None },
                "transient" => AiError::Transient(reason),
                "rejected" => AiError::Rejected(reason),
                "invalid" => AiError::InvalidOutput(reason),
                "unavailable" => unavailable(&reason),
                _ => AiError::CompletionUnknown(reason),
            }),
            Frame::Catalog { protocol: 1, .. } | Frame::Answer { protocol: 1, .. } => Ok(frame),
            _ => Err(AiError::CompletionUnknown(
                "omp_bridge_protocol_version_invalid".into(),
            )),
        }
    }

    fn catalog(&self) -> Result<Vec<String>, AiError> {
        match self.call(
            json!({"operation":"catalog"}),
            PROBE_DEADLINE,
            &CancelToken::new(),
        )? {
            Frame::Catalog { models, .. } => Ok(models),
            _ => Err(unavailable("omp_catalog_frame_invalid")),
        }
    }
}

fn unavailable(reason: &str) -> AiError {
    AiError::ProviderUnavailable(reason.to_owned())
}

fn is_package(path: &Path) -> bool {
    let manifest = path.join("package.json");
    std::fs::metadata(&manifest).is_ok_and(|meta| meta.len() <= 65536)
        && std::fs::read(&manifest)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|meta| meta["name"] == "@oh-my-pi/pi-coding-agent")
}

/// Unix package-manager symlinks and Windows npm's sibling node_modules.
/// Bounded lookup; never search a home directory or import an arbitrary module.
fn sdk_beside(binary: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(binary).ok()?;
    for parent in real.ancestors().skip(1).take(8) {
        for candidate in [
            parent.to_owned(),
            parent.join("node_modules/@oh-my-pi/pi-coding-agent"),
        ] {
            if is_package(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Catalog {
        protocol: u8,
        models: Vec<String>,
    },
    Answer {
        protocol: u8,
        text: String,
    },
    Error {
        protocol: u8,
        code: String,
        reason: String,
    },
}

impl AiBackend for OmpSdkBackend {
    fn id(&self) -> ProviderId {
        ProviderId::OMP
    }
    fn availability(&self) -> Availability {
        if self.config.cli.resolved("omp").is_none() {
            return Availability::NotInstalled;
        }
        match self.catalog() {
            Ok(models) if models.is_empty() => Availability::NeedsLogin,
            Ok(_) => Availability::Ready,
            Err(AiError::NotAuthenticated) => Availability::NeedsLogin,
            Err(error) => Availability::Unavailable {
                reason: error.to_string(),
            },
        }
    }
    fn models(&self) -> ModelCatalog {
        match self.catalog() {
            Ok(models) if !models.is_empty() => ModelCatalog::Fixed(models),
            Ok(_) => ModelCatalog::Unknown {
                reason: "omp_login_required".into(),
            },
            Err(error) => ModelCatalog::Unknown {
                reason: error.to_string(),
            },
        }
    }
    fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResponse, AiError> {
        let model = request.model(&self.config.cli.model);
        let frame = self.call(json!({"operation":"execute", "model": model,
            "system": system_with_schema(&request.system_prompt(), &request.output_schema),
            "input": request.input, "deadline_ms": request.deadline.as_millis().min(u32::MAX as u128)}),
            request.deadline, cancel)?;
        let Frame::Answer { text, .. } = frame else {
            return Err(AiError::CompletionUnknown(
                "omp_answer_frame_invalid".into(),
            ));
        };
        let value = serde_json::from_str(&text)
            .map_err(|_| AiError::InvalidOutput("omp_answer_not_json".into()))?;
        // No stream event is counted twice. Until per-model usage is normalized,
        // the existing ledger records this one call as usage unknown.
        Ok(AiResponse {
            value,
            usage: AiUsage::default(),
        })
    }
}
