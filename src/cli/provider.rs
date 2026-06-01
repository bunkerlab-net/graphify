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
        ProviderCommand::List => {
            list();
            Ok(())
        }
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

/// Load the global registry as an ordered JSON object (empty on missing/invalid).
fn load_registry() -> Map<String, Value> {
    let path = graphify_llm::custom_providers_path(true);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Write the registry back, pretty-printed with a trailing newline (matches Python).
fn save_registry(registry: &Map<String, Value>) -> Result<()> {
    let path = graphify_llm::custom_providers_path(true);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(&Value::Object(registry.clone()))?;
    std::fs::write(&path, format!("{body}\n"))?;
    Ok(())
}

fn list() {
    let registry = load_registry();
    if registry.is_empty() {
        println!("No custom providers registered.");
    } else {
        for (name, cfg) in &registry {
            let base = cfg
                .get("base_url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            println!("  {name}  ({base})");
        }
    }
}

fn show(name: &str) -> Result<()> {
    let registry = load_registry();
    let Some(cfg) = registry.get(name) else {
        return Err(anyhow!("Provider '{name}' not found."));
    };
    let mut single = Map::new();
    single.insert(name.to_string(), cfg.clone());
    println!("{}", serde_json::to_string_pretty(&Value::Object(single))?);
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

    let mut registry = load_registry();
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
    println!("Provider '{name}' added. Use with: graphify extract . --backend {name}");
    Ok(())
}

fn remove(name: &str) -> Result<()> {
    let mut registry = load_registry();
    if registry.shift_remove(name).is_none() {
        return Err(anyhow!("Provider '{name}' not found."));
    }
    save_registry(&registry)?;
    println!("Provider '{name}' removed.");
    Ok(())
}
