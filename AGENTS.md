# Reviu Agent Guide

## Product context

- Reviu is a desktop Git client.
- `Free`: local Git workflow.
- `Reviu Pro`: GitHub integration (`$9/month` or `$79/year` in app billing UI).
- Core UX: keyboard-first navigation, fast diff/review workflows, in-app GitHub context.
- Product/copy source for landing: `APP_FEATURES.md`.

## Monorepo map

- `desktop/`: Rust + GPUI desktop app.
- `backend/`: Hono API + Better Auth (GitHub) + Polar billing.
- `landing/`: Astro + Vue marketing site.
- Git test playground: `/Users/joris/workspace/git-playground/`.
- dashboard: Vue + vue-shadcn + tailwind

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
  - `backend/src/routes/github.ts` (`/notifications`, `/pr/latest`)
- GitHub repo details (Overview, Readme, Code, PRs, Issues, branch select):
  - `desktop/crates/workspace/src/github_repo_page.rs`
  - `backend/src/routes/github.ts` (`/repos/:owner/:repo*`)
- GitHub PR details and review (inline/split diff, comment create/edit/reply/delete):
  - `desktop/crates/workspace/src/github_pr_details_page.rs`
  - `backend/src/routes/github.ts` (`/pr/:id*`)
- Desktop API client/backend contract:
  - `desktop/crates/workspace/src/api.rs`
  - `backend/src/routes/github.ts`
  - `backend/src/services/github.ts`
- Markdown/GFM rendering:
  - `desktop/crates/gfm_markdown_viewer/src/gfm_markdown_viewer.rs`

## Required workflow

- Search in codebase: `osgrep "query"` (or `rg` when needed).
- Always use Context7 MCP for library/API docs, setup/config, and codegen guidance.
- Add tests for each feature/fix.
- **Changelog**: after each feature, add an entry to `CHANGELOG.md`. Use the next unreleased version section (create it if it doesn't exist). Follow the existing format: `## X.Y.Z` heading, then `### Feature Title` with a short paragraph. Keep changelog copy user-facing and outcome-focused. Do not describe internal implementation details unless they matter to users.
- **Copy tone**: avoid cliché phrases like "at a glance". Prefer direct alternatives ("immediately", "quickly", or restructure the sentence).

## Validation commands

- Desktop:
  - `cd ./desktop/ && cargo check`
- Backend:
  - `cd ./backend/ && pnpm typecheck`
- Landing:
  - `cd ./landing/ && pnpm typecheck`

## GPUI rules

- Framework: GPUI.
- For desktop icons, use gpui-components icons (IconName) or our custom icons (UiIconName), all coming from lucide
- GPUI tip: for `on_click` and overflow containers, set an `id`.
- Examples:
  - `./desktop/gpui`
  - `./desktop/gpui-components`

## Backend notes

- Framework: Hono.
- Auth: Better Auth (GitHub OAuth) + Polar subscriptions.
- Auth OpenAPI schema:
  - `http://localhost:3000/api/auth/open-api/generate-schema`

## Dashboard

- If you need to add new component from shadcn you can do `pnpm dlx shadcn-vue@latest add <component>` in the dashboard folder
- Use vueuse for utils functions
