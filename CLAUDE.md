## CRITICAL: Be Optimally Concise
Provide exactly the information required—no filler, no repetition, no expansions beyond what is asked.
Do not omit necessary details that affect accuracy or correctness.
Use the minimum words needed to be precise and unambiguous.
For code: provide the smallest correct implementation without extra abstractions, comments, or features not explicitly requested.

## Prioritize Dependency Sources
This project includes git submodules for all major dependencies under the `git/` directory (e.g. `bevy`, `lightyear`, `avian`). When working with or researching dependency functionality, **you MUST prioritize these local sources** over external documentation or web searches. 

### Mandatory Rules

1. **Always check local sources first**: Before consulting external documentation or performing web searches, search the relevant git submodule for:
   - Source code examples
   - API implementations
   - Internal documentation
   - Example projects
   - Tests that demonstrate usage patterns

2. **Prefer source code over documentation**: The source code is the single source of truth. When understanding how a feature works. External documentation may be consulted when:
   - The local sources have been thoroughly searched first
   - You need high-level conceptual understanding before diving into source
   - You're looking for community patterns or best practices not evident in examples

3. **Check version compatibility**: The submodules contain the latest source. Cross-reference with `Cargo.toml` to ensure you're using features available in the project's dependency versions.

## Act as a Driver-Orchestrator
CRITICAL: Maintain the global context, goals, and constraints of the human user. Delegate all technical, detailed, or multi-step tasks to sub-agents wherever possible and appropriate