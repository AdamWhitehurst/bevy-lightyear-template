# Persistence Abstraction

Abstract all saving/loading logic into a `persistence` module with event-driven systems that listen to save, load, delete, and list events and dispatch to pluggable backends (filesystem, nostr). The abstraction must support multiple backends and be able to report "not found" so the server can (re)generate missing data.
