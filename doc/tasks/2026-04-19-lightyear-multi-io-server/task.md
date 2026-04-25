# Task

Determine whether lightyear supports a single server hosting multiple I/O transports (e.g. UDP, WebTransport, and Steam) simultaneously in the same process.

If supported: document how a server is configured to register and run multiple transports concurrently, including any per-transport prerequisites.

If not supported: document the architectural reasons and any constraints (shared global state, runtime conflicts, layering assumptions) that prevent it.
