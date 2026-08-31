set shell := ["bash", "-uc"]

# List available recipes.
default:
  @just --list

# CI-safe desktop verification.
verify: desktop-fmt desktop-clippy desktop-test git-smoke

# CI-safe checks without the smoke runner.
desktop-check: desktop-fmt desktop-clippy desktop-test

# Check Rust formatting for the desktop workspace.
desktop-fmt:
  cd desktop && cargo fmt -- --check

# Run clippy like CI.
desktop-clippy:
  cd desktop && cargo clippy --all-targets -- -D warnings

# Run desktop tests, preferring nextest when installed.
desktop-test:
  cd desktop && if cargo nextest --version >/dev/null 2>&1; then cargo nextest run; else cargo test; fi

# Build all reviu_driver binaries.
driver-bins:
  cd desktop && cargo build -p reviu_driver --bins

# Run the CI-compatible local Git smoke suite.
git-smoke: driver-bins
  cd desktop && target/debug/reviu-git-smoke --driver-bin target/debug/reviu-driver

# Run one local Git smoke scenario.
git-smoke-scenario scenario: driver-bins
  cd desktop && target/debug/reviu-git-smoke --driver-bin target/debug/reviu-driver --scenario "{{scenario}}" --fail-fast

# List local Git smoke scenarios.
git-smoke-list: driver-bins
  cd desktop && target/debug/reviu-git-smoke --list

# Run the local visual smoke check.
visual-smoke: driver-bins
  cd desktop && target/debug/reviu-visual-smoke --driver-bin target/debug/reviu-driver

# Run the live GitHub smoke check with primary Reviu auth and bot actor.
github-smoke repo="../reviu-github-smoke": driver-bins
  cd desktop && \
    if [ -z "${REVIU_AUTH_TOKEN:-}" ]; then \
      if command -v security >/dev/null 2>&1; then \
        export REVIU_AUTH_TOKEN="$(security find-internet-password -s reviu_auth.dev -a bearer -w)"; \
      else \
        echo "REVIU_AUTH_TOKEN is required outside macOS Keychain" >&2; \
        exit 1; \
      fi; \
    fi; \
    REVIU_PROFILE="${REVIU_PROFILE:-dev}" \
    API_BASE_URL="${API_BASE_URL:-http://localhost:3001}" \
    REVIU_GITHUB_SMOKE=1 \
    target/debug/reviu-github-smoke \
      --backend test \
      --driver-bin target/debug/reviu-driver \
      --repo "{{repo}}" \
      --require-bot

# Run all local checks, including live GitHub smoke.
verify-live: verify github-smoke
