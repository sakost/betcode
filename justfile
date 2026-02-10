# Default recipe: run all checks
default: check

# Run all checks (format, lint, test, deny, machete, duplicates)
check: fmt-check lint test deny machete duplicates

# Format check (no modification)
fmt-check:
    cargo fmt --all -- --check

# Auto-format
fmt:
    cargo fmt --all

# Clippy with strict warnings
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all tests
test:
    cargo test --workspace

# Dependency audit
deny:
    cargo deny check

# Unused dependency check
machete:
    cargo machete

# DRY / duplicate code detection
duplicates:
    npx jscpd --config .jscpd.json

# Code metrics (non-blocking, generates report)
metrics:
    rust-code-analysis-cli -m -O json -o metrics/ -p crates/

# Full build
build:
    cargo build --workspace

# Clean
clean:
    cargo clean
    rm -rf jscpd-report/ metrics/

# Fix auto-fixable clippy lints
fix:
    cargo clippy --workspace --all-targets --fix --allow-dirty -- -D warnings
