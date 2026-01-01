---
date: 2025-12-31 14:55:53 PST
researcher: Adam Whitehurst
git_commit: 5f8c241617dda79210e709b0d8305fa8e9650098
branch: master
repository: bevy-lightyear-template
topic: "Implementing Test Coverage Reporting with cargo llvm-cov and VS Code Integration"
tags: [research, testing, coverage, cargo-llvm-cov, vscode, ci, documentation]
status: complete
last_updated: 2025-12-31
last_updated_by: Adam Whitehurst
---

# Research: Implementing Test Coverage Reporting with cargo llvm-cov and VS Code Integration

**Date**: 2025-12-31 14:55:53 PST
**Researcher**: Adam Whitehurst
**Git Commit**: 5f8c241617dda79210e709b0d8305fa8e9650098
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to implement test coverage reporting for this Bevy/Lightyear workspace using cargo llvm-cov, including:
- Coverage engine setup (cargo llvm-cov with lcov output)
- VS Code line-by-line highlighting (Coverage Gutters extension)
- CI integration (GitHub Actions)
- Developer documentation in README.md
- Validation steps to verify correct implementation

## Summary

This workspace has a well-organized test infrastructure across 6 crates with both native and WASM tests. cargo llvm-cov integrates cleanly with the existing workspace structure and provides accurate line coverage without requiring nightly Rust. The implementation requires minimal configuration: installing the tool, generating lcov.info, configuring VS Code Coverage Gutters, optionally adding GitHub Actions, and documenting the setup for developers.

## Current Testing Infrastructure

### Test Organization

The workspace has tests distributed across crates:

**Integration Tests** (`crates/*/tests/`):
- `crates/server/tests/` - 5 test files (integration.rs:802, multi_transport.rs, connection_flow.rs, observers.rs, plugin.rs)
- `crates/client/tests/` - 2 test files (connection.rs, plugin.rs)
- `crates/web/tests/` - 2 test files (plugin.rs, wasm_integration.rs:37)
- `crates/render/tests/` - 1 test file (render_plugin.rs)
- `crates/ui/tests/` - 1 test file (ui_plugin.rs)

**Test Utilities**:
- `crates/protocol/src/test_utils.rs` - Shared test utilities

**Test Infrastructure** (Cargo.toml:26-29):
- `approx = "0.5.1"` - Floating-point comparison
- `mock_instant = "0.6"` - Time mocking
- `test-log = { version = "0.2.17", features = ["trace", "color"] }` - Logging in tests

### Test Commands

From Makefile.toml and .cargo/config.toml:

**Native tests**:
```bash
cargo make test-native      # Runs protocol, client, server tests
cargo make test-protocol    # cargo test -p protocol --all-features
cargo make test-client      # cargo test -p client
cargo make test-server      # cargo test -p server
```

**WASM tests**:
```bash
cargo make test-wasm        # wasm-pack test --headless --firefox crates/web
```

**All tests**:
```bash
cargo make test-all         # Runs both native and WASM tests
cargo test-all              # Alias in .cargo/config.toml
```

### Test Profiles

From Cargo.toml:32-43:

**Native test profile**:
```toml
[profile.test]
opt-level = 1
debug = true
```

**WASM test profile**:
```toml
[profile.wasm-test]
inherits = "test"
opt-level = "s"              # Size optimization
codegen-units = 1            # Sequential compilation to reduce RAM
debug = false                # No debug info to save memory
strip = true                 # Strip symbols
lto = "thin"                 # Thin LTO for smaller binary
```

### Test Characteristics

**Native tests** use:
- Standard `#[test]` attribute
- Bevy's `App` with `MinimalPlugins` for unit tests
- Manual time control via `TimeUpdateStrategy::ManualInstant`
- Crossbeam transport for deterministic networking tests
- Real UDP sockets for integration tests

**WASM tests** (crates/web/tests/wasm_integration.rs:1-37) use:
- `wasm-bindgen-test` framework
- `#[cfg(target_arch = "wasm32")]` conditional compilation
- Browser-based test execution

### Current Coverage Setup

**No existing coverage configuration found**:
- No `.lcov`, `lcov.info`, or coverage output files
- No coverage-related entries in .gitignore
- No VS Code settings for coverage
- No CI coverage jobs

## cargo llvm-cov Integration Research

### Tool Capabilities

From [GitHub - taiki-e/cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov):

**cargo-llvm-cov** is a cargo subcommand that provides:
- LLVM source-based code coverage (`-C instrument-coverage`)
- Line, region, and branch coverage
- No nightly Rust required (stable only)
- Workspace support with selective package inclusion/exclusion
- Multiple output formats: HTML, LCOV, JSON, text

### Installation

From [cargo-llvm-cov installation](https://github.com/taiki-e/cargo-llvm-cov):

```bash
cargo install cargo-llvm-cov
```

**No manual llvm-tools-preview installation required**: cargo-llvm-cov automatically installs llvm-tools-preview when first run (prompts on local machine, auto-installs in CI).

**Alternative installation methods**:
- Homebrew: `brew install cargo-llvm-cov`
- GitHub Actions: `uses: taiki-e/install-action@cargo-llvm-cov`

### Workspace Coverage Commands

From [cargo-llvm-cov README](https://github.com/taiki-e/cargo-llvm-cov/blob/main/README.md):

**Basic workspace coverage**:
```bash
cargo llvm-cov --workspace
```

**Generate LCOV for VS Code**:
```bash
cargo llvm-cov --workspace --lcov --output-path lcov.info
```

**Clean before generating** (recommended for accurate results):
```bash
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --lcov --output-path lcov.info
```

**HTML report** (for visual inspection):
```bash
cargo llvm-cov --workspace --html
cargo llvm-cov --workspace --open  # Opens HTML in browser
```

### Excluding Crates

From [cargo-llvm-cov workspace support](https://lib.rs/crates/cargo-llvm-cov):

**Exclusion flags**:
- `--exclude <package>` - Excludes from both test and report
- `--exclude-from-test <package>` - Excludes from test but includes in report
- `--exclude-from-report <package>` - Includes in test but excludes from report

**Example** (if WASM crate needs exclusion):
```bash
cargo llvm-cov --workspace --exclude web --lcov --output-path lcov.info
```

**File-level exclusion**:
- `--ignore-filename-regex <pattern>` - Exclude files matching pattern
- By default: `tests/` directories excluded from report
- Vendored sources excluded automatically

### WASM Considerations

**WASM coverage not directly supported**: The web search results don't indicate native WASM target support for cargo llvm-cov. WASM tests use wasm-pack which has different coverage tooling.

**Recommended approach**:
1. Run coverage on native tests only (protocol, client, server, render, ui)
2. Exclude `web` crate from coverage: `--exclude web`
3. WASM-specific code coverage would require separate tooling (not covered in this research)

## VS Code Coverage Gutters Integration

### Extension Installation

From [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=ryanluker.vscode-coverage-gutters):

Install "Coverage Gutters" extension by ryanluker.

### VS Code Configuration

From [Visualizing Rust Code Coverage in VS Code](https://nattrio.medium.com/visualizing-rust-code-coverage-in-vs-code-781aaf334f11):

Add to `.vscode/settings.json`:
```json
{
  "coverage-gutters.coverageFileNames": [
    "lcov.info"
  ],
  "coverage-gutters.showLineCoverage": true
}
```

### Usage Workflow

1. Generate coverage: `cargo llvm-cov --workspace --exclude web --lcov --output-path lcov.info`
2. In VS Code: `Ctrl+Shift+P` → "Coverage Gutters: Display Coverage"
3. Coverage indicators appear in editor gutter:
   - Green: Covered lines
   - Red: Uncovered lines
   - Yellow: Partially covered lines

### Auto-reload with cargo-watch

For continuous coverage updates:
```bash
cargo install cargo-watch
cargo watch -x "llvm-cov --workspace --exclude web --lcov --output-path lcov.info"
```

Coverage Gutters can auto-reload when lcov.info changes.

## GitHub Actions CI Integration

### Basic Coverage Workflow

From [GitHub Actions integration example](https://github.com/taiki-e/cargo-llvm-cov):

Create `.github/workflows/coverage.yml`:
```yaml
name: Coverage
on: [pull_request, push]

jobs:
  coverage:
    runs-on: ubuntu-latest
    env:
      CARGO_TERM_COLOR: always
    steps:
      - uses: actions/checkout@v5

      - name: Install Rust
        run: rustup update stable

      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@cargo-llvm-cov

      - name: Generate code coverage
        run: cargo llvm-cov --workspace --exclude web --lcov --output-path lcov.info

      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v5
        with:
          token: ${{ secrets.CODECOV_TOKEN }}
          files: lcov.info
          fail_ci_if_error: true
```

### Integration with Existing Tests

Since this project uses `cargo make test-all`:

**Option 1**: Separate coverage job (recommended)
- Keep existing test jobs
- Add dedicated coverage job using cargo llvm-cov
- Upload to coverage service (Codecov, Coveralls)

**Option 2**: Modify test-native task
- Add coverage generation to Makefile.toml
- Generate lcov.info as part of test run

## Validation Steps

### Local Validation

**1. Installation verification**:
```bash
cargo llvm-cov --version
# Should output: cargo-llvm-cov 0.6.x
```

**2. Clean build test**:
```bash
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --exclude web
# Should run tests and show coverage summary
```

**3. LCOV generation**:
```bash
cargo llvm-cov --workspace --exclude web --lcov --output-path lcov.info
ls -lh lcov.info
# Should show lcov.info file exists with non-zero size
```

**4. Coverage data sanity check**:
```bash
head -20 lcov.info
# Should show lcov format:
# TN:
# SF:/path/to/file.rs
# DA:line,hit_count
```

**5. HTML report inspection**:
```bash
cargo llvm-cov --workspace --exclude web --open
# Should open browser with coverage report
# Verify familiar source files are shown
# Check coverage percentages are reasonable
```

**6. VS Code integration**:
- Open project in VS Code
- Open a test file (e.g., `crates/server/tests/integration.rs`)
- Run: `Ctrl+Shift+P` → "Coverage Gutters: Display Coverage"
- Verify green/red indicators appear in gutter
- Check bottom status bar shows coverage percentage

### Validation Checklist

**Coverage completeness**:
- [ ] All workspace crates covered (except web)
- [ ] Integration tests show coverage
- [ ] Unit tests (if any exist inline) show coverage
- [ ] `test_utils.rs` modules are covered

**Accuracy verification**:
- [ ] Test functions themselves not counted as uncovered
- [ ] Known covered code shows green
- [ ] Known uncovered code shows red
- [ ] Conditional branches show partial coverage

**Tooling validation**:
- [ ] VS Code gutters update when regenerating lcov.info
- [ ] HTML report matches VS Code coverage
- [ ] CI job (if added) generates coverage without errors

## Documentation Requirements for README.md

### Recommended Documentation Structure

Add new section to README.md after "Development" section:

```markdown
## Testing

### Running Tests

**All native tests**:
```bash
cargo make test-native
```

**WASM tests**:
```bash
cargo make test-wasm
```

**All tests** (native + WASM):
```bash
cargo make test-all
```

### Test Coverage

This project uses [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) for code coverage analysis.

#### Setup

1. **Install cargo-llvm-cov**:
   ```bash
   cargo install cargo-llvm-cov
   ```

2. **Install VS Code Coverage Gutters** (optional, for visual coverage):
   - Install the [Coverage Gutters](https://marketplace.visualstudio.com/items?itemName=ryanluker.vscode-coverage-gutters) extension
   - Configuration is already set up in `.vscode/settings.json`

#### Generating Coverage

**Terminal report**:
```bash
cargo llvm-cov --workspace --exclude web
```

**LCOV for VS Code**:
```bash
cargo llvm-cov --workspace --exclude web --lcov --output-path lcov.info
```

**HTML report**:
```bash
cargo llvm-cov --workspace --exclude web --html
cargo llvm-cov --workspace --exclude web --open  # Opens in browser
```

#### Viewing Coverage in VS Code

1. Generate `lcov.info`: `cargo llvm-cov --workspace --exclude web --lcov --output-path lcov.info`
2. Open VS Code
3. Press `Ctrl+Shift+P` (or `Cmd+Shift+P` on macOS)
4. Select "Coverage Gutters: Display Coverage"
5. Coverage indicators will appear in the editor gutter:
   - 🟢 Green: Covered lines
   - 🔴 Red: Uncovered lines
   - 🟡 Yellow: Partially covered lines

#### Notes

- **WASM tests excluded**: Coverage only tracks native tests (protocol, client, server, render, ui)
- **Clean builds**: Run `cargo llvm-cov clean --workspace` before generating coverage if you encounter stale data
- **CI**: Coverage is automatically generated and uploaded in GitHub Actions (see `.github/workflows/coverage.yml`)

### Troubleshooting

**Issue**: VS Code doesn't show coverage gutters
- Solution: Ensure `lcov.info` exists in project root, click "Watch" in VS Code status bar

**Issue**: Coverage shows 0% for all files
- Solution: Run `cargo llvm-cov clean --workspace` then regenerate coverage

**Issue**: Old coverage data persists
- Solution: Delete `target/llvm-cov-target` directory and regenerate
```

### Documentation Validation

**Clarity checks**:
- [ ] Step-by-step instructions are clear for new developers
- [ ] All commands are copy-pasteable
- [ ] Examples show expected output
- [ ] Common issues have solutions

**Completeness checks**:
- [ ] Installation instructions present
- [ ] Multiple usage patterns documented (terminal, VS Code, HTML)
- [ ] Troubleshooting section covers known issues
- [ ] CI integration mentioned (if implemented)

## Implementation Recommendations

### Phase 1: Local Setup

1. Add to `.vscode/settings.json`:
```json
{
  "coverage-gutters.coverageFileNames": [
    "lcov.info"
  ],
  "coverage-gutters.showLineCoverage": true
}
```

2. Add to `.gitignore`:
```
# Coverage outputs
lcov.info
coverage/
target/llvm-cov-target/
```

3. Update README.md with Testing section (see above)

### Phase 2: Makefile Integration

Add to `Makefile.toml`:
```toml
[tasks.coverage]
description = "Generate test coverage report (native tests only)"
workspace = false
script = '''
#!/bin/bash
echo "Generating test coverage (excluding WASM)..."
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --exclude web --lcov --output-path lcov.info
echo "Coverage report generated: lcov.info"
echo "Open in VS Code: Ctrl+Shift+P -> 'Coverage Gutters: Display Coverage'"
'''

[tasks.coverage-open]
description = "Generate and open HTML coverage report"
workspace = false
script = '''
#!/bin/bash
cargo llvm-cov --workspace --exclude web --open
'''
```

Then developers can run:
```bash
cargo make coverage       # Generate lcov.info
cargo make coverage-open  # Open HTML report
```

### Phase 3: CI Integration (Optional)

Create `.github/workflows/coverage.yml` (see GitHub Actions section above).

**Consider**:
- Upload to Codecov/Coveralls for tracking over time
- Add coverage badge to README.md
- Set minimum coverage threshold (e.g., fail if < 70%)

## Code References

**Test infrastructure**:
- `Cargo.toml:26-43` - Test dependencies and profiles
- `Makefile.toml:93-136` - Test task definitions
- `.cargo/config.toml:11` - `test-all` alias

**Example tests**:
- `crates/server/tests/integration.rs:1-802` - Comprehensive integration tests
- `crates/web/tests/wasm_integration.rs:1-37` - WASM test examples

**Current configuration**:
- `.vscode/settings.json:1-14` - VS Code rust-analyzer config (coverage config needed)
- `README.md:70-98` - Development section (testing section needed)

## Open Questions

1. **WASM coverage**: Should WASM-specific coverage be tracked separately with different tooling?
   - Current recommendation: Exclude WASM crate, focus on native coverage

2. **Coverage targets**: What minimum coverage percentage should be enforced in CI?
   - Recommendation: Start without minimums, establish baseline, then set targets

3. **Coverage scope**: Should coverage include benchmarks or only tests?
   - Current scope: Tests only (standard practice)

4. **Integration test coverage**: Are integration tests providing sufficient coverage or are unit tests needed?
   - Observation: Most tests are integration tests; line coverage will show gaps

## Sources

- [GitHub - taiki-e/cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)
- [cargo-llvm-cov README](https://github.com/taiki-e/cargo-llvm-cov/blob/main/README.md)
- [Cargo-llvm-cov on Lib.rs](https://lib.rs/crates/cargo-llvm-cov)
- [cargo-llvm-cov on crates.io](https://crates.io/crates/cargo-llvm-cov)
- [Visualizing Rust Code Coverage in VS Code](https://nattrio.medium.com/visualizing-rust-code-coverage-in-vs-code-781aaf334f11)
- [Coverage Gutters - VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=ryanluker.vscode-coverage-gutters)
- [GitHub - taiki-e/install-action](https://github.com/taiki-e/install-action)
- [Test coverage - cargo-nextest](https://nexte.st/docs/integrations/test-coverage/)
- [LLVM Command Guide - llvm-cov](https://llvm.org/docs/CommandGuide/llvm-cov.html)
