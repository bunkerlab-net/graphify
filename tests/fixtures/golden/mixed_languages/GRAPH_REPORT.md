# Graph Report - mixed_languages  (2026-05-22)

## Corpus Check
- Corpus is ~631 words - fits in a single context window. You may not need a graph.

## Summary
- 34 nodes · 36 edges · 6 communities (4 shown, 2 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 5|Community 5]]

## God Nodes (most connected - your core abstractions)
1. `RequestHandler` - 8 edges
2. `ApiClient` - 5 edges
3. `Proxy` - 4 edges
4. `Mixed Language Demo` - 3 edges
5. `Architecture` - 2 edges
6. `Running` - 2 edges
7. `Simple HTTP server in Python.` - 1 edges
8. `Handle incoming HTTP requests.` - 1 edges
9. `Config` - 1 edges
10. `User` - 1 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Communities (6 total, 2 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.22
Nodes (3): ApiClient, ApiResponse, User

### Community 2 - "Community 2"
Cohesion: 0.33
Nodes (5): Architecture, code:block1 (Browser → TypeScript client), code:bash (# Start the Python server), Mixed Language Demo, Running

### Community 3 - "Community 3"
Cohesion: 0.50
Nodes (3): BaseHTTPRequestHandler, Handle incoming HTTP requests., RequestHandler

## Knowledge Gaps
- **5 isolated node(s):** `Config`, `User`, `ApiResponse`, `code:block1 (Browser → TypeScript client)`, `code:bash (# Start the Python server)`
  These have ≤1 connection - possible missing edges or undocumented components.
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `RequestHandler` connect `Community 3` to `Community 4`, `Community 5`?**
  _High betweenness centrality (0.069) - this node is a cross-community bridge._
- **What connects `Simple HTTP server in Python.`, `Handle incoming HTTP requests.`, `Config` to the rest of the system?**
  _7 weakly-connected nodes found - possible documentation gaps or missing edges._
