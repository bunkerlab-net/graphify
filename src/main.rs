//! `graphify` CLI binary.
//!
//! Ports `graphify-py/graphify/__main__.py`. All orchestration lives in
//! [`cli::run`]; this file is intentionally just an entry point so the
//! testable surface stays inside the `cli` module tree.

mod cli;

fn main() -> anyhow::Result<()> {
    cli::run()
}
