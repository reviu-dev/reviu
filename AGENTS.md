# Reviu Agent Guide

## Product context

- Reviu is a desktop Git client.
- `Free`: local Git workflow.
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
- Local Git workspace page:
  - `desktop/crates/workspace/src/git_page.rs`
- Command palette actions (commit, fetch, push, rebase, stash, cherry-pick, etc.):
  - `desktop/crates/ui/src/command_palette.rs`
- GitHub home (notifications + latest PRs):
  - `desktop/crates/workspace/src/github_page.rs`
- GitHub repo details (Overview, Readme, Code, PRs, Issues, branch select):
  - `desktop/crates/workspace/src/github_repo_page.rs`
- GitHub PR details and review (inline/split diff, comment create/edit/reply/delete):
  - `desktop/crates/workspace/src/github_pr_details_page.rs`
- Desktop API client (talks to the GitHub-integration backend):
  - `desktop/crates/workspace/src/api.rs`
- Markdown/GFM rendering:
  - `desktop/crates/gfm_markdown_viewer/src/gfm_markdown_viewer.rs`

## Required workflow

- Search in codebase: `osgrep "query"` (or `rg` when needed).
- Always use Context7 MCP for library/API docs, setup/config, and codegen guidance.
- Add tests for each feature/fix.
- **Changelog**: after each feature, add an entry to `CHANGELOG.md`. Use the next unreleased version section (create it if it doesn't exist). Follow the existing format: `## X.Y.Z` heading, then `### Feature Title` with a short paragraph. Keep changelog copy user-facing and outcome-focused. Do not describe internal implementation details unless they matter to users.
