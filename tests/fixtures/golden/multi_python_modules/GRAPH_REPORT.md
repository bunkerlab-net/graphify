# Graph Report - multi_python_modules  (2026-05-22)

## Corpus Check
- Corpus is ~928 words - fits in a single context window. You may not need a graph.

## Summary
- 51 nodes · 86 edges · 7 communities (4 shown, 3 thin omitted)
- Extraction: 65% EXTRACTED · 35% INFERRED · 0% AMBIGUOUS · INFERRED: 30 edges (avg confidence: 0.57)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]

## God Nodes (most connected - your core abstractions)
1. `TaskService` - 20 edges
2. `TaskStore` - 15 edges
3. `UserStore` - 14 edges
4. `ProjectStore` - 13 edges
5. `Task` - 10 edges
6. `Project` - 10 edges
7. `User` - 8 edges
8. `Priority` - 6 edges
9. `Status` - 6 edges
10. `run_demo()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `run_demo()` --calls--> `TaskService`  [INFERRED]
  cli.py → service.py
- `TaskService` --uses--> `User`  [INFERRED]
  service.py → models.py
- `TaskService` --uses--> `Task`  [INFERRED]
  service.py → models.py
- `TaskService` --uses--> `Project`  [INFERRED]
  service.py → models.py
- `TaskService` --uses--> `Priority`  [INFERRED]
  service.py → models.py

## Communities (7 total, 3 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.17
Nodes (3): Business logic layer for task management., Orchestrates task operations and enforces business rules., TaskService

### Community 1 - "Community 1"
Cohesion: 0.25
Nodes (5): ProjectStore, In-memory storage layer for task management., Simple in-memory project repository., Simple in-memory user repository., UserStore

### Community 2 - "Community 2"
Cohesion: 0.25
Nodes (4): Represents a system user., User, Simple in-memory task repository., TaskStore

### Community 3 - "Community 3"
Cohesion: 0.60
Nodes (4): Enum, Priority, Data models for the task management system., Status

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `TaskService` connect `Community 0` to `Community 1`, `Community 2`, `Community 3`, `Community 4`, `Community 5`, `Community 6`?**
  _High betweenness centrality (0.527) - this node is a cross-community bridge._
- **Why does `TaskStore` connect `Community 2` to `Community 0`, `Community 1`, `Community 3`, `Community 4`, `Community 5`?**
  _High betweenness centrality (0.214) - this node is a cross-community bridge._
- **Why does `Task` connect `Community 5` to `Community 0`, `Community 1`, `Community 2`, `Community 3`?**
  _High betweenness centrality (0.171) - this node is a cross-community bridge._
- **Are the 9 inferred relationships involving `TaskService` (e.g. with `User` and `Task`) actually correct?**
  _`TaskService` has 9 INFERRED edges - model-reasoned connections that need verification._
- **Are the 7 inferred relationships involving `TaskStore` (e.g. with `TaskService` and `User`) actually correct?**
  _`TaskStore` has 7 INFERRED edges - model-reasoned connections that need verification._
- **Are the 7 inferred relationships involving `UserStore` (e.g. with `TaskService` and `User`) actually correct?**
  _`UserStore` has 7 INFERRED edges - model-reasoned connections that need verification._
- **Are the 7 inferred relationships involving `ProjectStore` (e.g. with `TaskService` and `User`) actually correct?**
  _`ProjectStore` has 7 INFERRED edges - model-reasoned connections that need verification._
