# Reviu driver

`reviu_driver` mounts the real `WorkspaceView` in a GPUI test window and accepts JSON-lines commands on stdin. It is useful for agent-driven UI checks, smoke tests, and local perf probes without launching the packaged app.

## Build

From `desktop/`:

```sh
cargo build -p reviu_driver --bins
```

This builds:

- `target/debug/reviu-driver`
- `target/debug/reviu-git-smoke`
- `target/debug/reviu-perf`

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
{"cmd":"dialog_state"}
{"cmd":"confirm_dialog"}
{"cmd":"notification_log"}
{"cmd":"quit"}
```

The test backend also supports selector-driven commands such as `bounds`, `click`, `type`, `key`, `clock`, `wait`, and `park`. The visual backend supports point clicks and `screenshot`.

The driver reads and writes real repositories. Use temporary repositories for repeatable tests.

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

## `reviu-perf`

`reviu-perf` wraps the driver with repeatable local performance scenarios. It creates temporary fixtures, can use the stub agent, and samples process stats.

List available options:

```sh
cargo run -p reviu_driver --bin reviu-perf -- --help
```

Use it for local perf investigation, not as a correctness smoke suite.
