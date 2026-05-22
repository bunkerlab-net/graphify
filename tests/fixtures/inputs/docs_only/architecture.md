# Architecture

## Broker Cluster

Each broker node maintains an in-memory ring buffer per topic partition. Nodes join
a cluster via a gossip protocol and elect a leader using Raft.

## Partitioning

Topics are partitioned using consistent hashing on the message key. The default
hash function is MurmurHash3 (128-bit variant). Rebalancing triggers when a node
joins or leaves and moves at most `ceil(P/N)` partitions per rebalance event.

## Storage

- **Hot tier**: ring buffer in shared memory (`mmap`). Evicted when full.
- **Warm tier**: write-ahead log (WAL) on NVMe SSD. Retained for `retention.hours`.
- **Cold tier**: object storage (S3-compatible). Optional; disabled by default.

## Replication

Each partition has a configurable replication factor (`replication.factor`, default 3).
Writes are acknowledged by the leader after `min.in.sync.replicas` followers confirm.

## Consumer Groups

Consumers belong to groups. The group coordinator assigns partitions to members
using the RangeAssignor strategy by default. Offsets are committed to the broker
and checkpointed every `auto.commit.interval.ms`.
