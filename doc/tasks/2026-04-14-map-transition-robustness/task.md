# Task: Robust Map Transition System

Redesign the map transition handshake to be a stateful, multi-phase process that ensures all map (voxel terrain, meshes, world objects) and other entities (players, etc.) are fully loaded around the player before physics unfreezes and the loading screen dismisses. Fix broken client-side chunk remesh prioritization (propagator has no sources on client), prevent late-arriving room entities from leaking across transitions, and add transition state reporting to the loading screen for debugging. The system must tolerate high-latency scenarios (10s+ of seconds for map data loading).

Root causes documented in `doc/bug/2026-04-14-map-transition-failures.md`: trivial readiness criterion (H1), broken client remesh prioritization (H2), server regeneration latency (H3), entity-before-terrain race (H5), late-arriving room entity leak (H6).

Additional goals:
- Encapsulate the transition process into its own module/crate/Plugin rather than having it spread across client, server, and UI code.
- Unify the initial client connection + first map load with the transition process where possible, so both paths share the same readiness and loading logic.
- Update systems that transition depends on in a decoupled way. e.g. use markers/indicators/states (Components) that the transition process can check for.

**Future consideration (not in scope):** The transition process should be designed so that a full server switch (disconnect from one server, connect to another) could be inserted at the appropriate phase. Identify and annotate the seam point(s) in code where this would slot in.
