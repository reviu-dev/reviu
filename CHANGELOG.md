# Changelog

All notable changes to Reviu are documented here.

## 0.4.0

### Windows Support

Reviu now runs on Windows, install it with the `.exe` installer from the downloads page, and get the same in-app update flow as macOS and Linux.

### Firefox Extension

A Firefox extension is now available alongside the existing Chrome extension, letting you open any GitHub repository, pull request, or issue directly in Reviu from Firefox.

### Issue Reactions

Issue details now show GitHub reactions on the description and comments, and you can add or remove your own reactions directly from the issue.

### Emoji Autocomplete in Comments

Pull request and issue comment editors now suggest emojis in a wider picker when you type `:`, filter the list as you keep typing, and insert the selected emoji with Enter.

## 0.3.0

### Chrome Extension

A new Chrome extension lets you open any GitHub repository, pull request, or issue directly in Reviu with one click.

### Repository Overview Redesign

The repository overview tab has been redesigned with a language breakdown, contributor avatars, recent commits, and the ability to star or unstar a repo.

### Image Preview in Repository Code Tab

Selecting an image file (PNG, JPEG, WebP, GIF, etc.) in the repository code tab now shows an inline preview instead of raw binary content.

### Pull Request Reactions

Pull request details now show GitHub reactions on the description, comments, and reviews, and you can add or remove your own reactions.

## 0.2.0

### Repository Pull Request Search and Filters

Repository pull request lists now split open, merged, and closed pull requests into separate tabs with search and a sidebar for filtering by labels, people, review state, draft visibility, base branch, and sorting.

### Repository Issue Search and Filters

Repository issue lists now split open, closed, and not planned issues into separate tabs with search and a sidebar for filtering by labels, authors, assignees, and sorting.

### Pull Request Check Provider Images

Pull request checks now show provider images when GitHub exposes them, making CI, security, and automation results easier to recognize.

### Repository README Links

Relative file links in GitHub repository READMEs now open the Code tab with the referenced file selected.

## 0.1.0

### Pull Request People Management

Pull request overview pages now let you assign teammates and request reviews directly in Reviu, with inline suggestions and quick removal for each person.

### Pull Request Label Editing

Pull request details now let you add or remove labels directly from the overview.

### Commit Links in Pull Request Comments

Commit links inside pull request comments now stay inside the current review when they point to a commit from the same pull request, clicking one of those links opens the Changes tab and selects the matching commit in the existing commit filter.

### Markdown Code Block Rendering

Markdown previews now render preformatted code blocks more cleanly, so diagrams and box-drawing tables keep their spacing without editor-style whitespace markers.

## 0.0.13

### Edit Branch Commits (In-Place Interactive Rebase)

The interactive rebase menu now offers an "Edit branch commits" option that lets you reorder, squash, or edit your branch's commits without pulling in upstream changes. This runs `git rebase -i --onto <merge-base>`, so your commits stay exactly where they diverged from the base branch.

### Smarter Diffs

Diffs now use the histogram algorithm, producing more accurate results when code is wrapped or unwrapped. Inner lines that stay the same are correctly shown as unchanged instead of being marked as fully removed and re-added.

### Hide Whitespace

A new "Hide whitespace" toggle in the diff editor header lets you ignore indentation-only changes. When enabled, lines that differ only by leading whitespace appear as context. The default can be set in Settings under Editor.

## 0.0.12

### Linux Support

Reviu now runs on Linux, install it with a single command from the downloads page, and get the same in-app update flow as macOS.

### Startup Crash Reports

When Reviu recovers from a Rust panic, the next launch now shows a persistent notification to send a report to the team.

### GitHub Label Colors

Pull request and issue labels now reuse their GitHub colors across home lists, repository lists, and detail pages, so labels stay easier to recognize throughout the app.

## 0.0.11

### GitHub Home Tab Manager

GitHub home now includes a dedicated `Manage tabs` view for your pull request lists. You can create, edit, and delete saved tabs from one place, reuse filters for repositories, labels, authors, assignees, requested reviewers, and review state, and keep `@me`-based lists like `My Open PRs` and `Need Review` easy to recreate and refine.

### GitHub Home Layout

Notifications now sit above your repositories on the GitHub home page, so the repositories list no longer stretches the full column height and the left side is easier to scan.

### Collapsible GitHub Repo Sections

GitHub home pull request lists and notifications now let you click each repository section to collapse or expand its items.

### Image File Previews in Review Editors

Git and pull request review editors now render PNG, JPEG, WebP, GIF, BMP, TIFF, and ICO files as previews instead of showing raw binary text. Unsupported binary files now show a clear placeholder so large review panes no longer fill with unreadable content.

### Contextual Refresh in the App Header

Reviu now includes a refresh button in the app header with a `Cmd-R` shortcut, so you can refresh the current Git, GitHub, repository, or pull request page without navigating away from what you are reviewing.

## 0.0.10

### More Precise Word Diff Highlights

Word diff highlights are now more precise, Reviu can better show the exact inserted text inside function names in inline and split diff views instead of marking the full name as changed.

### Pull Request Conflicts

Pull request overview pages now highlight merge conflicts and out-of-date branches more clearly, when Reviu finds the local repository the warning itself can open the Git page on the pull request branch so you can resolve conflicts or update the branch more quickly.

### Pull Request Changes Search Performance

Searching file contents from the pull request changes tree is now faster when you include unchanged local files from the current branch, Reviu now scans the local HEAD snapshot in a single pass.

## 0.0.9

### Global Git and GitHub Switch

The app header now includes a persistent Git and GitHub switch from anywhere in the app, GitHub notifications live on the GitHub tab itself.

### Branch Pull Request Shortcut in Git Header

The Git page header now checks whether the current GitHub branch already has an open pull request. When it does, you can open that pull request, when it does not, the same spot opens a create pull request dialog.

### Git Error Notifications

Failed Git actions on the Git page now show notifications instead of failing silently. This covers branch switching, fetch, pull, push, amend, undo last commit, and stage or restore operations.

### Configurable Keyboard Shortcuts

Settings now includes a Keyboard Shortcuts page where you can remap desktop shortcuts.

### Keyboard-First Desktop Shortcuts

Reviu now includes direct shortcuts for settings, terminal toggle, branch switching, Git sidebar mode, diff view toggle, and switching to the current pull request branch.

### Pull Request Draft Status Actions

The pull request page can now switch an open pull request between draft and ready for review.

## 0.0.8

### Staged and Unstaged File Groups

The Git sidebar now separates changed files into "Staged Changes" and "Changes" groups, so you immediately know what will be committed. Partially staged files appear in both groups. A new "Unified File View" option in Settings lets you switch back to the previous flat list.

### Persistent Diff View Mode

Your preferred diff view mode (inline or split) is now saved across sessions. Toggle once in the Git page or PR review, and Reviu remembers your choice. A new "Split Diff View" option is also available in Settings under the Editor group.

### Git Pull Command and Clickable Branch Indicators

A new "Pull" command is available in the command palette to pull changes from the remote branch. The ahead/behind indicators in the Git page header are now clickable - click the up arrow (↑) to push, or the down arrow (↓) to pull.

### Branch Selector Sorted by Recent Activity

The branch selector on the Git page now sorts branches by most recent commit, so your active branches appear first. Local branches still appear before remote branches.

---

## 0.0.7

### macOS Status Bar for GitHub Notifications

Reviu now lives in your macOS menu bar. See your unread GitHub notification count instantly and browse the latest notifications directly from the status bar dropdown — without switching to the app.

### Customizable Font Size

A new font size setting lets you scale the entire interface to your preference. Adjust the base size between 12 px and 24 px from Settings — every element scales proportionally in real time.

### Pin Favorite GitHub Repositories

Pin your most-used repositories to keep them at the top of the GitHub home list. Hover any repository row and click the pin icon to toggle. Pinned repos persist across sessions.

### macOS Native App Menu

Reviu now includes a native macOS menu bar with quick access to navigation (Git, GitHub, Git Config, Billing), Settings, About, and standard Edit actions (Undo, Redo, Cut, Copy, Paste, Select All).

### Stage, Unstage, and Restore Files Inline

You can now stage, unstage, or discard changes on individual files directly from the changes list on the Git page. Action buttons appear on hover for a faster, more precise staging workflow.

---

## 0.0.6

### In-App Feedback Dialog

Send bug reports and feature requests without leaving Reviu. A new feedback dialog lets you pick a type, enter a title and description, and submit directly to the team.

### GitHub Asset Image Rendering

Markdown content in pull request descriptions, comments, and reviews now renders GitHub-hosted images. Private asset URLs are resolved and signed automatically so images display inline.

### Pull Request Review Status Indicators

The pull request overview now shows clear visual indicators for each review state — approved, changes requested, commented, or dismissed — with color-coded icons and labels.

### Notification Count Badge

Your unread GitHub notification count is now visible on the user avatar and the macOS app dock icon, so you always know when something needs your attention.

### Notification Click Navigation

Clicking a notification navigates directly to the relevant page — pull request details, repository issues, or the GitHub web URL — and marks it as read automatically.

---

## 0.0.5

### Automatic App Updates

Reviu now checks for updates on launch. When a new version is available, a notification appears with a one-click download and install flow. Updates are verified with SHA-256 checksums before installation.

### Pull Request CI Checks

The pull request overview displays CI check status with a detailed summary — total, passing, failing, pending, and required checks — so you can quickly assess merge readiness.

### Pull Request Review Actions

Submit reviews directly from the pull request changes tab. Approve, request changes, or leave a comment with a review body — all without leaving the app.

### Merge Pull Requests

Merge pull requests from Reviu with support for merge commit, squash, and rebase strategies. The merge dialog validates readiness and lets you customize the commit title and message.

### GitHub Notifications Feed

A dedicated notifications tab on the GitHub home page shows your latest GitHub notifications grouped by repository, with unread indicators, search filtering, and a mark-as-done action.

---

## 0.0.4

### GitHub Repository Overview

Browse any GitHub repository directly in Reviu with an overview tab showing the README, description, language, stars, and forks.

### Pull Request Overview and Conversation

View pull request details with a full conversation timeline — description, comments, reviews, and replies — rendered with GitHub-Flavored Markdown.

### Pull Request Changes Diff

Review pull request file changes with inline and split diff view modes, syntax highlighting, and file-level navigation.

### Pull Request Commits List

Browse the list of commits in a pull request from the context sidebar, with search and filtering support.

### Editor Indent Rainbow

A new setting colors indentation guides by nesting level in the diff editor, making it easier to follow code structure visually.

---

## 0.0.3

### GitHub Integration

Connect your GitHub account to list your open pull requests and repositories needing review, all from the GitHub home page inside Reviu.

### Command Palette with Branch Operations

The command palette now supports switching branches, creating new branches, and creating branches from a base — all triggered with a keyboard shortcut.

### Settings Page with Theme Selection

A new settings page lets you toggle between dark and light themes, or enable auto-switching to follow your macOS system appearance.

---

## 0.0.2

### Advanced Git Operations via Command Palette

The command palette now handles merge, rebase, and interactive rebase workflows. Start, continue, skip, or abort a rebase directly from the palette. Merge conflicts are surfaced inline.

### Git Stash Support

Stash and restore your work-in-progress changes from the command palette. Browse your stash list, apply or drop entries, and keep your working tree clean between tasks.

---

## 0.0.1

### Local Git Workflow

Stage, unstage, and commit changes with a keyboard-first interface. Reviu handles the core Git operations you use every day — fetch, push, pull, and branch management.

### Repository and Branch Management

Add repositories to Reviu and switch between branches from the main Git page. The branch selector supports detached HEAD state and displays the current branch at all times.

### Diff Editor with Syntax Highlighting

View file diffs with inline and split modes, full syntax highlighting, and color-coded additions and deletions. The editor renders diffs in real time as you navigate the changes list.
