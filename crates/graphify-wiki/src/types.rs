//! Public data types used by the wiki generator.

/// Structured data for a god node passed to [`crate::to_wiki`].
///
/// A *god node* is a highly connected concept whose article anchors a wiki
/// section. The fields here are the minimum required to render the article
/// and the corresponding index entry.
#[derive(Debug, Clone)]
pub struct GodNodeData {
    /// Node ID in the graph.
    pub id: String,
    /// Display label for the article title.
    pub label: String,
    /// Pre-computed connection degree (used in index listing).
    pub degree: usize,
}
