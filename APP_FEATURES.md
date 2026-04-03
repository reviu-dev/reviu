# Reviu - Product Snapshot

## What Reviu is

Reviu is a native desktop Git client built for fast review workflows.

Core product shape:

- `Free` covers local Git workflows.
- `Reviu Pro` adds GitHub workflows directly inside the app.
- The product is keyboard-first, editor-oriented, and built to keep local Git and GitHub context in one place.

## Product positioning

- Desktop-native Git workflow, not a browser tab workflow.
- Fast diff and review experience, not just a repository manager.
- Keyboard-first navigation with command palette, shortcuts, and file search.
- Native Rust + GPUI app, not Electron and not a webview.

## Current product truth

- Public platform messaging that is safe today: `macOS (Apple Silicon)`.
- Local Git features do not require an account.
- GitHub features require `Sign in with GitHub` and `Reviu Pro`.
- Current pricing shown in the app and landing: `$19/month`.
- Current landing and terms also message a `14-day free trial`.

If pricing changes, update desktop billing, landing copy, structured data, and terms together.

## Free

### Local Git workflow

- Open local repositories and switch between recent repositories.
- Review repository status with staged, unstaged, partially staged, and conflicted files.
- View local changes in inline and split diff modes.
- Stage, unstage, and restore by file or by hunk.
- Commit changes, amend the latest commit, and undo the last commit.
- Switch branches and create branches from local or remote refs.
- Fetch, pull, push, and force push with lease.
- Rebase flows: start, continue, skip, abort, and interactive rebase.
- Cherry-pick one or more commits.
- Stash flows: stash, stash with untracked files, apply, pop, and drop.
- Resolve conflicts with dedicated conflict actions.

### Local UX strengths

- Keyboard-first command palette for local Git actions.
- File search on code and diff pages.
- Persistent app settings for theme, font size, diff view, and editor display options.
- Dedicated Git config page inside the app.

## Reviu Pro

### GitHub home

- GitHub notifications feed in-app.
- Unread counts and desktop-facing GitHub attention surface inside Reviu.
- Saved pull request lists and repository sections on the GitHub home page.
- Upgrade and sign-in flows directly in the desktop app.

### GitHub repository context

- Browse repository `Overview`, `Readme`, `Code`, `Pull Requests`, and `Issues`.
- Keep `Readme` and `Code` aligned with the selected branch.
- Open files directly inside Reviu instead of bouncing to the browser.
- Render repository content with Markdown support and file previews where supported.
- View issue details and work with issue descriptions and issue comments in-app.

### Pull request review

- Open pull requests directly inside the desktop app.
- Review changed files in inline or split diff modes.
- Render Markdown and SVG content in review flows.
- Create, reply to, edit, and delete review comments.
- Submit reviews from the PR page, including comment, approve, and request changes flows.
- See merge readiness and checks information inside the PR view.
- Merge pull requests from Reviu when the repository state allows it.

### Local-to-GitHub bridge

- Jump from the local Git page to the pull request for the current branch.
- Create a pull request from the Git page when a branch is ready to publish.
- Switch a local repository to the current pull request branch from the PR flow.
- Open GitHub URLs in Reviu through the command palette.

## Desktop surfaces beyond the core pitch

- Billing page with subscription state, free-trial entry point, and billing portal access.
- In-app feedback flow.
- Startup crash recovery notification with one-click crash reporting.
- Desktop update checking and release download flow.
- Settings and About pages inside the app.

These are not the main hero features, but they matter for polish, support, and retention.

## Backend-backed capabilities

The backend is not just auth glue. It carries core product behavior for Pro.

- GitHub OAuth sign-in and authenticated user state.
- Subscription and billing lifecycle for Reviu Pro.
- GitHub API-backed routes for notifications, repositories, pull requests, issues, comments, checks, and merge readiness.
- Desktop update metadata and download routes.
- Feedback submission and crash report intake.

## What the landing is currently saying

The landing is directionally correct, but it still compresses the product too much.

Accurate themes already present on the landing:

- Reviu is a desktop Git client.
- The local Git workflow is keyboard-first.
- Reviu Pro is the GitHub layer.
- Platform messaging stays on `macOS with Apple Silicon`.

What the current landing underrepresents:

- The depth of the local Git workflow.
- Hunk-level review and restore actions.
- The strength of the GitHub home surface.
- PR checks, merge readiness, and merge flows.
- Issue editing and issue comment flows.
- Desktop polish like crash recovery and in-app updates.

## Copy guardrails

Good claims:

- "Free local Git client."
- "Keyboard-first Git workflow."
- "GitHub notifications, repositories, pull request reviews, and issues in Reviu Pro."
- "Native desktop app built with Rust and GPUI."

Avoid these unless they are explicitly shipped and verified:

- `Windows` or `Linux` support.
- Browser extension claims.
- Team workflows or enterprise features that do not exist yet.
- Pricing that does not match app billing and landing copy.

## Suggested short description

Reviu is a native desktop Git client for fast review workflows. Use Free for local Git, then upgrade to Reviu Pro for GitHub notifications, repository browsing, pull request review, and issues in one keyboard-first app.
