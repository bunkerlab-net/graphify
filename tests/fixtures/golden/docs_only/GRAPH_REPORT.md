# Graph Report - docs_only  (2026-05-22)

## Corpus Check
- Corpus is ~324 words - fits in a single context window. You may not need a graph.

## Summary
- 23 nodes · 21 edges · 5 communities (4 shown, 1 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]

## God Nodes (most connected - your core abstractions)
1. `API Reference` - 4 edges
2. `Topics` - 4 edges
3. `Consumers` - 4 edges
4. `Project Overview` - 3 edges
5. ``POST /topics`` - 2 edges
6. `code:json ({)` - 2 edges
7. `Producers` - 2 edges
8. ``POST /topics/{name}/messages`` - 2 edges
9. `Broker Cluster` - 1 edges
10. `Partitioning` - 1 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Communities (5 total, 1 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.33
Nodes (5): Broker Cluster, Consumer Groups, Partitioning, Replication, Storage

### Community 1 - "Community 1"
Cohesion: 0.33
Nodes (6): code:json ({), `DELETE /topics/{name}`, `GET /topics/{name}`, `POST /topics`, `POST /topics/{name}/messages`, Topics

### Community 2 - "Community 2"
Cohesion: 0.50
Nodes (4): Consumers, `GET /consumers/{id}/messages`, `POST /consumers`, `POST /consumers/{id}/offsets`

### Community 3 - "Community 3"
Cohesion: 0.50
Nodes (3): Components, Goals, Project Overview

## Knowledge Gaps
- **12 isolated node(s):** `Broker Cluster`, `Partitioning`, `Storage`, `Replication`, `Consumer Groups` (+7 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **1 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `API Reference` connect `Community 4` to `Community 1`, `Community 2`?**
  _High betweenness centrality (0.190) - this node is a cross-community bridge._
- **Why does `Topics` connect `Community 1` to `Community 4`?**
  _High betweenness centrality (0.132) - this node is a cross-community bridge._
- **Why does `Consumers` connect `Community 2` to `Community 4`?**
  _High betweenness centrality (0.130) - this node is a cross-community bridge._
- **What connects `Broker Cluster`, `Partitioning`, `Storage` to the rest of the system?**
  _12 weakly-connected nodes found - possible documentation gaps or missing edges._
