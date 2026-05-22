//! Triage backend trait.
//!
//! The LLM-powered triage logic depends on `graphify-llm` which is being
//! ported separately. This module exposes a `TriageBackend` trait so the
//! dashboard can call into it without a hard dependency, and provides a
//! `NoOpTriageBackend` stub.

use crate::model::PrInfo;

/// Abstraction for LLM-powered triage ranking.
///
/// An implementation receives the list of actionable PRs and a prompt, and is
/// expected to stream or print ranked output.
pub trait TriageBackend {
    /// Run triage on `candidates`, printing results to stdout.
    ///
    /// # Errors
    ///
    /// Returns an error string describing the failure when the backend is
    /// unavailable or encounters an error during generation.
    fn triage(&self, candidates: &[&PrInfo], prompt: &str) -> Result<(), String>;
}

/// Stub implementation — prints a placeholder.  Replace once `graphify-llm`
/// is ported.
pub struct NoOpTriageBackend;

impl TriageBackend for NoOpTriageBackend {
    fn triage(&self, _candidates: &[&PrInfo], _prompt: &str) -> Result<(), String> {
        println!("  [triage: graphify-llm not yet wired — deferred]");
        Ok(())
    }
}

/// Build the triage prompt for a list of PR candidates.
#[must_use]
pub fn build_triage_prompt(candidates: &[&PrInfo]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for pr in candidates {
        let impact = if pr.blast_radius().is_empty() {
            String::new()
        } else {
            format!(", blast_radius={}", pr.blast_radius())
        };
        let review = if pr.review_decision.is_empty() {
            "none"
        } else {
            &pr.review_decision
        };
        lines.push(format!(
            "PR #{} [{}] CI={} review={} age={}d author={}{}\n  title: {}",
            pr.number,
            pr.status(),
            pr.ci_status,
            review,
            pr.days_old(),
            pr.author,
            impact,
            pr.title,
        ));
    }
    format!(
        "You are a senior engineer helping triage a PR review queue. \
Given these open PRs, rank them by review priority for the repo maintainer. \
For each PR give: priority number, one sentence on what action to take and why. \
Be direct and specific. Format each as: #<number> — <action>.\n\n{}",
        lines.join("\n\n")
    )
}
