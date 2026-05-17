# Changelog

All notable changes to Reviu are documented here.

## 0.13.0

### Pull Request Target Branch Editing

GitHub pull request details now let you change the target branch from Reviu. The overview refreshes after the change so the diff, commits, checks, and merge state match the new target.

### Linux System Tray Startup

Reviu now checks the Linux system tray runtime before creating the tray icon, preventing startup crashes in environments where GTK or AppIndicator is not available. If the tray cannot be created, the app continues without the tray icon.

## 0.12.0

### Review Comments For An AI Agent

Collect review comments on the Git page, copy them as structured markdown to send to an AI agent.

### Review Comment Shortcut

Starts a review comment on the focused hunk from your keyboard. Works on Git page and GitHub PR.

## 0.11.0

### File Search Includes Project Files

The Git page file search now includes tracked project files alongside changed files. Changed files stay grouped at the top, with unchanged files below, so keyboard navigation can jump to any file without hiding active work.

### Exclude Labels From Pull Request Lists

GitHub pull request lists can now exclude labels, matching GitHub's `-label:` search syntax. Saved lists and repository pull request filters can hide teams, dependencies, or other labels that do not belong in the current review queue.

### Git Command Preview

Merge and rebase branch pickers now show the Git command that will run for the selected target. Interactive rebase commit-count entry also previews the `git rebase -i HEAD~n` command before it starts.

### Better Rebase Branch Choices

Rebase branch pickers no longer default to the current local branch. Reviu now prioritizes the current upstream and default branch when choosing a rebase base, and the interactive rebase option is labeled more clearly as editing commits since a branch.

## 0.10.0

### AI Pull Request Briefs

Reviu Pro can now connect to a user-provided OpenAI or Anthropic key and generate a concise AI brief from a GitHub pull request overview. The brief highlights the summary, files to review first, risks, and blockers, with file links that open directly in the PR changes view.

### Edit Commits Now Handles Merge Commits

Edit Commits no longer refuses to start when the selected range contains merge commits. Reviu shows a confirmation dialog telling you how many merges will be dropped, then continues like `git rebase -i` does on the command line.

### Clearer Command Palette Availability

The Git command palette now keeps temporarily blocked actions visible with a short reason, so you can see why an action is not ready yet.

### Open Merge and Review Popovers from the Command Palette

The command palette on a pull request now exposes "Merge pull request" and "Review pull request" actions. Picking one opens the same popover as the header buttons, so you can submit a review or merge without leaving the keyboard.

## 0.9.0

### Preview Comments and Descriptions Before Posting

Every comment and description composer in the app now has Write and Preview tabs. Switch to Preview to see exactly how the rendered markdown, including issue references, code references and emoji shortcodes, will appear before you post.

### Insert a Suggestion While Reviewing a Pull Request

The PR Changes review composer now shows a Suggest button when commenting on the right side of the diff. Clicking it inserts a `suggestion` code block prefilled with the lines you selected, ready to edit before posting. Reviewers see the same diff-style block they can apply with one click.

### Review Pull Requests Commit by Commit

A new toggle in the PR Changes header switches between reviewing all changes at once and stepping through one commit at a time, starting from the oldest. Previous and next buttons walk through the commits with a position indicator showing where you are, and Cmd-Shift-C, Cmd-Alt-Shift-Left, and Cmd-Alt-Shift-Right drive the same flow from the keyboard.

## 0.8.0

### Keyboard Navigation on the Git Page

The focused hunk is highlighted with a blue gutter and border so you can see where you are while stepping through changes. Tab now cycles through the file list, and dedicated shortcuts act on the focused hunk or selected file: Shift-Enter and Shift-Backspace stage or restore the hunk, Cmd-Enter and Cmd-Backspace stage or restore the whole file.

### Keyboard Conflict Resolution

Resolve merge conflicts without leaving the keyboard. Cmd-Alt-Up and Cmd-Alt-Down step through conflicts, then Shift-Enter accepts the current change, Shift-Backspace accepts the incoming change, and Cmd-Shift-Enter accepts both. After each resolution Reviu jumps to the next conflict so you can clear them all in a row.

### Keyboard Navigation on GitHub Pages

Cmd-Alt-Left and Cmd-Alt-Right cycle through tabs on the GitHub home, repository, and pull request pages. In PR Changes and commit details, Cmd-Alt-Up and Cmd-Alt-Down jump between changes; Cmd-Alt-Shift-Up and Cmd-Alt-Shift-Down move through review comments - and now work on the PR Overview tab too, with the active conversation outlined and a floating counter showing your position in the list. Cmd-D marks the focused notification as done from the GitHub home list.

### Clearer Feedback for Command Palette Actions

Command palette actions now confirm success with a toast, including "Already up to date" for rebase, merge, and pull when there's nothing to do. Errors keep the palette open so you can retry, and commands that don't apply in the current state are hidden.

### Skipped Checks in Pull Request Overview

Skipped GitHub checks now appear separately from successful ones, with their own gray slice in the overview ring and count in the summary. Each row shows its status and duration, and jobs are prefixed with the workflow name.

## 0.7.0

### Recent Commands in the Command Palette

The command palette now shows a Recent section at the top listing the commands you use most often, weighted so recent and frequent commands both surface first.

### Jump Between Changes in a File

The previous/next arrows above the diff now step through every change in the file when there are no conflicts. The existing `cmd-alt-up` and `cmd-alt-down` shortcuts work in both modes: conflicts when present, otherwise hunks.

### GitHub Notifications in the System Tray

Windows and Linux now show the Reviu tray icon with unread GitHub notification counts, matching the existing macOS menu bar behavior.

### GitHub Review Suggestions

Suggested changes in review comments render as GitHub-style diff rows and can be committed directly to the pull request branch with a custom title, optional message, and the reviewer as co-author.

### GitHub Resolve Conversations

Review comment threads on a pull request can now be resolved and unresolved directly from the Reviu overview. Resolved threads collapse into a summary with a Show toggle so the discussion stays out of the way once it's addressed, and the Resolve / Unresolve button respects GitHub's permissions.

### Drag-and-drop Image Upload

Drag a PNG, JPEG, GIF, or WebP file onto any comment, reply, review, or description composer in a pull request or issue to upload it and embed it as markdown. A placeholder appears immediately while the upload runs and is replaced by the final image link when it completes.

### GitHub Mentions in Markdown

User mentions in GitHub markdown now show the profile avatar next to the linked username, matching the way mentions appear on GitHub while keeping profile navigation inside Reviu.

### GitHub Commit Co-Authors

Pull request timelines, repository overviews, and the commit details page now show commit co-authors when GitHub includes them, making shared work visible during review.

## 0.6.0

### Auto-merge for Pull Requests

The merge popover on a pull request now includes an Enable auto-merge action when the PR is blocked by pending reviews or checks. Pick a merge method and Reviu asks GitHub to merge the PR as soon as the requirements are satisfied. A Disable auto-merge button appears while it is active.

### Search GitHub Repositories

Find any repository on GitHub directly from the command palette. Trigger "Search GitHub repository", start typing, and Reviu shows matching repositories live with names, descriptions, and star counts. Press Enter to open the selected repository in Reviu.

### Grouped Command Palette

Command palette entries are now grouped by purpose, Changes, Sync, Branches, Stash, Pull request, Repository, GitHub, Navigation, and more, with section headers that make it easier to scan and find the right command.

### GitHub Profiles

Open a GitHub user profile inside Reviu to see their avatar, profile links, follower counts, repository totals, language mix, and recent repositories without leaving the desktop app.

### Watch Repositories

The repository page now includes a Watch button next to Star and Fork. Open the dropdown to switch between "Participating and @mentions", "All Activity", and "Ignore".

## 0.5.0

### Commit Details in Reviu

Repository commit links now open inside Reviu with commit metadata, changed files, and an inline or split diff, keeping review context in the desktop app.

### Create GitHub Repository

Create a new repository directly from Reviu. Pick yourself or one of your organizations as owner, set name, description, and visibility, then Reviu opens the new repository as soon as it's created. Available from the GitHub home screen and the command palette.

### Fork Repository

Fork any repository from its repo page in Reviu. Pick yourself or one of your organizations as owner, optionally rename the fork, and Reviu opens the new fork as soon as it's ready. Defaults to copying only the default branch for a faster fork.

### Clone Repository

Clone any GitHub repository from its repo page with one click. Pick a parent folder, and Reviu runs the clone and opens the new local repository on the Git page. HTTPS is used by default, with an SSH option available in Settings under Git.

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
