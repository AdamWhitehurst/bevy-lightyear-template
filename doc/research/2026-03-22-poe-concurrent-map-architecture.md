---
date: 2026-03-22T18:09:45-07:00
researcher: Claude
git_commit: 532dc2abc8babebd69a0431e876d4e8acae274d9
branch: bevy-lightyear-template-2
repository: bevy-lightyear-template-2
topic: "How Path of Exile handles running massive amounts of maps concurrently architecturally"
tags: [research, external, path-of-exile, server-architecture, instancing, networking]
status: complete
last_updated: 2026-03-22
last_updated_by: Claude
---

# Research: Path of Exile Concurrent Map Instance Architecture

**Date**: 2026-03-22T18:09:45-07:00
**Researcher**: Claude
**Git Commit**: 532dc2abc8babebd69a0431e876d4e8acae274d9
**Branch**: bevy-lightyear-template-2
**Repository**: bevy-lightyear-template-2

## Research Question

How does Path of Exile handle running massive amounts of maps concurrently from an architectural perspective?

## Summary

PoE runs each map instance as a **separate OS process** spawned via `fork()` from a prespawner that has all game data pre-loaded. Copy-on-write memory sharing keeps per-instance overhead at **5-20 MB**, allowing ~500 instances per commodity Xeon server. Backend services (account authority, instance manager, party manager, ladder authority) communicate through a custom **logical routing network** that addresses entities by ID rather than IP. Infrastructure runs on bare-metal servers (originally SoftLayer/IBM Cloud) with cloud burst capacity for league launches. PoE 2 moved to a microservices architecture splitting world, economy, and chat onto separate clusters.

## Detailed Findings

### Instance Process Model: fork()-Based Spawning

**Source**: [HN — GGG developer comment](https://news.ycombinator.com/item?id=20909591)

The core architectural insight. A GGG backend engineer described the system directly:

1. A **prespawner process** loads all game resources (assets, item definitions, monster data, skill trees) into memory.
2. When a new instance is needed, it calls `fork()`.
3. The child process generates random terrain in "a few hundred milliseconds."
4. `fork()` uses **copy-on-write** semantics — all shared game data remains shared in physical memory across all instance processes. Only per-instance state (generated terrain, monster positions, dropped items) allocates new pages.
5. Per-instance memory overhead: **5-20 MB**.
6. A single cheap single-processor Xeon server runs ~**500 instances** concurrently. In earlier, less CPU-intensive versions, this was as high as ~1,600.

This is a Linux-specific optimization. The massive read-only game dataset is loaded once and shared by the OS kernel across all forked processes for free.

### Instance Lifecycle

- **Creation**: Instance Manager selects a physical server. The prespawner on that server calls `fork()`. Child generates terrain.
- **Active**: Instance runs as its own OS process, handling game simulation for up to 6 players.
- **Idle timeout**: After all players leave, instance persists for **8-15 minutes** (campaign areas) or longer (map device instances) so players can return.
- **Teardown**: After timeout, process is killed. All in-instance state (dropped items, monster corpses) is lost. Character/item data was already persisted to the Account Authority.
- **No migration or failover**: If an instance server crashes, the instance is destroyed along with all items and monsters inside. Instances are ephemeral and disposable by design.

### Town vs Map Instances

- **Towns**: Shared instances holding many players. Persistent — no short expiry timer. Higher player cap.
- **Maps/zones**: Private to a party, capped at 6 players, temporary. Procedurally generated per-instance.
- Architecturally identical process model — same fork-based spawning, just different timeout and capacity parameters.

### Backend Service Architecture

**Source**: [HN — GGG developer comment](https://news.ycombinator.com/item?id=20909591)

Not microservices in the modern web sense, but "a few somewhat large services" broken up by sharding strategy:

| Service | Sharding | Notes |
|---------|----------|-------|
| **Account Authority** | 5 shards, 2 read-only replicas each | Account/character/item data. Handles vast majority of traffic |
| **Instance Manager** | — | Coordinates instance creation and server assignment |
| **Party Manager** | Separate sharding | Party state management |
| **Ladder Authority** | Separate sharding | League leaderboards |

### Logical Message Routing

**Source**: [HN — GGG developer comment](https://news.ycombinator.com/item?id=20909591)

GGG built a custom hierarchical message routing system:

- **Hierarchy**: One router per instance machine → multiple routers per data center → core routers connecting backend services.
- **Logical addressing**: Messages target entities (`Account:123`), not IP addresses. The router network tracks where each entity currently lives.
- **Multicast groups**: Chat channels, parties, guilds, and leagues are all multicast groups.
- Whispering someone sends a message to `Account:123` and the router network resolves the physical location.
- This **decouples service placement from addressing** — services and players can move between machines without changing their logical address.

### Technology Stack

| Component | Technology |
|-----------|-----------|
| Language | C++ (client and server) |
| Server OS | Linux |
| Database | PostgreSQL (from job postings) + embedded databases |
| Protocol | Custom binary. 2-byte packet type prefix. Variable-width stat encoding. Murmur2 content hashing |
| Transport | TCP (persistent state: inventory, currency) + UDP (latency-sensitive: combat, movement) |
| Content | `content.ggpk` — single ~11GB packed file, referenced by hash in network packets |
| Scripting | Lua for game mechanics (PoE 2) |

### Networking Modes

The server is always authoritative. Client chooses:

- **Deterministic Lockstep** (added v2.0.0): Client sends input, waits for server confirmation. Zero desync. Input lag = round-trip latency. For <100ms ping.
- **Predictive** (original): Client predicts outcomes immediately. Server validates and corrects asynchronously. Responsive but can desync (rubberbanding).
- **Auto/Hybrid**: Switches between modes based on latency (~150ms threshold).

Important: PoE's lockstep is **server-authoritative lockstep**, not peer-to-peer deterministic lockstep. Only the server runs the simulation — the client waits for authoritative state before rendering. No need for bitwise determinism.

### Infrastructure and Scaling

- **Hosting**: Originally SoftLayer/IBM Cloud bare-metal servers. Chris Wilson: "Extremely fast provisioning times and the ability to automate such provisioning allow us to treat SoftLayer's bare metal servers like virtual servers so that we can scale up rapidly."
- **Regions** (PoE 2, 16 gateways): Texas, Washington, California, Canada, Amsterdam, London, Frankfurt, Milan, Paris, Moscow, Singapore, Japan, Australia, Auckland, São Paulo, South Africa.
- **Single global realm**: Gateway selection determines which physical servers handle instances for latency; all players share one economy.
- **Master infrastructure**: Originally in Texas (Dallas).

### League Launch Scaling Strategy

1. **Baseline**: Bare-metal servers across 16 regions, automated provisioning.
2. **Pre-launch over-provisioning**: Scale up before league start.
3. **Cloud burst**: PoE 2 adds cloud capacity on top of bare metal for launch windows.
4. **Post-launch scale-down**: ~1 month after league start as player counts naturally decline.
5. **Instance efficiency**: fork()-based spawning keeps per-instance overhead minimal → high density per server.

### Scale Numbers

| Metric | Value |
|--------|-------|
| PoE 1 all-time Steam peak | 229k CCU (Settlers of Kalguur, July 2024); 350k cross-platform |
| PoE 2 EA Steam peak | 578k CCU (December 2024) |
| PoE 2 F2P spike | 353k Steam CCU (August 2025) |
| Instances per server | ~500 (2019, down from ~1,600 in early days) |
| Memory per instance | 5-20 MB |
| Instance player cap | 6 (maps), higher for towns |
| Instance idle timeout | 8-15 minutes (campaign), longer (maps) |

### PoE 2 Architectural Evolution

**Source**: [SM Mirror](https://smmirror.com/2025/02/how-the-development-of-cult-arpg-sequels-is-carried-out-using-path-of-exile-as-an-example-and-what-grinding-gear-games-changed-in-their-approach/)

- Moved to **microservices architecture**: world, economy, and chat split onto separate server clusters.
- Cloud server support for load balancing during peaks and DDoS attacks.
- Dual TCP/UDP protocol carried forward.
- Same lockstep/predictive/auto networking modes available.
- It is unclear whether PoE 2 still uses fork()-based instance spawning or has moved to containerized instances.

## Key Architectural Patterns

1. **fork() + copy-on-write** is the central insight. Load everything once, fork cheaply. Per-instance cost is only the mutable delta.
2. **Process-per-instance isolation**. A crash in one instance cannot take down others. Simple, battle-tested.
3. **Logical routing over physical routing**. Services address entities by ID; the routing mesh handles delivery. Physical topology is abstracted away.
4. **Ephemeral instances, persistent characters**. Instance state is disposable. Character/item state is persisted to the Account Authority continuously. No instance migration needed.
5. **Bare metal + cloud burst**. Steady-state on dedicated hardware for cost efficiency. Cloud capacity for launch spikes.

## Gaps and Open Questions

- **Database specifics**: Sharding strategy described but no details on schema, replication protocol, or persistence frequency.
- **Instance placement algorithm**: How the Instance Manager decides which server gets a new instance (least-loaded? geographic? deterministic?) is not documented.
- **PoE 2 instance model**: Whether fork()-based spawning survived the microservices rewrite is unknown.
- **Cross-region economy**: How a single economy works across globally distributed instance servers all hitting centralized databases is not detailed.
- **Failover for backend services**: How Account Authority or other services handle failover is not described.
- **Current hosting provider**: The SoftLayer information is from 2013. Current provider(s) unknown.

## Sources

- [HN: GGG Developer on PoE Backend Architecture](https://news.ycombinator.com/item?id=20909591) — **primary source**, a GGG engineer describing the system directly
- [HN: AAA Multiplayer Game Server Ops](https://news.ycombinator.com/item?id=20908168)
- [PoE Wiki: Networking Mode](https://www.poewiki.net/wiki/Networking_mode)
- [PoE Vault: Client-server Action Synchronisation (Chris Wilson)](https://www.poe-vault.com/dev-tracker/client-server-action-synchronisation)
- [Testudo Binarii: Reverse Engineering PoE Protocol](http://tbinarii.blogspot.com/2018/05/reverse-engineering-path-of-exile_0.html)
- [SM Mirror: PoE Development Approach](https://smmirror.com/2025/02/how-the-development-of-cult-arpg-sequels-is-carried-out-using-path-of-exile-as-an-example-and-what-grinding-gear-games-changed-in-their-approach/)
- [SoftLayer/IBM Cloud: Powers 100M Gamers](http://www.softlayer.com/press/softlayer-now-powers-online-games-more-100-million-gamers)
- [PCGamesN: Lockstep Mode](https://www.pcgamesn.com/path-of-exile/path-of-exile-s-new-lockstep-mode-banishes-desync-problems)
- [GDC Vault: Designing Path of Exile to Be Played Forever](https://www.gdcvault.com/play/1025784/Designing-Path-of-Exile-to)
- [Game World Observer: PoE 350k CCU](https://gameworldobserver.com/2024/07/29/path-of-exile-record-350k-concurrent-players-peak)
- [GamingBolt: PoE 2 578k CCU](https://gamingbolt.com/path-of-exile-2-surpasses-578000-concurrent-players-on-steam-becomes-15th-most-played-title-in-history)
- [PC Gamer: PoE 2 Queue Warning](https://www.pcgamer.com/games/rpg/path-of-exile-2-may-have-sold-too-well-so-beware-queues-we-really-didnt-expect-to-have-more-than-a-million-people-online-at-the-same-time/)
- [DigiStatement: PoE 2 Gateway List](https://digistatement.com/path-of-exile-2-poe-2-all-server-gateway-list-how-to-change/)
- [Steam Charts: Path of Exile](https://steamcharts.com/app/238960)
