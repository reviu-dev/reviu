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

- Public platform messaging that is safe today: `macOS (Apple Silicon and Intel)`, `Windows (ARM64 and x64)`, and `Linux` (install script for `x86_64` and `aarch64`).
- Local Git features do not require an account.
- GitHub features require `Sign in with GitHub` and `Reviu Pro`.
- Current pricing shown in the app and landing: `$9/month` or `$79/year`.
- Current landing and terms also message a `14-day free trial`.
- Browser extensions are shipped for Chrome and Firefox. Safe claim: open GitHub repositories, pull requests, and issues in Reviu from the browser.
- The dashboard is an internal/admin surface, not a marketing surface.

If pricing changes, update desktop billing, landing copy, structured data, and terms together.

## Free

### Local Git workflow

- Open local repositories and switch between recent repositories.
- Review repository status with staged, unstaged, partially staged, and conflicted files.
- View and edit local changes in the real-time diff editor with inline and split diff modes.
- Stage, unstage, and restore by file or by hunk, including stage all and unstage all flows.
- Commit changes, amend the latest commit, and undo the last commit.
- Switch, create, create from base, and delete branches from local or remote refs.
- Merge branches and abort in-progress merges.
- Fetch, pull, push, and force push with lease.
- Rebase flows: start, continue, skip, abort, rebase onto a branch, edit branch commits, rebase the last N commits, and interactive rebase.
- Cherry-pick one or more commits.
- Stash flows: stash, stash with untracked files, apply, pop, and drop.
- Resolve conflicts with dedicated current, incoming, both, and accept-all actions.
- Navigate hunks and conflicts from the keyboard with focused hunk highlighting.

### AI agent panel

- Built-in agent panel in the local Git sidebar, connecting to Claude or Codex through the Agent Client Protocol with a user-supplied API key.
- Chat with the agent about local changes, with conversation history persisted per repository.
- Plan/Build session modes for Claude and reasoning-effort selection for Codex.
- Visible tool calls, plans, thoughts, diff summaries, and tool outputs inside the chat.
- Review-to-agent inline comments on diff lines, with Draft, Copied, Addressed, and Outdated states and an export action to send the batch to the agent.

### Local UX strengths

- Keyboard-first command palette for local Git actions, with grouped commands, recent commands, visible disabled states with short reasons, and success/error feedback.
- Configurable keyboard shortcuts for core Git, navigation, review, and app actions.
- File search on code and diff pages.
- Persistent app settings for theme, font size, diff view, whitespace handling, indentation guides, and editor display options.
- Dedicated Git config page inside the app.
- Contextual refresh from the app header.

## Reviu Pro

### GitHub home

- GitHub notifications feed in-app, with search, repository grouping, unread state, mark-as-done, and click navigation.
- Unread counts in the app, user avatar, macOS dock and menu bar, and Windows/Linux system tray.
- Saved pull request lists with filters for repositories, labels, authors, assignees, requested reviewers, review state, draft visibility, base branch, and sorting.
- Manage saved pull request lists: create, edit, delete, and reorder.
- Repository sections with search, recently updated repositories, pinned repositories, private repository visibility, and collapsible groups.
- Search GitHub repositories from the command palette.
- Upgrade and sign-in flows directly in the desktop app.

### GitHub repository context

- Browse repository `Overview`, `Readme`, `Code`, `Pull Requests`, and `Issues`.
- View repository overview metadata: README, description, language breakdown, contributors, recent commits, stars, and forks.
- Keep `Readme` and `Code` aligned with the selected branch.
- Open files directly inside Reviu instead of bouncing to the browser.
- Render repository content with Markdown, SVG, and image previews where supported.
- Open README relative links inside the repository `Code` tab.
- Star, unstar, watch, fork, clone, and create GitHub repositories from Reviu.
- Open GitHub user profiles with profile links, follower counts, repository totals, language mix, and recent repositories.
- Open repository commit details with metadata, co-authors, changed files, and inline or split diffs.
- Browse repository pull requests with open, merged, and closed tabs plus search and filters.
- Browse repository issues with open, closed, and not-planned tabs plus search and filters.
- View issue details, edit issue descriptions, preview issue descriptions and comments before posting, create/edit/delete issue comments, and add/remove issue reactions in-app.

### Pull request review

- Open pull requests directly inside the desktop app.
- Review changed files in inline or split diff modes, with syntax highlighting, file navigation, and optional unchanged local files.
- Render Markdown, SVG, image previews, GitHub-hosted assets, linked GitHub code references, and suggested-change diffs in review flows.
- Create, reply to, edit, delete, and preview review comments before posting.
- Resolve and unresolve pull request review threads.
- Insert suggestions from selected diff lines and apply suggested changes as commits to the pull request branch with reviewer co-author support.
- Add and remove reactions on pull request descriptions, issue comments, review comments, and reviews.
- Use emoji autocomplete and drag-and-drop image upload in pull request and issue composers.
- Submit reviews from the PR page, including comment, approve, and request changes flows.
- Manage pull request labels, assignees, reviewers, and draft/ready-for-review state.
- See merge readiness, conflicts, out-of-date state, skipped checks, required checks, and provider images inside the PR view.
- Merge pull requests from Reviu when the repository state allows it, including merge commit, squash, and rebase methods.
- Enable and disable auto-merge when the pull request is blocked by pending checks or reviews.
- Browse pull request commits with search and filtering, and review pull request changes commit by commit.
- Keep pull request commit links inside Reviu when they point to the current review.
- AI pull request briefs (Reviu Pro) that use a user-provided OpenAI or Anthropic key to produce a summary, files to review first by priority, risks, and blockers, with file links that open in the PR changes view.

### Local-to-GitHub bridge

- Jump from the local Git page to the pull request for the current branch.
- Create a pull request from the Git page when a branch is ready to publish, including pull request templates and draft mode.
- Switch a local repository to the current pull request branch from the PR flow.
- Copy the current pull request branch name.
- Open GitHub URLs in Reviu through the command palette.
- Open GitHub repositories, pull requests, and issues in Reviu through the Chrome and Firefox extensions.

## Desktop surfaces beyond the core pitch

- Billing page with subscription state, free-trial entry point, and billing portal access.
- In-app feedback flow.
- Startup crash recovery notification with one-click crash reporting.
- Desktop update checking and release download flow.
- Settings and About pages inside the app.
- Native macOS app menu, macOS menu bar notifications, Windows/Linux tray notifications, and dock badge support where available.

These are not the main hero features, but they matter for polish, support, and retention.

## Backend-backed capabilities

The backend is not just auth glue. It carries core product behavior for Pro.

- GitHub OAuth sign-in and authenticated user state.
- Subscription and billing lifecycle for Reviu Pro.
- GitHub API-backed routes for notifications, repositories, pull requests, issues, comments, reactions, checks, merge readiness, user profiles, repository creation, and repository forking.
- GitHub API caching with public/viewer/installation scopes, conditional requests, and admin cache metrics.
- Asset upload and private GitHub asset resolution for comment images.
- Desktop update metadata and download routes.
- Feedback submission and crash report intake.
- Admin dashboard routes for health checks, user management, and GitHub cache inspection.

## What the landing is currently saying

The landing is broadly aligned with the current product and now leads with Reviu as a native Git client for GitHub pull request review. It still compresses the deepest Pro workflows in the details.

Accurate themes already present on the landing:

- Reviu is a desktop Git client.
- The local Git workflow is keyboard-first.
- Reviu Pro is the GitHub layer.
- Pricing is `$9/month` or `$79/year` with a `14-day free trial`.
- Platform messaging covers `macOS (Apple Silicon and Intel)`, `Windows (ARM64 and x64)`, and `Linux` (install script).
- Browser extensions for Chrome and Firefox are present.
- Saved pull request lists, GitHub notifications, repository browsing, pull request review, checks, merge, and issue browsing are present.
- The hero, workflow section, and pricing copy now position Pro as the GitHub review layer on top of free local Git.
- Switcher copy speaks directly to developers coming from Fork, GitHub Desktop, Tower, or GitKraken.

What the current landing underrepresents:

- The depth of the local Git workflow: merge, branch deletion, advanced rebase modes, stash variants, hunk-level restore, conflict keyboard actions, and configurable shortcuts.
- Diff editor polish: histogram diffs, precise word highlights, hide-whitespace mode, image previews, and indentation guide options.
- Repository actions: star, watch, fork, clone, create repository, commit details, and GitHub profiles.
- PR review depth is present at a high level, but still compresses comment/description previews, suggested changes, linked code reference previews, reactions, emoji autocomplete, image upload, labels, assignees, reviewers, draft/ready state, auto-merge, and commit-by-commit review.
- Issue editing and issue comment/reaction flows.
- Desktop and support polish like crash recovery, in-app updates, feedback, menu/tray/dock badges, and the internal admin dashboard.

## Copy guardrails

Good claims:

- "Free local Git client."
- "Keyboard-first Git workflow."
- "GitHub notifications, repositories, pull request reviews, and issues in Reviu Pro."
- "Native desktop app built with Rust and GPUI."
- "Chrome and Firefox extensions open GitHub repositories, pull requests, and issues in Reviu."

Avoid these unless they are explicitly shipped and verified:

- Team workflows or enterprise features that do not exist yet.
- AI review, AI code generation, AI suggested changes, or automated code-fix claims. The AI pull request brief feature is allowed: claim it as a triage helper that uses the user's own OpenAI or Anthropic key, never as an AI reviewer.
- Self-hosted Git provider support.
- Free GitHub workflow claims. GitHub workflows are Pro-gated.
- Pricing that does not match app billing and landing copy.

## Suggested short description

Reviu is a native Rust + GPUI desktop Git client for fast review workflows. Use Free for full local Git, then upgrade to Reviu Pro for GitHub notifications, repository browsing, pull request review, issues, and browser-to-desktop GitHub shortcuts in one keyboard-first app.
