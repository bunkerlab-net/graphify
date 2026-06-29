//! Parity tests for the WPF/XAML extractor (#1460, #1473), ported from
//! `graphify-py/tests/test_dotnet.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_extract::types::{Edge, FileResult, Node};
use graphify_extract::{ExtractOutput, extract, extract_xaml};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// `references`/`view_model` edges.
fn vm_edges(r: &FileResult) -> Vec<&Edge> {
    r.edges
        .iter()
        .filter(|e| e.relation == "references" && e.context.as_deref() == Some("view_model"))
        .collect()
}

fn node_by_id<'a>(r: &'a FileResult, id: &str) -> Option<&'a Node> {
    r.nodes.iter().find(|n| n.id == id)
}

fn labels(r: &FileResult) -> HashSet<&str> {
    r.nodes.iter().map(|n| n.label.as_str()).collect()
}

fn event_targets(r: &FileResult) -> HashSet<&str> {
    r.edges
        .iter()
        .filter(|e| e.relation == "references" && e.context.as_deref() == Some("event"))
        .map(|e| e.target.as_str())
        .collect()
}

/// Recursively copy a directory tree (the fixtures' `xaml_viewmodel` project).
fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("mkdir");
    for entry in std::fs::read_dir(src).expect("read_dir").flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).expect("copy");
        }
    }
}

#[test]
fn xaml_class_resolves_to_codebehind_partial_class() {
    let r = extract_xaml(&fixtures().join("sample.xaml"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let class_nodes: Vec<&Node> = r
        .nodes
        .iter()
        .filter(|n| n.label == "MainWindow" && n.source_file.ends_with("sample.xaml.cs"))
        .collect();
    assert!(
        !class_nodes.is_empty(),
        "no code-behind MainWindow class node"
    );
    assert!(r.edges.iter().any(|e| {
        e.relation == "references"
            && e.context.as_deref() == Some("x_class")
            && e.target == class_nodes[0].id
    }));
}

#[test]
fn xaml_named_controls_and_bindings() {
    let r = extract_xaml(&fixtures().join("sample.xaml"));
    let labels = labels(&r);
    for want in ["RootPanel", "UserNameBox", "SaveButton", "UserName"] {
        assert!(labels.contains(want), "missing {want}: {labels:?}");
    }
    assert!(
        r.edges.iter().any(|e| {
            e.relation == "references" && e.context.as_deref() == Some("binding_path")
        })
    );
}

#[test]
fn xaml_extracts_binding_paths_commands_and_converters() {
    let r = extract_xaml(&fixtures().join("bindings.xaml"));
    let labels_by_id: std::collections::HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let refs: HashSet<(&str, Option<&str>)> = r
        .edges
        .iter()
        .filter(|e| e.relation == "references")
        .filter_map(|e| Some((*labels_by_id.get(e.target.as_str())?, e.context.as_deref())))
        .collect();
    assert!(
        refs.contains(&("User.Name", Some("binding_path"))),
        "{refs:?}"
    );
    assert!(refs.contains(&("Order.Total", Some("binding_path"))));
    assert!(refs.contains(&("Invoice.Tax", Some("binding_path"))));
    assert!(refs.contains(&("SaveCommand", Some("binding_command"))));
    assert!(refs.contains(&("MoneyConverter", Some("binding_converter"))));
    assert!(refs.contains(&("TaxConverter", Some("binding_converter"))));
    assert!(!refs.contains(&("TwoWay", Some("binding_path"))));
}

#[test]
fn xaml_element_datacontext_links_real_viewmodel_class() {
    let r = extract_xaml(&fixtures().join("xaml_viewmodel/Views/ExplicitMainWindow.xaml"));
    let edges = vm_edges(&r);
    assert_eq!(edges.len(), 1, "{:?}", r.edges);
    assert_eq!(edges[0].confidence, "EXTRACTED");
    let target = node_by_id(&r, &edges[0].target).expect("vm node");
    assert_eq!(target.label, "MainViewModel");
    assert!(target.source_file.ends_with("MainViewModel.cs"));
}

#[test]
fn xaml_design_instance_datacontext_links_real_viewmodel_class() {
    let r = extract_xaml(&fixtures().join("xaml_viewmodel/Views/DesignView.xaml"));
    let edges = vm_edges(&r);
    assert_eq!(edges.len(), 1, "{:?}", r.edges);
    assert_eq!(edges[0].confidence, "EXTRACTED");
    assert_eq!(
        node_by_id(&r, &edges[0].target).unwrap().label,
        "DesignViewModel"
    );
}

#[test]
fn xaml_infers_viewmodel_by_name_only_without_datacontext() {
    let r = extract_xaml(&fixtures().join("xaml_viewmodel/Views/SettingsView.xaml"));
    let edges = vm_edges(&r);
    assert_eq!(edges.len(), 1, "{:?}", r.edges);
    assert_eq!(edges[0].confidence, "INFERRED");
    assert_eq!(
        node_by_id(&r, &edges[0].target).unwrap().label,
        "SettingsViewModel"
    );
}

#[test]
fn xaml_prism_autowire_infers_viewmodel_from_filename() {
    let r = extract_xaml(&fixtures().join("xaml_viewmodel/Views/PrismOrderView.xaml"));
    let edges = vm_edges(&r);
    assert_eq!(edges.len(), 1, "{:?}", r.edges);
    assert_eq!(edges[0].confidence, "INFERRED");
    assert_eq!(
        node_by_id(&r, &edges[0].target).unwrap().label,
        "PrismOrderViewModel"
    );
}

#[test]
fn xaml_prism_autowire_false_does_not_infer_from_filename() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("xaml_viewmodel");
    copy_tree(&fixtures().join("xaml_viewmodel"), &project);
    let xaml = project.join("Views/PrismOrderView.xaml");
    let src = std::fs::read_to_string(&xaml).unwrap();
    std::fs::write(
        &xaml,
        src.replace("AutoWireViewModel=\"True\"", "AutoWireViewModel=\"False\""),
    )
    .unwrap();
    let r = extract_xaml(&xaml);
    assert!(vm_edges(&r).is_empty());
}

#[test]
fn xaml_links_communitytoolkit_generated_members_and_event_to_command() {
    let r = extract_xaml(&fixtures().join("xaml_viewmodel/Views/ToolkitView.xaml"));
    let nodes: std::collections::HashMap<&str, &Node> =
        r.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let generated_defs: HashSet<(&str, Option<&str>)> = r
        .edges
        .iter()
        .filter(|e| e.relation == "defines")
        .filter_map(|e| {
            Some((
                nodes.get(e.target.as_str())?.label.as_str(),
                e.context.as_deref(),
            ))
        })
        .collect();
    assert!(generated_defs.contains(&("UserName", Some("communitytoolkit_observable_property"))));
    assert!(generated_defs.contains(&("Email", Some("communitytoolkit_observable_property"))));
    assert!(generated_defs.contains(&("SaveCommand", Some("communitytoolkit_relay_command"))));
    assert!(generated_defs.contains(&("RefreshCommand", Some("communitytoolkit_relay_command"))));
    assert!(
        !generated_defs.contains(&("IgnoredName", Some("communitytoolkit_observable_property")))
    );
    assert!(!generated_defs.contains(&("IgnoredCommand", Some("communitytoolkit_relay_command"))));

    // The binding references resolve to the generated members (INFERRED).
    let inferred_ref = |label: &str, ctx: &str| {
        r.edges.iter().any(|e| {
            e.relation == "references"
                && e.context.as_deref() == Some(ctx)
                && e.confidence == "INFERRED"
                && nodes.get(e.target.as_str()).is_some_and(|n| {
                    n.label == label && n.source_file.ends_with("ToolkitViewModel.cs")
                })
        })
    };
    assert!(inferred_ref("UserName", "binding_path"));
    assert!(inferred_ref("SaveCommand", "binding_command"));
    assert!(inferred_ref("Email", "binding_path"));
    assert!(inferred_ref("RefreshCommand", "binding_command"));
}

#[test]
fn extract_preserves_xaml_viewmodel_edge_after_id_remap() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("xaml_viewmodel");
    copy_tree(&fixtures().join("xaml_viewmodel"), &project);
    let mut files: Vec<PathBuf> = Vec::new();
    collect(&project, "xaml", &mut files);
    collect(&project, "cs", &mut files);
    let r = extract(&files, Some(&project));
    let vm_labels = view_model_target_labels(&r);
    assert!(
        vm_labels.iter().any(|l| l == "MainViewModel"),
        "{vm_labels:?}"
    );
    assert!(
        vm_labels.iter().any(|l| l == "DesignViewModel"),
        "{vm_labels:?}"
    );
    // SettingsViewModel is the INFERRED case and must survive the id remap too.
    assert!(
        r.edges.iter().any(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("references")
                && e.get("context").and_then(|v| v.as_str()) == Some("view_model")
                && e.get("confidence").and_then(|v| v.as_str()) == Some("INFERRED")
                && out_node_label(&r, e.get("target").and_then(|v| v.as_str()).unwrap_or(""))
                    == Some("SettingsViewModel".to_string())
        }),
        "SettingsViewModel inferred edge missing"
    );
}

#[test]
fn extract_xaml_viewmodel_resolution_stays_inside_cache_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("xaml_viewmodel");
    copy_tree(&fixtures().join("xaml_viewmodel"), &project);
    // cache_root = Views/, so the ViewModel scan (which lives in ../ViewModels)
    // is out of bounds and resolves no edge.
    let r = extract(
        &[project.join("Views/ExplicitMainWindow.xaml")],
        Some(&project.join("Views")),
    );
    assert!(view_model_target_labels(&r).is_empty(), "{:?}", r.edges);
}

#[test]
fn xaml_viewmodel_resolution_respects_graphifyignore() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("xaml_viewmodel");
    copy_tree(&fixtures().join("xaml_viewmodel"), &project);
    std::fs::write(
        project.join(".graphifyignore"),
        "ViewModels/MainViewModel.cs\n",
    )
    .unwrap();
    let r = extract_xaml(&project.join("Views/ExplicitMainWindow.xaml"));
    assert!(vm_edges(&r).is_empty());
}

#[test]
fn xaml_ambiguous_viewmodel_names_emit_no_edge() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("Views")).unwrap();
    std::fs::create_dir_all(tmp.path().join("ViewModels")).unwrap();
    std::fs::write(
        tmp.path().join("App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\" />",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("Views/MainWindow.xaml"),
        "<Window x:Class=\"Demo.MainWindow\"\n  xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n  xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\">\n</Window>\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("ViewModels/MainWindowViewModel.cs"),
        "namespace Demo { public class MainWindowViewModel { } }\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("ViewModels/MainViewModel.cs"),
        "namespace Demo { public class MainViewModel { } }\n",
    )
    .unwrap();
    let r = extract_xaml(&tmp.path().join("Views/MainWindow.xaml"));
    assert!(vm_edges(&r).is_empty());
}

#[test]
fn xaml_events_resolve_to_codebehind_methods() {
    let r = extract_xaml(&fixtures().join("sample.xaml"));
    let method_nodes: std::collections::HashMap<String, &str> = r
        .nodes
        .iter()
        .filter(|n| n.source_file.ends_with("sample.xaml.cs"))
        .map(|n| {
            (
                n.label
                    .trim_matches(|c| c == '(' || c == ')')
                    .trim_start_matches('.')
                    .to_string(),
                n.id.as_str(),
            )
        })
        .collect();
    for want in ["Window_Loaded", "UserNameChanged", "Save_Click"] {
        assert!(method_nodes.contains_key(want), "missing method {want}");
    }
    let targets = event_targets(&r);
    assert!(targets.contains(method_nodes["Window_Loaded"]));
    assert!(targets.contains(method_nodes["UserNameChanged"]));
    assert!(targets.contains(method_nodes["Save_Click"]));
}

#[test]
fn xaml_event_match_requires_handler_signature() {
    // A value matching an ordinary method's name must not become an event edge —
    // only methods with a (object sender, ...EventArgs e) signature do.
    let tmp = tempfile::tempdir().expect("tempdir");
    let xaml = tmp.path().join("view.xaml");
    std::fs::write(
        &xaml,
        "<Window x:Class=\"Demo.MainWindow\"\n  xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n  xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\">\n  <Button Content=\"Refresh\" Click=\"Refresh\"/>\n</Window>\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("view.xaml.cs"),
        "using System.Windows;\nnamespace Demo { public partial class MainWindow : Window {\n  public void Refresh() {}\n}}\n",
    )
    .unwrap();
    let r = extract_xaml(&xaml);
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(event_targets(&r).is_empty());
}

#[test]
fn xaml_non_event_attribute_value_does_not_fabricate_event() {
    // Content=/Tag= holding a real handler's name must not create an event edge;
    // only the genuine event attribute (Click) should.
    let tmp = tempfile::tempdir().expect("tempdir");
    let xaml = tmp.path().join("view.xaml");
    std::fs::write(
        &xaml,
        "<Window x:Class=\"Demo.MainWindow\"\n  xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\"\n  xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\">\n  <Button x:Name=\"B1\" Content=\"Save_Click\" Tag=\"OnLoaded\" Click=\"Save_Click\"/>\n</Window>\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("view.xaml.cs"),
        "using System.Windows;\nnamespace Demo { public partial class MainWindow : Window {\n  private void Save_Click(object sender, RoutedEventArgs e) {}\n  private void OnLoaded(object sender, RoutedEventArgs e) {}\n}}\n",
    )
    .unwrap();
    let r = extract_xaml(&xaml);
    let handlers: std::collections::HashMap<String, &str> = r
        .nodes
        .iter()
        .filter(|n| n.source_file.ends_with("view.xaml.cs"))
        .map(|n| {
            (
                n.label
                    .trim_matches(|c| c == '(' || c == ')')
                    .trim_start_matches('.')
                    .to_string(),
                n.id.as_str(),
            )
        })
        .collect();
    let targets = event_targets(&r);
    assert!(targets.contains(handlers["Save_Click"]));
    assert!(
        handlers
            .get("OnLoaded")
            .is_none_or(|id| !targets.contains(id))
    );
    assert_eq!(targets.len(), 1);
}

#[test]
fn xaml_viewmodel_with_non_utf8_codebehind_does_not_crash() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("xaml_viewmodel");
    copy_tree(&fixtures().join("xaml_viewmodel"), &project);
    let vm = project.join("ViewModels/SettingsViewModel.cs");
    let mut bytes = b"\xff// stray byte\n".to_vec();
    bytes.extend_from_slice(&std::fs::read(&vm).unwrap());
    std::fs::write(&vm, bytes).unwrap();
    let r = extract_xaml(&project.join("Views/SettingsView.xaml"));
    assert!(r.error.is_none(), "{:?}", r.error);
    let edges = vm_edges(&r);
    assert_eq!(edges.len(), 1);
    assert_eq!(
        node_by_id(&r, &edges[0].target).unwrap().label,
        "SettingsViewModel"
    );
}

// ── ExtractOutput helpers (for the `extract()` pipeline tests) ────────────────

fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect(&p, ext, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(p);
        }
    }
    out.sort();
}

fn out_node_label(r: &ExtractOutput, id: &str) -> Option<String> {
    r.nodes
        .iter()
        .find(|n| n.get("id").and_then(|v| v.as_str()) == Some(id))
        .and_then(|n| n.get("label").and_then(|v| v.as_str()).map(str::to_string))
}

fn view_model_target_labels(r: &ExtractOutput) -> Vec<String> {
    r.edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("references")
                && e.get("context").and_then(|v| v.as_str()) == Some("view_model")
        })
        .filter_map(|e| out_node_label(r, e.get("target").and_then(|v| v.as_str()).unwrap_or("")))
        .collect()
}
