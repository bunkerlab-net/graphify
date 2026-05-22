# Graph Report - single_python_file  (2026-05-22)

## Corpus Check
- Corpus is ~368 words - fits in a single context window. You may not need a graph.

## Summary
- 24 nodes · 43 edges · 5 communities (2 shown, 3 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]

## God Nodes (most connected - your core abstractions)
1. `Shape` - 8 edges
2. `Rectangle` - 8 edges
3. `Circle` - 7 edges
4. `Triangle` - 7 edges
5. `bounding_box()` - 4 edges
6. `Point` - 3 edges
7. `largest_shape()` - 3 edges
8. `Geometry module with basic shape classes and area calculations.` - 1 edges
9. `Euclidean distance to another point.` - 1 edges
10. `Abstract base for all shapes.` - 1 edges

## Surprising Connections (you probably didn't know these)
- `Circle` --inherits--> `Shape`  [EXTRACTED]
  shapes.py → shapes.py  _Bridges community 1 → community 3_
- `Rectangle` --inherits--> `Shape`  [EXTRACTED]
  shapes.py → shapes.py  _Bridges community 1 → community 0_

## Communities (5 total, 3 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.32
Nodes (6): bounding_box(), Point, Geometry module with basic shape classes and area calculations., An axis-aligned rectangle., Approximate bounding box of a list of circles (simplified)., Rectangle

### Community 1 - "Community 1"
Cohesion: 0.47
Nodes (4): Abstract base for all shapes., A triangle defined by three vertices., Shape, Triangle

## Knowledge Gaps
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Rectangle` connect `Community 0` to `Community 1`, `Community 3`, `Community 4`?**
  _High betweenness centrality (0.234) - this node is a cross-community bridge._
- **Why does `Circle` connect `Community 3` to `Community 0`, `Community 1`, `Community 2`, `Community 4`?**
  _High betweenness centrality (0.176) - this node is a cross-community bridge._
- **Why does `Shape` connect `Community 1` to `Community 0`, `Community 3`, `Community 4`?**
  _High betweenness centrality (0.138) - this node is a cross-community bridge._
- **What connects `Geometry module with basic shape classes and area calculations.`, `Euclidean distance to another point.`, `Abstract base for all shapes.` to the rest of the system?**
  _8 weakly-connected nodes found - possible documentation gaps or missing edges._
