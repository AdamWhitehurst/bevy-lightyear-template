# Research Questions

## Context

This project is a Bevy game that uses the `lightyear` networking crate; the lightyear source is vendored as a git submodule under `git/lightyear/`. The project's server networking code lives in `crates/server/src/network.rs` with an associated test at `crates/server/tests/multi_transport.rs`. Lightyear's transport layer is split across several sub-crates: `lightyear_udp`, `lightyear_webtransport`, `lightyear_steam`, `lightyear_crossbeam`, `lightyear_transport`, and `lightyear_netcode`.

## Questions

1. How does lightyear architect the concepts of "server", "link", and "transport" — what ECS entities/components represent each, and how does a single server entity relate to many links each potentially bound to a different transport type?

2. What do `lightyear_udp`, `lightyear_webtransport`, `lightyear_steam`, and `lightyear_crossbeam` each expose as their server-side API (plugins, components, bundles)? At what architectural layer does each sit — a transport entity attached to a link, a standalone server plugin, a replacement for netcode, or something else?

3. Do any of those transport crates register conflicting or singleton global state (resources, unique-per-app plugins, global async runtimes, I/O threads) that would prevent registering more than one of them in the same Bevy app?

4. How does `lightyear_netcode` fit relative to the transport crates — does it layer transparently on top of every transport, or is it specific to certain ones? Do any transports bypass netcode entirely, and if so what does that imply for a unified server-side connection abstraction?

5. In the project's `crates/server/src/network.rs`, what does `spawn_server_transports` do per-variant (UDP, WebTransport, Crossbeam), and how do the entities/components/resources spawned by each branch differ?

6. What does `crates/server/tests/multi_transport.rs` actually exercise — does it spin up more than one transport in a single server run, and what does it assert about the resulting state?

7. Where in the `git/lightyear/` submodule (examples, integration tests, docs) is there precedent for a server registering more than one transport simultaneously, and what pattern do those sites use to wire transports together?

8. What runtime-level prerequisites differ between UDP, WebTransport, and Steam (async runtime choice, TLS cert handling, Steam SDK init / AppID requirements, thread model, port/socket ownership), and do any of those differences impose practical constraints on hosting them in the same process?
