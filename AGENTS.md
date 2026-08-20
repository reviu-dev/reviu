# Reviu Agent Guide

## Product context

- Reviu is a desktop app where a coding agent works and you review what it did, with a real git client under it.
- One view: agent sessions on the left, the conversation or the diff at the centre, the repository in the right dock.
- `Free`: local git workflow.
- `Reviu Pro`: GitHub integration (`$9/month` or `$79/year` in app billing UI).
- Core UX: keyboard-first navigation, fast diff/review workflows, in-app GitHub context.

## Repo map

- `desktop/`: Rust + GPUI desktop app.
- `website/`: Astro marketing site.
- `extension/`: browser extension.

The GitHub-integration backend (Reviu Pro) is closed-source in a separate private repo and is not part of this tree. Only the desktop-side API client (`api.rs`) lives here.

## Feature -> code map

- App entry + global keybindings:
  - `desktop/crates/reviu/src/main.rs`
- Workspace routing + subscription gating:
  - `desktop/crates/workspace/src/workspace.rs`
- Billing / subscription UI:
  - `desktop/crates/workspace/src/billing_page.rs`
- The workspace, the only view: agent sessions, the diff at the centre, the right dock:
  - `desktop/crates/workspace/src/session_page.rs` (+ `session_page/render.rs`, `session_page/agent.rs`)
- Right dock surfaces (Changes, Files, History, Pull request, Terminal):
  - `desktop/crates/workspace/src/dock_panel.rs`
- Working-tree changes with staging:
  - `desktop/crates/workspace/src/changes_list.rs`
- Git commands as data (23 one-shot commands, their messages and telemetry keys):
  - `desktop/crates/workspace/src/repo_command.rs`
- Which command the repository state allows:
  - `desktop/crates/workspace/src/repo_state.rs`
- Command palette actions (commit, fetch, push, rebase, stash, cherry-pick, etc.):
  - `desktop/crates/ui/src/command_palette.rs`
- GitHub sign-in, sign-out and the keychain:
  - `desktop/crates/workspace/src/auth_flow.rs`
- GitHub PR details and review (inline/split diff, comment create/edit/reply/delete):
  - `desktop/crates/workspace/src/github_pr_details_page.rs`
- Agent catalogue (ACP registry: which agents exist, how each one launches):
  - `desktop/crates/agent_registry/src/lib.rs` (+ `model.rs`, `assets/registry.json` snapshot)
- Desktop API client (talks to the GitHub-integration backend):
  - `desktop/crates/workspace/src/api.rs`
- Markdown/GFM rendering:
  - `desktop/crates/gfm_markdown_viewer/src/gfm_markdown_viewer.rs`

## UI conventions

- **Focus when a surface closes**: dialogs (command palette, file search, confirmations) restore focus by themselves via gpui-component's `Root`. Page-level surfaces (right dock, centre views) must hand focus to `SessionPage`'s `Focusable` impl on the next frame - it resolves to the diff editor or the composer - never to a hardcoded target.
- One dock surface = one entity in its own file, communicating by events; `session_page.rs` composes and routes.

## Driving the real app (reviu_driver)

- `desktop/crates/reviu_driver`: mounts the real `WorkspaceView` in a test window and takes JSON-lines commands on stdin, one response per line on stdout. Use it to verify UI behavior live without launching the app.
- Run: `cargo run -p reviu_driver` (from `desktop/`). Add `--agent-command <path>` to plug a fake agent; `cargo build -p agent_acp --features test-support --bin stub_agent` builds the stub (minimal ACP agent that acks every prompt) at `target/debug/stub_agent`.
- Verbs: `bounds` (painted bounds of a `debug_selector`), `click` (selector or point), `type`, `key` (e.g. `{"cmd":"key","keystrokes":"cmd-p"}`), `clock` (virtual ms, timers/debounces), `wait` (real ms, live processes), `park`, `path_prompt`, `quit`.
- Limits: it reads and writes the real config store and repositories (point it at a temp repo via `path_prompt`); no screenshots (macOS visual backend planned); animations do not run to completion under the test scheduler, judge end states.

## Required workflow

- Search in codebase: `osgrep "query"` (or `rg` when needed).
- Always use Context7 MCP for library/API docs, setup/config, and codegen guidance.
- Add tests for each feature/fix.
- **Changelog**: after each feature, add an entry to `CHANGELOG.md`. Use the next unreleased version section (create it if it doesn't exist). Follow the existing format: `## X.Y.Z` heading, then `### Feature Title` with a short paragraph. Keep changelog copy user-facing and outcome-focused. Do not describe internal implementation details unless they matter to users.
