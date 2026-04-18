# Task

Extend the `dev` crate's `DevPlugin` with `bevy-inspector-egui` powering several independently-toggleable debug panels: a world inspector, a chunk debugger (with force-load/evict controls), live RON editors for ability and world-object definitions with hot-reload, a network entity viewer exposing Lightyear replication and interest-management state, and a reflected-registry-driven spawn panel. Each feature must be individually togglable (Cargo feature, runtime flag, or both) so devs can enable only what they need without paying for the rest.
