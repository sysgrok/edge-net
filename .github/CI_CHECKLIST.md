# CI Checklist for Pull Requests

This document serves as a reminder for all CI steps that must be completed before submitting a PR to ensure it passes CI successfully.

## Required CI Steps

Based on the CI workflow in `.github/workflows/ci.yml`, the following steps must be completed for each PR:

### 1. Code Formatting
```bash
cargo fmt -- --check
```
If formatting issues are found, fix them with:
```bash
cargo fmt
```

### 2. Clippy Checks
Run clippy with various feature combinations to ensure no warnings:

```bash
# Standard features with examples
cargo clippy --features std --examples --no-deps -- -Dwarnings

# With defmt feature
cargo clippy --features std,defmt --no-deps -- -Dwarnings

# With log feature and examples
cargo clippy --features std,log --examples --no-deps -- -Dwarnings

# Embassy / smoltcp checks (Requires Rust 1.91)
cargo clippy --features std,edge-nal-embassy/all --examples --no-deps -- -Dwarnings
cargo clippy --features std,edge-nal-embassy/all,defmt --no-deps -- -Dwarnings
cargo clippy --features std,edge-nal-embassy/all,log --examples --no-deps -- -Dwarnings
```

### 3. Build Checks
Verify the project builds with different feature configurations:

```bash
# Default build with log
cargo build --features log

# No default features
cargo build --no-default-features

# Embassy with defmt (Requires Rust 1.91)
cargo build --no-default-features --features embassy,defmt

# Examples with log
cargo build --examples --features log

# Examples with defmt
export DEFMT_LOG=trace
cargo check --examples --features std,defmt
```

### 4. Tests
Run all tests to ensure functionality:
```bash
cargo test --all-features
```

## Toolchain Support

CI runs on:
- **Nightly** Rust
- **1.88** MSRV (Minimum Supported Rust Version)
- **1.91** for Embassy / smoltcp checks that require it

Ensure each check keeps MSRV compatibility with its specified Rust version.

## Pre-PR Checklist

Before submitting or updating a PR, verify:
- [ ] Code is formatted (`cargo fmt`)
- [ ] No clippy warnings with all feature combinations
- [ ] All builds succeed with different feature configurations
- [ ] All tests pass
- [ ] Changes are compatible with their specified MSRV
- [ ] Documentation is updated if needed
- [ ] Examples are updated or added if needed

## Quick Local Verification

To quickly verify your changes locally before pushing:

```bash
# Format check
cargo fmt -- --check

# Quick clippy check
cargo clippy --features std --examples --no-deps -- -Dwarnings

# Quick build check
cargo build --features log

# Embassy test check (Requires Rust 1.91)
cargo test -p edge-nal-embassy --all-features
```

If all these pass, your PR should pass CI successfully.
