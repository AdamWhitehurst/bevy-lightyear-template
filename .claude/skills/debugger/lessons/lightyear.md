# Lightyear Debugging Lessons

## Single Entity Architecture (lightyear 0.25+)

As of lightyear 0.25, Predicted/Confirmed/Interpolated are merged into a single entity. There is only ONE entity per replicated character on the client. The `Predicted` component is a marker on that same entity.

This project uses lightyear 0.26.4 (from git submodule). The old two-entity architecture does not apply.

## Tracy Instrumentation for Lightyear

- `tracy-client` without the `enable` feature compiles `plot!()` to no-ops — no `#[cfg]` guards needed
- Use `tracy_client::Client::running()` + `.message()` for discrete events (not `plot!()`)
- `plot!()` sets a named value — the last call per frame wins. In FixedUpdate (which runs multiple times per frame during rollback), the last replayed tick's value is what shows in Tracy
