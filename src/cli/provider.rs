//! `provider` command — manage custom LLM providers in
//! `~/.graphify/providers.json` (#1084).
//!
//! Ports the `provider [add|list|show|remove]` block from
//! `graphify-py/graphify/__main__.py`.

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use crate::cli::args::ProviderCommand;

/// Dispatch a `provider` subcommand.
pub(crate) fn cmd_provider(cmd: ProviderCommand) -> Result<()> {
    match cmd {
        ProviderCommand::List => list(),
        ProviderCommand::Show { name } => show(&name),
        ProviderCommand::Add {
            name,
            base_url,
            default_model,
            env_key,
            pricing_input,
            pricing_output,
        } => add(
            &name,
            base_url.as_deref(),
            default_model.as_deref(),
            env_key.as_deref(),
            pricing_input.unwrap_or(0.0),
            pricing_output.unwrap_or(0.0),
        ),
        ProviderCommand::Remove { name } => remove(&name),
    }
}

/// Load the global registry as an ordered JSON object. A missing file yields an
/// empty registry; a file that exists but is unreadable or not a JSON object is
/// an error, so `add`/`remove` never clobber a malformed file (a divergence from
/// graphify-py, which silently overwrites it).
fn load_registry() -> Result<Map<String, Value>> {
    let path = graphify_llm::custom_providers_path(true);
    if !path.is_file() {
        return Ok(Map::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow!("cannot read {}: {e}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| anyhow!("malformed providers.json at {}: {e}", path.display()))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("providers.json at {} is not a JSON object", path.display()))
}

/// Write the registry back, pretty-printed with a trailing newline (matches Python).
fn save_registry(registry: &Map<String, Value>) -> Result<()> {
    let path = graphify_llm::custom_providers_path(true);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(registry)?;
    // Write to a sibling temp file then rename, so a crash mid-write can't leave
    // a truncated/corrupt registry. `std::fs::rename` replaces an existing
    // destination on both Unix and Windows, but the exact atomicity / overwrite
    // semantics are platform- and filesystem-dependent (e.g. Windows needs
    // specific kernel support for POSIX-style atomic replace), so this is a
    // best-effort guard rather than a hard atomicity guarantee.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, format!("{body}\n"))?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        // Don't leave the orphaned temp file behind on a failed rename.
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

fn list() -> Result<()> {
    let registry = load_registry()?;
    if registry.is_empty() {
        outln!("No custom providers registered.");
    } else {
        for (name, cfg) in &registry {
            let base = cfg
                .get("base_url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            outln!("  {name}  ({base})");
        }
    }
    Ok(())
}

fn show(name: &str) -> Result<()> {
    let registry = load_registry()?;
    let Some(cfg) = registry.get(name) else {
        return Err(anyhow!("Provider '{name}' not found."));
    };
    let mut single = Map::new();
    single.insert(name.to_string(), cfg.clone());
    outln!("{}", serde_json::to_string_pretty(&Value::Object(single))?);
    Ok(())
}

fn add(
    name: &str,
    base_url: Option<&str>,
    default_model: Option<&str>,
    env_key: Option<&str>,
    pricing_input: f64,
    pricing_output: f64,
) -> Result<()> {
    if graphify_llm::is_builtin_backend(name) {
        return Err(anyhow!(
            "Error: '{name}' is a built-in provider and cannot be overridden."
        ));
    }
    let (Some(base_url), Some(default_model), Some(env_key)) = (
        base_url.filter(|s| !s.is_empty()),
        default_model.filter(|s| !s.is_empty()),
        env_key.filter(|s| !s.is_empty()),
    ) else {
        return Err(anyhow!(
            "Error: --base-url, --default-model, and --env-key are required."
        ));
    };

    // Reject NaN/+-Inf pricing up front: `serde_json` serializes non-finite
    // floats to `null`, which the loader then reads back as the 0.0 default, so
    // an invalid price would be silently lost rather than stored.
    if !pricing_input.is_finite() || !pricing_output.is_finite() {
        return Err(anyhow!(
            "Error: --pricing-input and --pricing-output must be finite numbers."
        ));
    }

    let mut registry = load_registry()?;
    registry.insert(
        name.to_string(),
        json!({
            "base_url": base_url,
            "default_model": default_model,
            "env_key": env_key,
            "pricing": {"input": pricing_input, "output": pricing_output},
            "temperature": 0,
        }),
    );
    save_registry(&registry)?;
    outln!("Provider '{name}' added. Use with: graphify extract . --backend {name}");
    Ok(())
}

fn remove(name: &str) -> Result<()> {
    let mut registry = load_registry()?;
    if registry.shift_remove(name).is_none() {
        return Err(anyhow!("Provider '{name}' not found."));
    }
    save_registry(&registry)?;
    outln!("Provider '{name}' removed.");
    Ok(())
}
