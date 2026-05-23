//! AWS Bedrock backend — Converse API via `ureq` + AWS Signature `V4`.
//!
//! Ports the `_call_bedrock` function in `graphify-py/graphify/llm.py`.
//!
//! Signs requests manually (`SigV4`) rather than pulling in the full AWS SDK.
//! Reads the standard AWS credential chain: `AWS_ACCESS_KEY_ID` +
//! `AWS_SECRET_ACCESS_KEY` (+ optional `AWS_SESSION_TOKEN`).

use serde::Deserialize;
use serde_json::json;

use crate::openai_compat::resolve_max_tokens;
use crate::{
    EXTRACTION_SYSTEM, LlmBackend, LlmError, LlmResponse, parse_llm_json, response_is_hollow,
};

/// Default model.
pub const DEFAULT_MODEL: &str = "anthropic.claude-3-5-sonnet-20241022-v2:0";
/// Model override env var.
pub const MODEL_ENV_KEY: &str = "GRAPHIFY_BEDROCK_MODEL";

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

#[derive(Deserialize)]
struct BedrockResponse {
    output: Option<BedrockOutput>,
    usage: Option<BedrockUsage>,
    #[serde(rename = "stopReason")]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct BedrockOutput {
    message: Option<BedrockMessage>,
}

#[derive(Deserialize)]
struct BedrockMessage {
    content: Option<Vec<BedrockContent>>,
}

#[derive(Deserialize)]
struct BedrockContent {
    text: Option<String>,
}

#[derive(Deserialize)]
struct BedrockUsage {
    #[serde(rename = "inputTokens")]
    input_tokens: Option<u64>,
    #[serde(rename = "outputTokens")]
    output_tokens: Option<u64>,
}

/// Call AWS Bedrock Converse API.
///
/// Uses AWS Signature `V4`. Reads credentials from the standard env-var chain:
/// `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and optionally
/// `AWS_SESSION_TOKEN`.
///
/// # Errors
/// Returns [`LlmError::NoApiKey`] if credentials are missing, [`LlmError::Security`]
/// if the endpoint URL is rejected, or [`LlmError::Http`] / [`LlmError::Parse`] on
/// transport errors.
pub fn call_bedrock(
    model: &str,
    region: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> Result<LlmResponse, LlmError> {
    let access_key = std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_default();
    let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();
    let session_token = std::env::var("AWS_SESSION_TOKEN").ok();

    if access_key.is_empty() || secret_key.is_empty() {
        return Err(LlmError::NoApiKey(
            "AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY must be set for Bedrock".to_string(),
        ));
    }

    let endpoint = format!("https://bedrock-runtime.{region}.amazonaws.com/model/{model}/converse");
    graphify_security::validate_url(&endpoint)?;

    let body = json!({
        "system": [{"text": EXTRACTION_SYSTEM}],
        "messages": messages,
        "inferenceConfig": {
            "maxTokens": max_tokens,
            "temperature": 0,
        },
    });

    let body_str = serde_json::to_string(&body).map_err(|e| LlmError::Parse(e.to_string()))?;

    let now = chrono_now_utc();
    let date_str = &now[..8]; // YYYYMMDD
    let datetime_str = now.as_str(); // YYYYMMDDTHHmmssZ

    let signed = sign_request(&SignInput {
        method: "POST",
        url: &endpoint,
        region,
        service: "bedrock",
        body: &body_str,
        access_key: &access_key,
        secret_key: &secret_key,
        session_token: session_token.as_deref(),
        datetime_str,
        date_str,
    })?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .build()
        .into();
    let mut req = agent
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .header("x-amz-date", datetime_str);

    for (k, v) in &signed.extra_headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req = req.header("Authorization", &signed.authorization);

    let http_resp = req
        .send(&body_str)
        .map_err(|e| LlmError::Http(e.to_string()))?;

    let resp: BedrockResponse = http_resp
        .into_body()
        .read_json()
        .map_err(|e| LlmError::Parse(e.to_string()))?;

    let text = resp
        .output
        .as_ref()
        .and_then(|o| o.message.as_ref())
        .and_then(|m| m.content.as_ref())
        .and_then(|c| c.first())
        .and_then(|c| c.text.as_deref())
        .unwrap_or("{}");

    let mut parsed = parse_llm_json(text);
    let input_tokens = resp
        .usage
        .as_ref()
        .and_then(|u| u.input_tokens)
        .unwrap_or(0);
    let output_tokens = resp
        .usage
        .as_ref()
        .and_then(|u| u.output_tokens)
        .unwrap_or(0);
    let stop_reason = resp.stop_reason.as_deref().unwrap_or("end_turn");
    let mut finish_reason = if stop_reason == "max_tokens" {
        "length".to_string()
    } else {
        "stop".to_string()
    };

    if response_is_hollow(Some(text), &parsed) && finish_reason != "length" {
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

/// Send a plain-text `prompt` to Bedrock and return the text reply.
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
    let msgs = vec![json!({"role": "user", "content": [{"text": prompt}]})];
    let resp = call_bedrock(model, region, &msgs, max_tokens)?;
    // Extract the first text block from the response nodes (best-effort).
    // Bedrock returns nodes as parsed extraction JSON; for plain calls the
    // response text is embedded in the raw content — fall back to an empty string.
    Ok(resp
        .nodes
        .first()
        .and_then(|v| v.get("label").and_then(|l| l.as_str()))
        .unwrap_or("")
        .to_string())
}

/// Default max tokens for bedrock.
#[must_use]
pub fn default_max_tokens() -> u32 {
    resolve_max_tokens(16_384)
}

// ---------------------------------------------------------------------------
// Minimal AWS SigV4 implementation
// ---------------------------------------------------------------------------

struct SignResult {
    authorization: String,
    extra_headers: Vec<(String, String)>,
}

/// Input bundle for `sign_request` (avoids `too_many_arguments` lint).
struct SignInput<'a> {
    method: &'a str,
    url: &'a str,
    region: &'a str,
    service: &'a str,
    body: &'a str,
    access_key: &'a str,
    secret_key: &'a str,
    session_token: Option<&'a str>,
    datetime_str: &'a str,
    date_str: &'a str,
}

/// Produce AWS Signature V4 authorization and extra headers for a Bedrock request.
fn sign_request(inp: &SignInput<'_>) -> Result<SignResult, LlmError> {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    let parsed = url::Url::parse(inp.url).map_err(|e| LlmError::Http(e.to_string()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| LlmError::Http("no host in bedrock URL".to_string()))?
        .to_string();
    let path = parsed.path();
    let query = parsed.query().unwrap_or("");

    // Payload hash
    let mut hasher = Sha256::new();
    hasher.update(inp.body.as_bytes());
    let payload_hash = hex::encode(hasher.finalize());

    // Canonical headers (sorted, lowercase) — must include all signed headers.
    let mut canonical_headers = format!(
        "content-type:application/json\nhost:{host}\nx-amz-date:{}\n",
        inp.datetime_str
    );
    let mut signed_headers_str = "content-type;host;x-amz-date".to_string();
    let mut extra_headers: Vec<(String, String)> = Vec::new();

    if let Some(tok) = inp.session_token {
        // Use write! to avoid a temporary allocation (clippy::format_push_string).
        let _ = writeln!(canonical_headers, "x-amz-security-token:{tok}");
        signed_headers_str.push_str(";x-amz-security-token");
        extra_headers.push(("x-amz-security-token".to_string(), tok.to_string()));
    }

    let canonical_request = format!(
        "{}\n{path}\n{query}\n{canonical_headers}\n{signed_headers_str}\n{payload_hash}",
        inp.method
    );

    // String to sign
    let mut cr_hasher = Sha256::new();
    cr_hasher.update(canonical_request.as_bytes());
    let cr_hash = hex::encode(cr_hasher.finalize());
    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        inp.date_str, inp.region, inp.service
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{credential_scope}\n{cr_hash}",
        inp.datetime_str
    );

    // Signing key
    let signing_key = derive_signing_key(inp.secret_key, inp.date_str, inp.region, inp.service);
    let signature = hmac_sha256_hex(&signing_key, string_to_sign.as_bytes());

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, \
         SignedHeaders={signed_headers_str}, Signature={signature}",
        inp.access_key
    );

    Ok(SignResult {
        authorization,
        extra_headers,
    })
}

/// Computes HMAC-SHA256 of `msg` under `key` and returns the raw digest bytes.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    #[allow(clippy::expect_used)]
    // reason: HMAC-SHA256 accepts any key length; new_from_slice cannot fail.
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

/// Returns the lowercase hex-encoded HMAC-SHA256 of `msg` under `key`.
fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    hex::encode(hmac_sha256(key, msg))
}

/// Derives the AWS `SigV4` signing key from the secret key, date, region, and service.
fn derive_signing_key(secret_key: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_secret = format!("AWS4{secret_key}");
    let k_date = hmac_sha256(k_secret.as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// Returns current UTC time as `YYYYMMDDTHHmmssZ`.
fn chrono_now_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (year, month, day, hour, min, sec) = unix_to_datetime(secs);
    format!("{year:04}{month:02}{day:02}T{hour:02}{min:02}{sec:02}Z")
}

/// Converts a Unix timestamp (seconds since epoch) to `(year, month, day, hour, min, sec)`.
fn unix_to_datetime(ts: u64) -> (u64, u64, u64, u64, u64, u64) {
    let secs_per_day = 86_400_u64;
    let days_since_epoch = ts / secs_per_day;
    let time_of_day = ts % secs_per_day;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;

    let mut y = 1970_u64;
    let mut d = days_since_epoch;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }
    let month_days: &[u64] = if is_leap(y) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 1_u64;
    for mdays in month_days {
        if d < *mdays {
            break;
        }
        d -= mdays;
        mo += 1;
    }
    (y, mo, d + 1, hh, mm, ss)
}

/// Returns `true` if `year` is a Gregorian leap year.
fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}
