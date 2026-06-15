//! 1:1 port of `graphify-py/tests/test_terraform.py`.
//!
//! Exercises the Terraform/HCL extractor: every block type becomes a node,
//! interpolations become `references`/`depends_on` edges, meta heads are
//! suppressed, and directory-scoped IDs let cross-file references resolve at
//! build time.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use graphify_extract::{FileResult, extract_terraform};

const SAMPLE: &str = r#"# leading comment so the body is not children[0]
terraform {
  required_providers { azurerm = { source = "hashicorp/azurerm" } }
}

variable "region" { default = "us-east-1" }

provider "aws" { region = var.region }

data "aws_ami" "ubuntu" { most_recent = true }

resource "aws_instance" "web" {
  ami       = data.aws_ami.ubuntu.id
  subnet_id = var.region
  depends_on = [aws_security_group.sg]
}

resource "aws_security_group" "sg" { name = "sg" }

module "vpc" {
  source = "./modules/vpc"
  cidr   = local.cidr
}

locals { cidr = "10.0.0.0/16" }

output "ip" { value = aws_instance.web.private_ip }
"#;

/// Write `body` to `<dir>/<name>` and return the path.
fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write tf");
    p
}

fn labels(r: &FileResult) -> Vec<&str> {
    r.nodes.iter().map(|n| n.label.as_str()).collect()
}

/// `(source_label, target_label)` pairs for a relation, mirroring `_rel_pairs`.
fn rel_pairs(r: &FileResult, relation: &str) -> HashSet<(String, String)> {
    let lab: HashMap<&str, &str> = r
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    r.edges
        .iter()
        .filter(|e| e.relation == relation)
        .map(|e| {
            (
                (*lab.get(e.source.as_str()).unwrap_or(&e.source.as_str())).to_string(),
                (*lab.get(e.target.as_str()).unwrap_or(&e.target.as_str())).to_string(),
            )
        })
        .collect()
}

/// `test_no_error_and_all_block_types_become_nodes`
#[test]
fn no_error_and_all_block_types_become_nodes() {
    let tmp = tempfile::tempdir().unwrap();
    let r = extract_terraform(&write(tmp.path(), "main.tf", SAMPLE));
    assert!(r.error.is_none(), "{:?}", r.error);
    let ls: HashSet<&str> = labels(&r).into_iter().collect();
    for expected in [
        "var.region",
        "provider.aws",
        "data.aws_ami.ubuntu",
        "aws_instance.web",
        "aws_security_group.sg",
        "module.vpc",
        "local.cidr",
        "output.ip",
    ] {
        assert!(ls.contains(expected), "missing node {expected:?}");
    }
}

/// `test_reference_edges`
#[test]
fn reference_edges() {
    let tmp = tempfile::tempdir().unwrap();
    let r = extract_terraform(&write(tmp.path(), "main.tf", SAMPLE));
    let refs = rel_pairs(&r, "references");
    for (s, t) in [
        ("provider.aws", "var.region"),
        ("aws_instance.web", "data.aws_ami.ubuntu"),
        ("aws_instance.web", "var.region"),
        ("module.vpc", "local.cidr"),
        ("output.ip", "aws_instance.web"),
    ] {
        assert!(
            refs.contains(&(s.to_string(), t.to_string())),
            "missing reference {s} -> {t}: {refs:?}"
        );
    }
}

/// `test_depends_on_edge`
#[test]
fn depends_on_edge() {
    let tmp = tempfile::tempdir().unwrap();
    let r = extract_terraform(&write(tmp.path(), "main.tf", SAMPLE));
    assert!(rel_pairs(&r, "depends_on").contains(&(
        "aws_instance.web".to_string(),
        "aws_security_group.sg".to_string()
    )));
}

/// `test_file_contains_blocks`
#[test]
fn file_contains_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let r = extract_terraform(&write(tmp.path(), "main.tf", SAMPLE));
    let contains = rel_pairs(&r, "contains");
    assert!(contains.contains(&("main.tf".to_string(), "aws_instance.web".to_string())));
    assert!(contains.contains(&("main.tf".to_string(), "var.region".to_string())));
}

/// `test_meta_heads_not_emitted`
#[test]
fn meta_heads_not_emitted() {
    let tmp = tempfile::tempdir().unwrap();
    let body = "resource \"aws_instance\" \"web\" {\n  count = 2\n  name  = \"web-${count.index}\"\n  tags  = each.value\n  dir   = path.module\n}\n";
    let r = extract_terraform(&write(tmp.path(), "main.tf", body));
    let targets: HashSet<String> = rel_pairs(&r, "references")
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    assert!(
        !targets
            .iter()
            .any(|t| t.starts_with("count") || t.starts_with("each") || t.starts_with("path")),
        "meta heads leaked into references: {targets:?}"
    );
}

/// `test_cross_file_references_resolve_after_merge`
#[test]
fn cross_file_references_resolve_after_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let defn = "resource \"azurerm_resource_group\" \"main\" { name = \"rg\" }\n";
    let user = "resource \"azurerm_network_interface\" \"nic\" {\n  resource_group_name = azurerm_resource_group.main.name\n}\n";
    let r_defn = extract_terraform(&write(tmp.path(), "main.tf", defn));
    let r_user = extract_terraform(&write(tmp.path(), "nic.tf", user));

    // The cross-file edge target id equals the definition's node id.
    let rg_id = r_defn
        .nodes
        .iter()
        .find(|n| n.label == "azurerm_resource_group.main")
        .map(|n| n.id.clone())
        .expect("rg node");
    let nic_ref_targets: HashSet<&str> = r_user
        .edges
        .iter()
        .filter(|e| e.relation == "references")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        nic_ref_targets.contains(rg_id.as_str()),
        "cross-file ref target mismatch"
    );

    // And it survives a real merge: the edge is present, not dropped as dangling.
    let mut nodes = r_defn.nodes.clone();
    nodes.extend(r_user.nodes.clone());
    let mut edges = r_defn.edges.clone();
    edges.extend(r_user.edges.clone());
    let extraction = serde_json::json!({ "nodes": nodes, "edges": edges });
    let graph = graphify_build::build_from_json(extraction, false, None).expect("build_from_json");
    let nic_id = r_user
        .nodes
        .iter()
        .find(|n| n.label == "azurerm_network_interface.nic")
        .map(|n| n.id.clone())
        .expect("nic node");
    assert!(
        graph
            .edge_list
            .iter()
            .any(|e| e.source == nic_id && e.target == rg_id),
        "merged graph is missing the cross-file edge {nic_id} -> {rg_id}"
    );
}

/// `test_empty_and_commentonly_files_are_safe`
#[test]
fn empty_and_commentonly_files_are_safe() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        extract_terraform(&write(tmp.path(), "a.tf", ""))
            .error
            .is_none()
    );
    let r = extract_terraform(&write(tmp.path(), "b.tf", "# just a comment\n"));
    assert_eq!(r.nodes.len(), 1, "only the file node expected");
}

/// `test_tfvars_key_value_is_safe`
#[test]
fn tfvars_key_value_is_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let r = extract_terraform(&write(
        tmp.path(),
        "terraform.tfvars",
        "region = \"us-east-1\"\nenv = \"prod\"\n",
    ));
    assert!(r.error.is_none());
    assert_eq!(r.nodes.len(), 1, "tfvars yields only the file node");
}
