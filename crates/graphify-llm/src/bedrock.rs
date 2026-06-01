//! AWS Bedrock backend — Converse API via `aws-sdk-bedrockruntime`.
//!
//! Replaces a hand-rolled `ureq` + AWS Signature V4 implementation with the
//! official SDK so we pick up the standard credential provider chain:
//! environment variables, `~/.aws/credentials` profiles, IMDS / IAM Roles,
//! ECS task roles, SSO, and STS web-identity / role-assumption.
//!
//! Divergence from `graphify-py/graphify/llm.py`: the Python reference uses
//! `boto3` (which has the same credential chain) but auto-detection there
//! only checks `AWS_REGION` / `AWS_PROFILE` env vars — it would happily pick
//! Bedrock as the backend and then fail on every chunk if real credentials
//! aren't configured. This port tightens the auto-detect rule (see
//! [`crate::backends::detect_backend`]) so we only land on Bedrock when
//! credentials are actually resolvable.

use std::sync::OnceLock;

use aws_config::{BehaviorVersion, Region};
use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::config::Builder as SdkConfigBuilder;
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, InferenceConfiguration, Message, StopReason, SystemContentBlock,
};
use serde_json::json;
use tokio::runtime::Runtime;

use crate::openai_compat::resolve_max_tokens;
use crate::{
    EXTRACTION_SYSTEM, LlmBackend, LlmError, LlmResponse, parse_llm_json, response_is_hollow,
};

/// Default model.
pub const DEFAULT_MODEL: &str = "anthropic.claude-3-5-sonnet-20241022-v2:0";
/// Model override env var.
pub const MODEL_ENV_KEY: &str = "GRAPHIFY_BEDROCK_MODEL";
/// Endpoint override env var. When set, the SDK will route requests to this
/// URL instead of the standard `https://bedrock-runtime.<region>.amazonaws.com`.
/// Used by tests (mockito) and rarely for VPC endpoint overrides.
pub const BASE_URL_ENV_KEY: &str = "GRAPHIFY_BEDROCK_BASE_URL";

/// Effective base URL — defaults to AWS's regional endpoint, overrideable via env.
#[must_use]
pub fn base_url(region: &str) -> String {
    std::env::var(BASE_URL_ENV_KEY)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("https://bedrock-runtime.{region}.amazonaws.com"))
}

/// Bedrock backend.
pub struct BedrockBackend {
    region: String,
}

impl BedrockBackend {
    /// Create from environment.
    #[must_use]
    pub fn from_env() -> Self {
        let region = resolve_region();
        Self { region }
    }

    /// Create with explicit region.
    #[must_use]
    pub fn new(region: String) -> Self {
        Self { region }
    }
}

impl LlmBackend for BedrockBackend {
    /// Returns the backend identifier string.
    fn name(&self) -> &'static str {
        "bedrock"
    }

    /// Dispatches to [`call_bedrock`] using the stored region.
    fn call(
        &self,
        messages: &[serde_json::Value],
        model: &str,
        max_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
        call_bedrock(model, &self.region, messages, max_tokens)
    }

    /// Delegates to the shared tiktoken-based estimator.
    fn estimate_tokens(&self, text: &str) -> usize {
        crate::tokenizer::estimate_tokens(text)
    }
}

/// Resolve AWS region from env vars.
#[must_use]
pub fn resolve_region() -> String {
    std::env::var("AWS_REGION")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("AWS_DEFAULT_REGION")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "us-east-1".to_string())
}

/// Returns `true` when the environment looks like it has AWS credentials
/// configured — i.e. one of the standard credential-provider entry points
/// is set. Used by [`crate::backends::detect_backend`] to avoid picking
/// Bedrock when only `AWS_REGION` is set.
///
/// This is intentionally a fast env-var check rather than a full
/// credential-chain resolution (which would require spinning up an async
/// runtime). Real credential resolution happens inside [`call_bedrock`]
/// via the SDK.
#[must_use]
pub fn credentials_appear_configured() -> bool {
    let env_set = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
    // Explicit static credentials require BOTH the access key id and secret;
    // every other entry point in `CREDENTIAL_ENV_VARS` is sufficient on its own.
    if env_set("AWS_ACCESS_KEY_ID") && env_set("AWS_SECRET_ACCESS_KEY") {
        return true;
    }
    CREDENTIAL_ENV_VARS
        .iter()
        .filter(|k| !matches!(**k, "AWS_ACCESS_KEY_ID" | "AWS_SECRET_ACCESS_KEY"))
        .any(|k| env_set(k))
}

/// AWS credential-provider environment variables that
/// [`credentials_appear_configured`] treats as evidence that credentials are
/// configured. Centralised so the no-backend detection list
/// ([`crate::backend_selection_env_vars`]) and its test scrub stay in lockstep
/// with the detection logic above. `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`
/// are listed individually but only count as credentials when both are present.
pub const CREDENTIAL_ENV_VARS: &[&str] = &[
    // Explicit static credentials (need both of these together).
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    // Profile in ~/.aws/credentials or ~/.aws/config.
    "AWS_PROFILE",
    // Web identity (IRSA on EKS, GitHub OIDC, etc.).
    "AWS_WEB_IDENTITY_TOKEN_FILE",
    // ECS / container task roles.
    "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
];

/// Process-wide tokio runtime used to drive the (async-only) AWS SDK.
///
/// Building the runtime fails only when the OS denies thread/file-descriptor
/// resources at process start — at which point Bedrock calls have no way to
/// proceed anyway, so an unrecoverable panic is the honest signal. The
/// runtime is shared across calls so the SDK's connection pool and
/// background tasks stay alive for the life of the process.
fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        // panic justification: see the doc comment — failure to build the
        // tokio runtime at process start is unrecoverable for any Bedrock
        // call this process could possibly make.
        Runtime::new().unwrap_or_else(|e| {
            panic!(
                "failed to build tokio runtime for Bedrock: {e} \
                 (check `ulimit` for open files / threads; the runtime needs both \
                 to spawn worker threads)"
            )
        })
    })
}

/// Build the SDK client. Each call resolves credentials freshly via the
/// SDK's default provider chain, so callers that change `AWS_*` env vars
/// mid-process still pick up the new values.
fn client_for(region: &str) -> Client {
    let endpoint_override = std::env::var(BASE_URL_ENV_KEY)
        .ok()
        .filter(|s| !s.is_empty());

    let base = runtime().block_on(async {
        aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region.to_string()))
            .load()
            .await
    });

    let mut builder: SdkConfigBuilder = (&base).into();
    if let Some(endpoint) = endpoint_override {
        builder = builder.endpoint_url(endpoint);
    }
    Client::from_conf(builder.build())
}

/// Call AWS Bedrock Converse API.
///
/// Uses the AWS SDK and its standard credential provider chain. The caller
/// no longer needs to set `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`
/// directly — any provider supported by `aws-config` works (env, profile,
/// IMDS, ECS, web identity, SSO).
///
/// # Errors
/// Returns [`LlmError::NoApiKey`] if no AWS credentials can be resolved,
/// or [`LlmError::Http`] / [`LlmError::Parse`] on transport / response
/// errors.
pub fn call_bedrock(
    model: &str,
    region: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> Result<LlmResponse, LlmError> {
    call_bedrock_with_system(model, region, messages, max_tokens, EXTRACTION_SYSTEM)
}

/// Call AWS Bedrock Converse API with an explicit system prompt.
///
/// # Errors
/// Returns [`LlmError::NoApiKey`] if no AWS credentials can be resolved,
/// or [`LlmError::Http`] / [`LlmError::Parse`] on transport / response errors.
pub fn call_bedrock_with_system(
    model: &str,
    region: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    system_prompt: &str,
) -> Result<LlmResponse, LlmError> {
    let client = client_for(region);
    let sdk_messages = build_messages(messages)?;
    let system = SystemContentBlock::Text(system_prompt.to_string());

    let inference = InferenceConfiguration::builder()
        .max_tokens(i32::try_from(max_tokens).unwrap_or(i32::MAX))
        .temperature(0.0)
        .build();

    let output = runtime().block_on(async {
        client
            .converse()
            .model_id(model)
            .system(system)
            .set_messages(Some(sdk_messages))
            .inference_config(inference)
            .send()
            .await
    });

    let output = output.map_err(|e| map_sdk_error(&e))?;

    // Extract assistant text — the Converse output is a sequence of
    // content blocks; we concatenate every Text block.
    let mut text = String::new();
    if let Some(out) = output.output.as_ref()
        && let Ok(msg) = out.as_message()
    {
        for block in &msg.content {
            if let ContentBlock::Text(t) = block {
                text.push_str(t);
            }
        }
    }
    if text.is_empty() {
        text.push_str("{}");
    }

    let input_tokens = output
        .usage
        .as_ref()
        .map(|u| u.input_tokens)
        .and_then(|v| u64::try_from(v).ok())
        .unwrap_or(0);
    let output_tokens = output
        .usage
        .as_ref()
        .map(|u| u.output_tokens)
        .and_then(|v| u64::try_from(v).ok())
        .unwrap_or(0);

    let mut finish_reason = match output.stop_reason {
        StopReason::MaxTokens => "length".to_string(),
        _ => "stop".to_string(),
    };

    let mut parsed = parse_llm_json(&text);
    if response_is_hollow(Some(text.as_str()), &parsed) && finish_reason != "length" {
        eprintln!(
            "[graphify] bedrock returned a hollow response; treating as \
             truncation so adaptive retry can bisect the chunk."
        );
        finish_reason = "length".to_string();
    }

    parsed["input_tokens"] = json!(input_tokens);
    parsed["output_tokens"] = json!(output_tokens);
    parsed["model"] = json!(model);
    parsed["finish_reason"] = json!(&finish_reason);

    Ok(LlmResponse {
        nodes: parsed["nodes"].as_array().cloned().unwrap_or_default(),
        edges: parsed["edges"].as_array().cloned().unwrap_or_default(),
        hyperedges: parsed["hyperedges"].as_array().cloned().unwrap_or_default(),
        input_tokens,
        output_tokens,
        model: model.to_string(),
        finish_reason,
        elapsed_seconds: 0.0,
        failed_chunk_indices: vec![],
    })
}

/// Convert the wire-format messages (`[{role, content: [{text}]}]`) into
/// SDK `Message` builders. Each content list element becomes a `Text`
/// content block on the SDK side.
fn build_messages(messages: &[serde_json::Value]) -> Result<Vec<Message>, LlmError> {
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    for msg in messages {
        let role = match msg.get("role").and_then(|v| v.as_str()) {
            Some("assistant") => ConversationRole::Assistant,
            // Any other value (including missing) → user role. Bedrock's
            // Converse API rejects an empty `messages` array, so this also
            // covers the common case of callers omitting the role.
            _ => ConversationRole::User,
        };

        let mut builder = Message::builder().role(role);
        if let Some(content) = msg.get("content").and_then(|v| v.as_array()) {
            for block in content {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    builder = builder.content(ContentBlock::Text(text.to_string()));
                }
            }
        }
        let built = builder
            .build()
            .map_err(|e| LlmError::Parse(format!("bedrock message build failed: {e}")))?;
        out.push(built);
    }
    Ok(out)
}

/// Translate an SDK `ConverseError` into the crate's [`LlmError`].
///
/// `NoApiKey` is reserved for the case where the credential chain returned
/// no usable credentials. Everything else is an [`LlmError::Http`] carrying
/// the SDK's diagnostic string.
fn map_sdk_error(
    err: &aws_sdk_bedrockruntime::error::SdkError<
        aws_sdk_bedrockruntime::operation::converse::ConverseError,
    >,
) -> LlmError {
    use aws_sdk_bedrockruntime::error::SdkError;
    let msg = match err {
        SdkError::DispatchFailure(d) => {
            let mut s = format!("{d:?}");
            // Recognise credential-resolution failures so the user sees the
            // same hint they'd have gotten from the old hand-rolled check.
            if s.contains("CredentialsNotLoaded") || s.contains("no credentials") {
                return LlmError::NoApiKey(
                    "AWS credentials not configured for Bedrock (set AWS_ACCESS_KEY_ID + \
                     AWS_SECRET_ACCESS_KEY, run `aws configure`, or assume a role)"
                        .to_string(),
                );
            }
            s.truncate(512);
            s
        }
        _ => err.to_string(),
    };
    LlmError::Http(msg)
}

/// Send a plain-text `prompt` to Bedrock and return the first text content
/// of the response. Used by callers that want a free-form completion rather
/// than the extraction-shaped JSON.
///
/// # Errors
/// Returns [`LlmError::NoApiKey`] when AWS credentials are missing,
/// or [`LlmError::Http`] / [`LlmError::Parse`] on transport or parse failure.
pub fn call_bedrock_plain(
    model: &str,
    region: &str,
    prompt: &str,
    max_tokens: u32,
) -> Result<String, LlmError> {
    let client = client_for(region);
    let messages = vec![
        Message::builder()
            .role(ConversationRole::User)
            .content(ContentBlock::Text(prompt.to_string()))
            .build()
            .map_err(|e| LlmError::Parse(format!("bedrock message build failed: {e}")))?,
    ];

    let inference = InferenceConfiguration::builder()
        .max_tokens(i32::try_from(max_tokens).unwrap_or(i32::MAX))
        .temperature(0.0)
        .build();

    let output = runtime().block_on(async {
        client
            .converse()
            .model_id(model)
            .set_messages(Some(messages))
            .inference_config(inference)
            .send()
            .await
    });
    let output = output.map_err(|e| map_sdk_error(&e))?;

    let mut text = String::new();
    if let Some(out) = output.output.as_ref()
        && let Ok(msg) = out.as_message()
    {
        for block in &msg.content {
            if let ContentBlock::Text(t) = block {
                text.push_str(t);
            }
        }
    }
    Ok(text)
}

/// Default max tokens for bedrock.
#[must_use]
pub fn default_max_tokens() -> u32 {
    resolve_max_tokens(16_384)
}
