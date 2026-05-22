// crate_b has no dependency on crate_a.
// These calls use Type::method() (scoped_identifier) and common names that
// previously produced spurious INFERRED edges into crate_a (#908).

// Local stub so the fixture parses *and* compiles cleanly. Tree-sitter
// only looks at the source text, but `cargo clippy` (run by pre-commit
// hooks) walks every Cargo.toml and refuses unresolved paths.
pub struct Url;

impl Url {
    pub fn parse(_: &str) -> bool {
        false
    }
}

pub struct Server;

impl Server {
    pub fn run(&self) {
        // Server::start() — scoped call, should not wire to crate_a::start
        let _ = Server::start();
        // Url::parse() — scoped call, should not wire to crate_a::parse
        let _ = Url::parse("http://example.com");
    }

    fn start() -> bool {
        false
    }
}
