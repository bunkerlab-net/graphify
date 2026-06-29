//! Parity tests for `.vue` SFC extraction (#1468), ported from
//! `graphify-py/tests/test_vue_extraction.py`. The mask-internals tests are
//! exercised observably through `extract_vue`/`extract`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_extract::{FileResult, extract, extract_vue, make_id};

fn write_file(path: &Path, body: &str) -> PathBuf {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create_dir_all");
    std::fs::write(path, body).expect("write");
    path.to_path_buf()
}

/// Target ids of edges carrying `relation`.
fn targets(r: &FileResult, relation: &str) -> HashSet<String> {
    r.edges
        .iter()
        .filter(|e| e.relation == relation)
        .map(|e| e.target.clone())
        .collect()
}

/// Expected node id for an on-disk path (mirrors Python `_make_id(str(path))`).
fn id_for(path: &Path) -> String {
    make_id(&[&path.to_string_lossy()])
}

#[test]
fn vue_script_setup_ts_static_imports_resolve() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_file(
        &tmp.path().join("Child.vue"),
        "<template><div/></template>\n",
    );
    write_file(
        &tmp.path().join("utils/helper.ts"),
        "export function helper(){}\n",
    );
    let comp = write_file(
        &tmp.path().join("Comp.vue"),
        "<template>\n  <Child />\n</template>\n\n\
         <script setup lang=\"ts\">\n\
         import Child from './Child.vue'\n\
         import { helper } from './utils/helper'\n\
         helper()\n\
         </script>\n",
    );
    let result = extract_vue(&comp);
    let targets = targets(&result, "imports_from");
    assert!(
        targets.contains(&id_for(&tmp.path().join("Child.vue"))),
        "{targets:?}"
    );
    assert!(
        targets.contains(&id_for(&tmp.path().join("utils/helper.ts"))),
        "{targets:?}"
    );
}

#[test]
fn vue_script_setup_extracts_symbols_with_correct_lines() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let comp = write_file(
        &tmp.path().join("Widget.vue"),
        "<template>\n  <button @click=\"onClick\">x</button>\n</template>\n\n\
         <script setup lang=\"ts\">\n\
         import { ref } from 'vue'\n\
         \n\
         const count = ref(0)\n\
         \n\
         function onClick(): void {\n\
         \x20 count.value += 1\n\
         }\n\
         </script>\n",
    );
    let result = extract_vue(&comp);
    let count = result.nodes.iter().find(|n| n.label == "count");
    let on_click = result.nodes.iter().find(|n| n.label == "onClick()");
    assert!(count.is_some(), "no `count` node");
    assert!(on_click.is_some(), "no `onClick()` node");
    // `count` is declared on line 8, `onClick` on line 10 of the SFC (preserved
    // line numbers prove the mask kept newlines).
    assert_eq!(count.unwrap().source_location.as_deref(), Some("L8"));
    assert_eq!(on_click.unwrap().source_location.as_deref(), Some("L10"));
}

#[test]
fn vue_dynamic_import_recovered() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_file(
        &tmp.path().join("Lazy.vue"),
        "<template><div/></template>\n",
    );
    let comp = write_file(
        &tmp.path().join("Host.vue"),
        "<script setup lang=\"ts\">\n\
         import { defineAsyncComponent } from 'vue'\n\
         const Lazy = defineAsyncComponent(() => import('./Lazy.vue'))\n\
         </script>\n\n\
         <template><Lazy /></template>\n",
    );
    let result = extract_vue(&comp);
    assert!(targets(&result, "dynamic_import").contains(&id_for(&tmp.path().join("Lazy.vue"))));
}

#[test]
fn vue_plain_js_script_block() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_file(&tmp.path().join("dep.js"), "export const x = 1\n");
    let comp = write_file(
        &tmp.path().join("Legacy.vue"),
        "<script>\n\
         import { x } from './dep'\n\
         export default { name: 'Legacy' }\n\
         </script>\n\n\
         <template><div/></template>\n",
    );
    let result = extract_vue(&comp);
    assert!(targets(&result, "imports_from").contains(&id_for(&tmp.path().join("dep.js"))));
}

#[test]
fn vue_two_script_blocks_both_parsed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_file(&tmp.path().join("a.ts"), "export const a = 1\n");
    write_file(&tmp.path().join("b.ts"), "export const b = 2\n");
    let comp = write_file(
        &tmp.path().join("Dual.vue"),
        "<script lang=\"ts\">\n\
         import { a } from './a'\n\
         export default { name: 'Dual' }\n\
         </script>\n\n\
         <script setup lang=\"ts\">\n\
         import { b } from './b'\n\
         </script>\n\n\
         <template><div/></template>\n",
    );
    let result = extract_vue(&comp);
    let targets = targets(&result, "imports_from");
    assert!(
        targets.contains(&id_for(&tmp.path().join("a.ts"))),
        "{targets:?}"
    );
    assert!(
        targets.contains(&id_for(&tmp.path().join("b.ts"))),
        "{targets:?}"
    );
}

#[test]
fn vue_template_only_file_does_not_crash() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let comp = write_file(
        &tmp.path().join("Static.vue"),
        "<template>\n  <h1>hi</h1>\n</template>\n",
    );
    let result = extract_vue(&comp);
    // No `<script>` means no imports/symbols, but no crash either.
    assert!(targets(&result, "imports_from").is_empty());
}

#[test]
fn vue_whole_file_not_parsed_as_js_blob() {
    // The SFC must not be parsed as one JS blob; only the script is parsed, so its
    // imports are recovered despite invalid-JS markup.
    let tmp = tempfile::tempdir().expect("tempdir");
    write_file(&tmp.path().join("dep.ts"), "export const v = 1\n");
    let comp = write_file(
        &tmp.path().join("Guard.vue"),
        "<template>\n  <div class=\"x\" :data-y=\"z\">markup that is not valid JS</div>\n</template>\n\n\
         <script setup lang=\"ts\">\n\
         import { v } from './dep'\n\
         const z = v\n\
         </script>\n",
    );
    let result = extract_vue(&comp);
    assert!(targets(&result, "imports_from").contains(&id_for(&tmp.path().join("dep.ts"))));
}

#[test]
fn vue_generic_component_open_tag_with_angle_brackets() {
    // A Vue 3.3+ `generic=` attribute containing '>' (Record<string, unknown>)
    // must not prematurely end the <script> open tag and swallow the body (#1468).
    let tmp = tempfile::tempdir().expect("tempdir");
    write_file(
        &tmp.path().join("utils/helper.ts"),
        "export function helper(){}\n",
    );
    let comp = write_file(
        &tmp.path().join("Generic.vue"),
        "<template><div/></template>\n\
         <script setup lang=\"ts\" generic=\"T extends Record<string, unknown>\">\n\
         import { helper } from './utils/helper'\n\
         const value = helper()\n\
         </script>\n",
    );
    let result = extract_vue(&comp);
    assert!(
        targets(&result, "imports_from").contains(&id_for(&tmp.path().join("utils/helper.ts"))),
        "import inside a generic-component script body must be recovered"
    );
}

#[test]
fn vue_joins_cross_file_symbol_resolution() {
    // A `.vue` calling an imported function wires to the real symbol across files,
    // like any `.ts` file would.
    let tmp = tempfile::tempdir().expect("tempdir");
    let helper = write_file(
        &tmp.path().join("helper.ts"),
        "export function helper() {}\n",
    );
    let comp = write_file(
        &tmp.path().join("Caller.vue"),
        "<script setup lang=\"ts\">\n\
         import { helper } from './helper'\n\
         \n\
         function go(): void {\n\
         \x20 helper()\n\
         }\n\
         </script>\n\n\
         <template><div @click=\"go\" /></template>\n",
    );
    let result = extract(&[comp, helper], Some(tmp.path()));
    let by_label: std::collections::HashMap<&str, &str> = result
        .nodes
        .iter()
        .filter_map(|n| {
            Some((
                n.get("label").and_then(|v| v.as_str())?,
                n.get("id").and_then(|v| v.as_str())?,
            ))
        })
        .collect();
    let (Some(go), Some(helper_id)) = (by_label.get("go()"), by_label.get("helper()")) else {
        panic!(
            "missing go()/helper() nodes: {:?}",
            by_label.keys().collect::<Vec<_>>()
        );
    };
    let edge_exists = result.edges.iter().any(|e| {
        e.get("source").and_then(|v| v.as_str()) == Some(go)
            && e.get("target").and_then(|v| v.as_str()) == Some(helper_id)
            && e.get("relation").and_then(|v| v.as_str()) == Some("calls")
    });
    assert!(edge_exists, "go() -> helper() calls edge missing");
}
