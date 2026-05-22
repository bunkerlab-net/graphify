# Graph Report - per_language_samples  (2026-05-22)

## Corpus Check
- Corpus is ~892 words - fits in a single context window. You may not need a graph.

## Summary
- 76 nodes · 88 edges · 11 communities (7 shown, 4 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 2 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 7|Community 7]]
- [[_COMMUNITY_Community 8|Community 8]]
- [[_COMMUNITY_Community 9|Community 9]]
- [[_COMMUNITY_Community 10|Community 10]]

## God Nodes (most connected - your core abstractions)
1. `HttpClient` - 7 edges
2. `ApiClient` - 5 edges
3. `UserService` - 4 edges
4. `DataProcessor` - 4 edges
5. `run_analysis()` - 4 edges
6. `Analyzer` - 4 edges
7. `Graph` - 4 edges
8. `createClient()` - 3 edges
9. `Server` - 3 edges
10. `main()` - 3 edges

## Surprising Connections (you probably didn't know these)
- `parse_response()` --calls--> `parse()`  [INFERRED]
  sample.rb → crate_a/src/lib.rs

## Communities (11 total, 4 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.18
Nodes (7): DEFAULT_ROLES, IUserRepository, USER_CONFIG, UserId, UserModule, UserService, UserStatus

### Community 1 - "Community 1"
Cohesion: 0.31
Nodes (3): Config, createClient(), HttpClient

### Community 2 - "Community 2"
Cohesion: 0.24
Nodes (3): ApiClient, parse_response(), parse()

### Community 3 - "Community 3"
Cohesion: 0.39
Nodes (5): Analyzer, compute_score(), normalize(), Fixture: functions and methods that call each other - for call-graph extraction, run_analysis()

### Community 7 - "Community 7"
Cohesion: 0.47
Nodes (3): main(), NewServer(), Server

### Community 8 - "Community 8"
Cohesion: 0.83
Nodes (3): App(), fmtCount(), fmtDate()

## Knowledge Gaps
- **6 isolated node(s):** `IUserRepository`, `UserStatus`, `UserId`, `DEFAULT_ROLES`, `USER_CONFIG` (+1 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **What connects `IUserRepository`, `UserStatus`, `UserId` to the rest of the system?**
  _7 weakly-connected nodes found - possible documentation gaps or missing edges._
