//! Import-cycles section renderer.
//!
//! Surfaces circular import dependencies detected at the file level, mirroring
//! the `## Import Cycles` block `report.generate` adds (#961).

use graphify_analyze::find_import_cycles;
use graphify_build::Graph;

/// Render the "Import Cycles" section.
///
/// Always emits the heading; lists one bullet per detected cycle (rendered as a
/// closed path `a -> b -> a`), or `- None detected.` when there are none.
pub(crate) fn render_import_cycles(lines: &mut Vec<String>, graph: &Graph) {
    lines.push(String::new());
    lines.push("## Import Cycles".to_string());

    let cycles = find_import_cycles(graph);
    if cycles.is_empty() {
        lines.push("- None detected.".to_string());
        return;
    }

    for c in &cycles {
        if c.cycle.is_empty() {
            continue;
        }
        let mut path = c.cycle.clone();
        path.push(c.cycle[0].clone());
        let cycle_path = path.join(" -> ");
        lines.push(format!("- {}-file cycle: `{cycle_path}`", c.length));
    }
}
