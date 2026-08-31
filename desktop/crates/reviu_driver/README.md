# Reviu driver

`reviu_driver` mounts the real `WorkspaceView` in a GPUI test window and accepts JSON-lines commands on stdin. It is useful for agent-driven UI checks, smoke tests, and local perf probes without launching the packaged app.

## Build

From `desktop/`:

```sh
cargo build -p reviu_driver --bins
```

This builds:

- `target/debug/reviu-driver`
- `target/debug/reviu-driver-mcp`
- `target/debug/reviu-git-smoke`
- `target/debug/reviu-github-smoke`
- `target/debug/reviu-perf`
- `target/debug/reviu-visual-smoke`

## `reviu-driver`

Start the JSON-lines driver:

```sh
cargo run -p reviu_driver --bin reviu-driver -- --backend test
```

Or use the visual backend on macOS:

```sh
cargo run -p reviu_driver --bin reviu-driver -- --backend visual
```

Useful options:

- `--backend test`: GPUI test backend, default, no screenshots.
- `--backend visual`: macOS offscreen Metal renderer, supports screenshots.
- `--agent-command <path>`: use a fake or custom agent command.

Build the stub ACP agent when agent sessions are needed:

```sh
cargo build -p agent_acp --features test-support --bin stub_agent
cargo run -p reviu_driver --bin reviu-driver -- --agent-command target/debug/stub_agent
```

Common commands:

```json
{"cmd":"path_prompt","path":"/tmp/repo"}
{"cmd":"git_state"}
{"cmd":"run_git_action","action":{"action":"push"}}
{"cmd":"open_pull_request_file","path":"fixtures/pr-open.txt"}
{"cmd":"create_pull_request_review_comment","path":"fixtures/pr-open.txt","line":0,"body":"note"}
{"cmd":"show_review"}
{"cmd":"discard_pull_request_review"}
{"cmd":"dialog_state"}
{"cmd":"confirm_dialog"}
{"cmd":"notification_log"}
{"cmd":"quit"}
```

The test backend also supports selector-driven commands such as `bounds`, `click`, `type`, `key`, `clock`, `wait`, and `park`. The visual backend supports point clicks and `screenshot`.

The driver reads and writes real repositories. Use temporary repositories for repeatable tests.

## `reviu-driver-mcp`

`reviu-driver-mcp` is a thin MCP stdio wrapper around `reviu-driver`. It keeps one driver process alive, starts it lazily on the first tool call, restarts it if it dies before a command, and maps MCP tools to the existing JSON-lines commands.

Run it from `desktop/`:

```sh
cargo build -p reviu_driver --bins
target/debug/reviu-driver-mcp --driver-bin target/debug/reviu-driver --backend test
```

Example MCP config:

```json
{
  "mcpServers": {
    "reviu-driver": {
      "command": "/path/to/reviu/desktop/target/debug/reviu-driver-mcp",
      "args": [
        "--driver-bin",
        "/path/to/reviu/desktop/target/debug/reviu-driver",
        "--backend",
        "test"
      ]
    }
  }
}
```

Useful options:

- `--backend test|visual`: default backend for the wrapped driver.
- `--driver-bin <path>`: path to a prebuilt `reviu-driver`; if omitted, the wrapper looks for a sibling binary and then falls back to `cargo run`.
- `--agent-command <path>`: forwarded to `reviu-driver`.

Core tools:

- lifecycle: `start`, `restart`, `status`, `quit`
- UI input: `bounds`, `click`, `type`, `key`, `clock`, `wait`, `park`, `scroll`
- app state: `path_prompt`, `open_file`, `open_pull_request_file`, `show_changes`, `show_pull_request`, `show_review`, `hide_dock`, `agent_stats`, `editor_stats`, `auth_state`
- Git/debug: `git_state`, `dialog_state`, `confirm_dialog`, `cancel_dialog`, `notification_stats`, `notification_log`, `run_git_action`, `create_pull_request_review_comment`, `discard_pull_request_review`
- visual: `screenshot` with `--backend visual` on macOS

Like the raw driver, the MCP wrapper talks to real repositories. Point it at temporary repos unless you deliberately want to inspect a live checkout.

## `reviu-git-smoke`

`reviu-git-smoke` runs CI-compatible Git smoke scenarios through `reviu-driver`. Each scenario creates its own temporary repositories and isolates driver config with a temporary `HOME`, `XDG_CONFIG_HOME`, and `REVIU_PROFILE=dev`.

Run the full suite:

```sh
cargo build -p reviu_driver --bins
target/debug/reviu-git-smoke --driver-bin target/debug/reviu-driver
```

List scenarios:

```sh
target/debug/reviu-git-smoke --list
```

Run one scenario:

```sh
target/debug/reviu-git-smoke \
  --driver-bin target/debug/reviu-driver \
  --scenario merge_conflict_abort
```

Useful options:

- `--scenario <name>`: run one scenario.
- `--scenario a,b,c`: run a comma-separated subset.
- `--fail-fast`: stop after the first failed scenario.
- `--keep-temp`: keep the run directory after success or failure.
- `--backend test|visual`: select the driver backend.
- `--driver-bin <path>`: use a prebuilt driver binary instead of `cargo run`.

On failure, the runner keeps the temp directory and prints:

- a rerun command for the failed scenario
- `git status --short --branch`
- recent decorated `git log`
- `git stash list`
- remote heads when a bare remote exists
- the tail of `driver.stderr.log`

CI runs the smoke suite on Unix runners. Windows is skipped because the existing desktop tests assume Unix-style paths.

## `reviu-visual-smoke`

`reviu-visual-smoke` is a macOS-only local smoke check for the visual backend. It creates a temporary repo, opens it through `reviu-driver --backend visual`, triggers the force-push confirmation dialog, captures a screenshot, confirms the dialog, and verifies the force push landed.

Run it from `desktop/`:

```sh
cargo build -p reviu_driver --bins
target/debug/reviu-visual-smoke --driver-bin target/debug/reviu-driver
```

Useful options:

- `--driver-bin <path>`: use a prebuilt driver binary instead of `cargo run`.
- `--screenshot <path>`: write the dialog screenshot to a specific path.
- `--keep-temp`: keep the temporary repository and driver logs.

This is intentionally not wired into CI yet.

## `reviu-github-smoke`

`reviu-github-smoke` is an opt-in live GitHub smoke check. It is not wired into CI and refuses to run unless `REVIU_GITHUB_SMOKE=1` is set.

Default fixture expectations:

- repository: `reviu-dev/reviu-github-smoke`
- visibility: private
- stable open PR: `smoke/pr-open` -> `main`

Run it from `desktop/`:

```sh
cargo build -p reviu_driver --bins
REVIU_PROFILE=dev API_BASE_URL=http://localhost:3001 REVIU_GITHUB_SMOKE=1 \
  REVIU_AUTH_TOKEN="$(security find-internet-password -s reviu_auth.dev -a bearer -w)" \
  target/debug/reviu-github-smoke \
  --backend test \
  --driver-bin target/debug/reviu-driver \
  --repo /Users/joris/workspace/reviu-github-smoke \
  --require-bot
```

Current scope is intentionally safe for the fixture repo. The multi-actor path creates one temporary review comment with the bot account and deletes it before exiting.

- verifies `gh` can read the private GitHub fixture repo
- verifies the stable fixture PR exists
- opens the live checkout through `reviu-driver --backend test|visual`
- prints Reviu auth diagnostics with `auth_state`
- uses `REVIU_AUTH_TOKEN` when provided so local and future CI runs do not depend on platform keychain access
- verifies Reviu detects the GitHub remote
- verifies Reviu resolves the current branch to the stable fixture PR
- opens the Pull Request dock tab and verifies the changed files match the fixture PR
- when bot credentials are available, creates a temporary review comment as `joris-gallot-bot` and verifies Reviu sees it as the primary user
- creates a pending PR review comment as the primary Reviu user, verifies the Review panel lists it, discards the pending review, and verifies it disappears
- fetches and verifies the fixture PR branch is available locally

Useful options:

- `--repo <path>`: local checkout of the fixture repo, required.
- `--backend test|visual`: driver backend. `test` is enough when `REVIU_AUTH_TOKEN` is set; `visual` is useful for screenshot-oriented local debugging.
- `--driver-bin <path>`: use a prebuilt driver binary instead of `cargo run`.
- `--auth-token-env <env>`: environment variable containing a Reviu API bearer token, default `REVIU_AUTH_TOKEN`. On macOS dev builds, you can usually populate it from `security find-internet-password -s reviu_auth.dev -a bearer -w`.
- `--bot-token-env <env>`: environment variable containing the bot GitHub token, default `REVIU_GITHUB_BOT_TOKEN`. If it is missing, the runner tries `gh auth token --user joris-gallot-bot`.
- `--bot-gh-user <user>`: expected bot login, default `joris-gallot-bot`.
- `--require-bot`: fail if the bot credential is unavailable instead of skipping the multi-actor check.
- `--owner <owner>` and `--name <repo>`: override the expected GitHub repository.
- `--pr-branch <branch>`: override the expected open PR branch.
- `--keep-temp`: keep temporary driver config and logs.

## `reviu-perf`

`reviu-perf` wraps the driver with repeatable local performance scenarios. It creates temporary fixtures, can use the stub agent, and samples process stats.

List available options:

```sh
cargo run -p reviu_driver --bin reviu-perf -- --help
```

Use it for local perf investigation, not as a correctness smoke suite.
