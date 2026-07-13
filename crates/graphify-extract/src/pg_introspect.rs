//! `PostgreSQL` introspection: reconstruct DDL from the live catalog and extract
//! it via the SQL extractor.
//!
//! Mirrors `graphify-py/graphify/pg_introspect.py`. The pure DDL-reconstruction
//! core (`build_ddl` / `introspect_catalog`) is always compiled and unit-tested
//! against fixture catalogs. The live connection (`introspect_postgres`) is
//! gated behind the optional `postgres` feature — matching graphify-py, where
//! `--postgres` requires the `graphify[postgres]` extra.

use std::path::PathBuf;

use crate::extractors::extract_sql_with_content;
use crate::types::FileResult;

/// Error raised while introspecting a `PostgreSQL` database.
#[derive(Debug, thiserror::Error)]
pub enum PgIntrospectError {
    /// The connection could not be established (message is sanitized — no DSN).
    #[error("{0}")]
    Connection(String),
    /// A catalog query failed.
    #[error("query failed: {0}")]
    Query(String),
}

/// A `information_schema.tables` row.
#[derive(Debug, Clone)]
pub struct PgTable {
    /// Schema the table lives in.
    pub schema: String,
    /// Table name.
    pub name: String,
    /// `table_type`, e.g. `"BASE TABLE"` or `"VIEW"`.
    pub table_type: String,
}

/// A `information_schema.views` row.
#[derive(Debug, Clone)]
pub struct PgView {
    /// Schema the view lives in.
    pub schema: String,
    /// View name.
    pub name: String,
    /// `view_definition`, or `None` when permission was denied.
    pub definition: Option<String>,
}

/// A `information_schema.routines` row.
#[derive(Debug, Clone)]
pub struct PgRoutine {
    /// Schema the routine lives in.
    pub schema: String,
    /// Routine name.
    pub name: String,
    /// `routine_type`, e.g. `"FUNCTION"` or `"PROCEDURE"`.
    pub routine_type: String,
    /// `routine_definition`, or `None` when unavailable.
    pub definition: Option<String>,
    /// `external_language`, or `None`.
    pub external_language: Option<String>,
}

/// A foreign-key constraint, grouped across its (possibly composite) columns.
#[derive(Debug, Clone)]
pub struct PgForeignKey {
    /// Constraint name.
    pub constraint_name: String,
    /// Referencing table's schema.
    pub table_schema: String,
    /// Referencing table's name.
    pub table_name: String,
    /// Referencing columns, ordered by position.
    pub columns: Vec<String>,
    /// Referenced table's schema.
    pub foreign_table_schema: String,
    /// Referenced table's name.
    pub foreign_table_name: String,
    /// Referenced columns, ordered by position.
    pub foreign_columns: Vec<String>,
}

/// The catalog data needed to reconstruct DDL.
#[derive(Debug, Clone, Default)]
pub struct PgCatalog {
    /// `information_schema.tables` rows.
    pub tables: Vec<PgTable>,
    /// `information_schema.views` rows.
    pub views: Vec<PgView>,
    /// `information_schema.routines` rows.
    pub routines: Vec<PgRoutine>,
    /// Foreign-key constraints (one per constraint, composites grouped).
    pub foreign_keys: Vec<PgForeignKey>,
}

/// Double-quote a `PostgreSQL` identifier, escaping embedded double-quotes.
/// Mirrors `_quote_ident`.
#[must_use]
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Wrap a connection error message: keep only the first line (drops the DSN /
/// `DETAIL:` noise psycopg/libpq append) behind a stable prefix. Mirrors the
/// sanitization in `introspect_postgres`.
#[must_use]
pub fn sanitize_connection_error(raw: &str) -> String {
    let first_line = raw.split('\n').next().unwrap_or(raw);
    format!("could not connect to PostgreSQL: {first_line}")
}

/// Reconstruct CREATE/ALTER DDL from catalog data. Mirrors the DDL-building
/// loops in `introspect_postgres`.
#[must_use]
pub fn build_ddl(catalog: &PgCatalog) -> String {
    let mut ddl: Vec<String> = Vec::new();

    // Tables — quote identifiers to handle reserved words, hyphens, mixed-case.
    for t in &catalog.tables {
        if t.table_type == "BASE TABLE" {
            ddl.push(format!(
                "CREATE TABLE {}.{} (id INT);",
                quote_ident(&t.schema),
                quote_ident(&t.name)
            ));
        }
    }

    // Views — real body if available, stub if NULL (permission denied).
    for v in &catalog.views {
        let sig = format!("{}.{}", quote_ident(&v.schema), quote_ident(&v.name));
        match v.definition.as_deref().filter(|b| !b.is_empty()) {
            Some(body) => ddl.push(format!("CREATE VIEW {sig} AS {body};")),
            None => ddl.push(format!("CREATE VIEW {sig} AS SELECT 1;")),
        }
    }

    // Functions & procedures — real body if available, stub if NULL. `$gfx$` is
    // the dollar-quote tag (avoids collision with `$$` inside bodies); procedures
    // are represented as FUNCTION so tree-sitter can parse them.
    for r in &catalog.routines {
        let lang = r
            .external_language
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("plpgsql")
            .to_lowercase();
        let fn_sig = format!("{}.{}()", quote_ident(&r.schema), quote_ident(&r.name));
        if r.routine_type == "FUNCTION" || r.routine_type == "PROCEDURE" {
            let body = r
                .definition
                .as_deref()
                .filter(|b| !b.is_empty())
                .unwrap_or("BEGIN SELECT 1; END;");
            ddl.push(format!(
                "CREATE FUNCTION {fn_sig} RETURNS void AS $gfx$ {body} $gfx$ LANGUAGE {lang};"
            ));
        }
    }

    // FK edges — one ALTER TABLE per constraint (handles composite FKs).
    for fk in &catalog.foreign_keys {
        let col_list = fk
            .columns
            .iter()
            .map(|c| quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ");
        let ref_col_list = fk
            .foreign_columns
            .iter()
            .map(|c| quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ");
        ddl.push(format!(
            "ALTER TABLE {}.{} ADD CONSTRAINT {} FOREIGN KEY ({col_list}) REFERENCES {}.{}({ref_col_list});",
            quote_ident(&fk.table_schema),
            quote_ident(&fk.table_name),
            quote_ident(&fk.constraint_name),
            quote_ident(&fk.foreign_table_schema),
            quote_ident(&fk.foreign_table_name),
        ));
    }

    ddl.join("\n")
}

/// Reconstruct DDL from `catalog` and extract it as a graph, attributing nodes
/// and edges to a sanitized virtual `postgresql:/<host>/<dbname>` path (no
/// credentials). Mirrors the tail of `introspect_postgres`.
#[must_use]
pub fn introspect_catalog(catalog: &PgCatalog, host: &str, dbname: &str) -> FileResult {
    let ddl = build_ddl(catalog);
    // Build the virtual URI with explicit posix `/` separators (never via
    // `Path::join`, which inserts the platform separator and yields the invalid
    // `postgresql:\host\db` on Windows). Mirrors graphify-py's #1672 switch to
    // `PurePosixPath`: the scheme's `//` collapses to a single `/`, so the path
    // is `postgresql:/host/db` on every platform.
    let virtual_path = PathBuf::from(format!("postgresql:/{host}/{dbname}"));
    extract_sql_with_content(&virtual_path, ddl.as_bytes())
}

/// Connect to `PostgreSQL`, query the catalog, and extract the reconstructed DDL.
/// Mirrors `introspect_postgres`. Requires the `postgres` feature.
///
/// # Errors
///
/// Returns [`PgIntrospectError::Connection`] (sanitized) if the connection
/// fails, or [`PgIntrospectError::Query`] if a catalog query fails.
#[cfg(feature = "postgres")]
pub fn introspect_postgres(dsn: &str) -> Result<FileResult, PgIntrospectError> {
    use postgres::Client;

    let connector = native_tls::TlsConnector::new()
        .map_err(|e| PgIntrospectError::Connection(sanitize_connection_error(&e.to_string())))?;
    let tls = postgres_native_tls::MakeTlsConnector::new(connector);

    // Empty DSN lets libpq fall back to PG* environment variables.
    let mut client = Client::connect(dsn, tls)
        .map_err(|e| PgIntrospectError::Connection(sanitize_connection_error(&e.to_string())))?;

    client
        .batch_execute("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE")
        .map_err(|e| PgIntrospectError::Query(e.to_string()))?;

    let catalog = pg_query_catalog(&mut client)?;

    let cfg: postgres::Config = dsn.parse().unwrap_or_else(|_| postgres::Config::new());
    let host = cfg
        .get_hosts()
        .iter()
        .find_map(|h| match h {
            postgres::config::Host::Tcp(t) => Some(t.clone()),
            postgres::config::Host::Unix(_) => None,
        })
        .unwrap_or_else(|| "localhost".to_string());
    let dbname = cfg.get_dbname().unwrap_or("db").to_string();

    Ok(introspect_catalog(&catalog, &host, &dbname))
}

/// Run the four catalog queries and assemble a [`PgCatalog`]. Requires `postgres`.
#[cfg(feature = "postgres")]
fn pg_query_catalog(client: &mut postgres::Client) -> Result<PgCatalog, PgIntrospectError> {
    let q = |c: &mut postgres::Client, sql: &str| {
        c.query(sql, &[])
            .map_err(|e| PgIntrospectError::Query(e.to_string()))
    };

    let tables = q(
        client,
        "SELECT table_schema, table_name, table_type \
         FROM information_schema.tables \
         WHERE table_schema NOT IN ('pg_catalog', 'information_schema') \
         ORDER BY table_schema, table_name",
    )?
    .iter()
    .map(|r| PgTable {
        schema: r.get(0),
        name: r.get(1),
        table_type: r.get(2),
    })
    .collect();

    let views = q(
        client,
        "SELECT table_schema, table_name, view_definition \
         FROM information_schema.views \
         WHERE table_schema NOT IN ('pg_catalog', 'information_schema') \
         ORDER BY table_schema, table_name",
    )?
    .iter()
    .map(|r| PgView {
        schema: r.get(0),
        name: r.get(1),
        definition: r.get(2),
    })
    .collect();

    let routines = q(
        client,
        "SELECT routine_schema, routine_name, routine_type, routine_definition, external_language \
         FROM information_schema.routines \
         WHERE routine_schema NOT IN ('pg_catalog', 'information_schema') \
         ORDER BY routine_schema, routine_name",
    )?
    .iter()
    .map(|r| PgRoutine {
        schema: r.get(0),
        name: r.get(1),
        routine_type: r.get(2),
        definition: r.get(3),
        external_language: r.get(4),
    })
    .collect();

    let foreign_keys = q(
        client,
        // #1746: read foreign keys from `pg_catalog.pg_constraint`, NOT
        // `information_schema.referential_constraints` — that view only exposes
        // constraints where the current user has WRITE access to the referencing
        // table, so a read-only introspection role saw zero FK rows (while
        // tables/views/routines still appeared) and the graph silently lost
        // every `references` edge. pg_constraint is world-readable and keyed by
        // constraint oid, which also avoids cross-matching same-named
        // constraints on sibling tables. Composite-FK column order is preserved
        // via UNNEST(conkey/confkey) WITH ORDINALITY.
        //
        // The `att.attname::text` casts mirror the old query's `::text`: attname
        // is the `name` type, and ARRAY_AGG over it yields a `name[]` the
        // postgres client cannot map onto `Vec<String>` (it deserializes only
        // text/varchar/name arrays reliably); the per-element cast makes the
        // result a plain `text[]`.
        "SELECT con.conname AS constraint_name, ns.nspname AS table_schema, \
         rel.relname AS table_name, \
         (SELECT ARRAY_AGG(att.attname::text ORDER BY k.ord) \
            FROM UNNEST(con.conkey) WITH ORDINALITY AS k(attnum, ord) \
            JOIN pg_catalog.pg_attribute att \
              ON att.attrelid = con.conrelid AND att.attnum = k.attnum) AS columns, \
         fns.nspname AS foreign_table_schema, frel.relname AS foreign_table_name, \
         (SELECT ARRAY_AGG(att.attname::text ORDER BY k.ord) \
            FROM UNNEST(con.confkey) WITH ORDINALITY AS k(attnum, ord) \
            JOIN pg_catalog.pg_attribute att \
              ON att.attrelid = con.confrelid AND att.attnum = k.attnum) AS foreign_columns \
         FROM pg_catalog.pg_constraint con \
         JOIN pg_catalog.pg_class rel ON rel.oid = con.conrelid \
         JOIN pg_catalog.pg_namespace ns ON ns.oid = rel.relnamespace \
         JOIN pg_catalog.pg_class frel ON frel.oid = con.confrelid \
         JOIN pg_catalog.pg_namespace fns ON fns.oid = frel.relnamespace \
         WHERE con.contype = 'f' \
           AND ns.nspname NOT IN ('pg_catalog', 'information_schema') \
         ORDER BY ns.nspname, rel.relname, con.conname",
    )?
    .iter()
    .map(|r| PgForeignKey {
        constraint_name: r.get(0),
        table_schema: r.get(1),
        table_name: r.get(2),
        columns: r.get(3),
        foreign_table_schema: r.get(4),
        foreign_table_name: r.get(5),
        foreign_columns: r.get(6),
    })
    .collect();

    Ok(PgCatalog {
        tables,
        views,
        routines,
        foreign_keys,
    })
}
