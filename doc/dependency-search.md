## Prioritize Dependency Sources
This project includes git submodules for all major dependencies under the `git/` directory (e.g. `bevy`, `lightyear`, `avian`). When working with or researching dependency functionality, **prioritize these local sources** over external documentation or web searches

## Always check local sources first
Before consulting external documentation or performing web searches, search the relevant git submodule for:
   - Source code examples
   - API implementations
   - Internal documentation
   - Example projects
   - Tests that demonstrate usage patterns

## Prefer source code over documentation
The source code is the single source of truth. When understanding how a feature works. External documentation may be consulted when:
   - The local sources have been thoroughly searched first
   - You need high-level conceptual understanding before diving into source
   - You're looking for community patterns or best practices not evident in examples

## Check version compatibility 
The submodules contain the latest source. Cross-reference with `Cargo.toml` to ensure you're using features available in the project's dependency versions
