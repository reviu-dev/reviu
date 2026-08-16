# Changelog

All notable changes to Reviu are documented here.

## 1.0.0

### New Agent-First Workspace

Reviu now opens on a new Sessions workspace built around working with a coding agent. The left sidebar lists your agent sessions so you can switch between conversations or start a new one, the center is the agent conversation, and the right panel is reserved for reviewing the changes the agent makes. The Git and GitHub pages remain available from the navigation tabs while the new workspace takes shape.

### Review And Commit From The Sessions Workspace

The right panel of the Sessions workspace now shows your working-tree changes and refreshes automatically each time the agent finishes a turn, so you always see what the agent just touched. Write a commit message (or generate one with AI from your diff) and commit without leaving the workspace; unstaged changes are staged automatically, matching the Git page behavior.

### Images Open As Images In The Sessions Workspace

Opening a PNG, JPEG, or other image from the Files tab or the changes list now shows the picture instead of raw editor content, and unsupported binaries show a clear placeholder, matching the Git page. File headers across both pages share the same layout: file-type icon, file name, then the folder path trailing on the right.

### One Home For The Agent

The agent now lives only in the Sessions workspace; the Git page is a focused Git tool again (staging, history, stash, rebase, terminal) without its own agent sidebar. The bridges remain: send your local review comments to the agent (`cmd-shift-a`) or attach a diff selection (`cmd-shift-l`) from the Git page, and Reviu switches to the session with your content delivered, queued automatically if the agent is still connecting. Running the agent in two places at once caused conflicting conversation state; one home fixes that class of issues.

### Take Over From The Agent: Files Tab And Editing

A Files tab in the right panel shows your whole repository as a tree (modified files marked), and clicking a file opens it in the editor at the center, whether the agent touched it or not. Files are editable right there: type your changes and Save (or cmd-s); your manual edits land in the same changeset as the agent's work, the Changes panel picks them up immediately, and the agent sees them on its next turn. The agent codes first, but taking over is always one click away.

### GitHub In The Sessions Workspace

The Sessions workspace now surfaces your GitHub context without leaving it. The right panel gains a Pull request tab showing the pull request linked to your current branch (state, title, comment count) with one click to open it, or a Create pull request button when the branch has none, using the same dialog as the Git page. The sidebar gains a compact GitHub section with your notification inbox count and a shortcut to your pull requests.

### Checkpoints: Roll Back Any Agent Turn

Every prompt now snapshots your working tree (including untracked files) before the agent starts, shown as a discreet checkpoint line in the conversation. Roll back restores your files exactly as they were at that point (your branch, HEAD, and staged state are never touched) and trims the conversation back to the checkpoint, so a bad agent turn is never more than one click from undone. The rollback itself takes a safety snapshot first, so even a rollback can be recovered. Checkpoints live under hidden git refs, are pruned automatically, and work on both the Sessions workspace and the Git page.

### Review The Agent's Diff And Comment Back

Click a changed file in the Sessions workspace (or a file location in the agent's tool calls) to open its diff right where the conversation was; Escape brings the conversation back. Comment on diff lines like a pull request review, then send all your comments to the agent in one batch: they arrive as a structured prompt with file, lines, and side, and each comment tracks whether the agent addressed it or whether the code moved on (outdated). Sending the batch returns you to the conversation to watch the agent work.

### Lighter GitHub: Repositories, Profiles, And Commits Open On GitHub

Reviu no longer carries its own repository browser (Overview, Readme, Code, Issues), profile page, and commit page. Links to a repository, a person, an issue, or a commit now open on github.com, where they are always up to date. What stays in Reviu is the part you came for: the pull request, its diff, and the review loop with the agent.

### The GitHub Inbox Moves Into The Sessions Sidebar

The GitHub home page is gone. Your notifications now live directly in the Sessions sidebar: unread count, one click to open (pull requests open in Reviu, everything else on github.com), and a check button to mark a notification as done without leaving your session. The top navigation is down to two tabs, Sessions and Git, on `cmd-1` and `cmd-2`. Without Reviu Pro, GitHub links land on the billing page.

### A Pull Request Page Focused On The Diff

The pull request Overview is now a summary, not a second GitHub. It keeps what you act on (branches, labels, assignees and reviewers, checks, merge readiness, the AI brief) and hands the rest to an Open on GitHub button: the long description and the comment thread. The nested scrollbar that came with the embedded conversation is gone, and auto-merge moves back to GitHub. The Changes tab is untouched: inline diff comments, suggestions, and sending your review to the agent all work as before.

### One AI In The App, The Agent

Reviu no longer asks for an OpenAI or Anthropic API key, and the AI settings are gone: nothing to configure, no key to hand over. The two features that used it go with it, the AI pull request brief and the generate-commit-message button. Your agent already reads the repository and the diff in the session, so a summary or a commit message is one prompt away, with the model you already pay for.

### A Calmer Conversation And Composer

The agent conversation now reads in a centered column instead of stretching across the whole window, so long answers stay readable however wide you work. The composer is one box again: the message field and its controls share a single frame that lights up when you type in it. The agent's extra settings (reasoning effort, sandbox, approvals) collapse from a row of unlabeled dropdowns into one control that shows what is actually in effect, like `high · off`, and only stands out once you change something.

### Choose Where Reviu Opens

Settings gains a Home Page choice: Sessions, the default, or Git for those who come to Reviu for the Git client first. Reviu opens there on every launch.

### Git Commands Where You Work

The command palette in the Sessions workspace was navigation only, even though the workspace shows your changes and commits them. It now carries the commands that match what is on screen: commit, stage all, unstage all, push, pull and fetch, on the repository the session is working in. Each one reports what happened and refreshes the changes panel.

### Switch Repositories From The Sessions Workspace

The Sessions workspace was stuck on whichever repository it picked at startup. The repository line at the bottom of the sidebar is now a button that opens the repository switcher, and the command palette gains Switch, Open and Forget repository. A session belongs to a repository: switching swaps the conversation list, the changes panel and the branch, and Reviu refuses to switch while the agent is mid-turn.

### Push And Pull From The Sidebar

The sidebar now shows how many commits you are ahead of and behind your branch's remote, and each counter runs its command when clicked: the up arrow pushes, the down arrow pulls. The counters appear only when there is something to sync, and they refresh after every commit made from the workspace.

### A Terminal In The Sessions Workspace

The right panel gains a Terminal tab, running in the repository the session is working in, so a quick command no longer means leaving for the Git page. The shell starts the first time you open the tab and follows along when you switch repositories.

### Stage And Discard From The Sessions Workspace

The Changes panel listed your modified files but could do nothing with them. It now groups them into Staged and Changes like the Git page, and each row carries the same actions on hover: stage, unstage, and discard. Files partially staged appear in both groups, discarding asks before destroying work, and the list refreshes itself after every action. The list is keyboard-navigable too: move through it with the arrow keys and the highlighted file opens as you go.

### Inline Or Split, In The Sessions Workspace Too

The diff in the Sessions workspace was stuck on whatever the settings said. It now carries the same toggle as the Git page, in the diff header or with `cmd-/`, and the preference is shared between the two. Files with a single side, an untracked or deleted file, or a binary preview, stay inline: there is nothing to put on the other half. A file opened without any pending change gets no toggle at all.

### Read Markdown And SVG Files In The Sessions Workspace

Opening a `.md` or `.svg` in the Sessions workspace showed raw text. A Preview button in the diff header now swaps the pane for the rendered file, and Code brings the diff back. While the rendered file is up, the inline/split control steps aside: there is no diff on screen to switch. Previewing is a detour rather than a mode, so opening the next file shows its code again.

### Hide Whitespace Changes In The Sessions Workspace

The Sessions workspace applied the whitespace setting when opening a file but gave you no way to change your mind. The diff header now carries the same Whitespace button as the Git page, with `cmd-alt-/`, and the choice holds for the rest of the session instead of resetting on every file. A file with no pending change, or a file being previewed, has no diff on screen: the button steps aside there, like the inline/split control.

### Browse The History From The Sessions Workspace

The right panel gains a History tab: your commits, expandable to the files each one touched. Click a file to read it as it was in that commit, right where the diff shows up, read-only so a snapshot can never overwrite your work. Opening the same file from the Changes tab brings the working-tree version back. The history loads the first time you open the tab, not before.

### A Command Palette That Only Offers What Works

The Sessions workspace listed Commit, Stage all, Unstage all, Push and Pull whether or not they could run: committing with nothing staged and no message, pushing a branch with nothing ahead. The palette now follows the same rules as the Git page, so a command is there when it does something.

### Conflicts Stop You On The File, Not On An Error

Git commands that stop on conflicts (merge, rebase, skip, pull) now say which file is waiting and put it on screen, in the Git page as before and in the Sessions workspace too. Every git command in the app now shares one implementation, so the same command reports the same thing wherever you run it.

### Jump Between Changes In The Sessions Workspace

Reviewing a long file in the Sessions workspace meant scrolling to hunt for the next change. `cmd-alt-down` and `cmd-alt-up` now walk the diff change by change, as on the Git page and the pull request diff, and wrap around at the ends. While a Markdown or SVG file is previewed there is no diff to walk, so the shortcuts stay out of the way.

## 0.18.0

### No More Failed Single Comments During A Review

While you have a review in progress, GitHub only accepts comments added to that review, and posting a standalone comment fails. Reviu now matches github.com: the "Add single comment" button is hidden while a review is pending, and the keyboard shortcut adds the comment to your pending review instead of failing with an error.

### Refreshed Command Palette

The command palette has a cleaner, roomier look: a larger search field, rounded rows with subtler icons, and details like a stash's commit hash shown on the right of each row. A new footer strip lists the keyboard hints (navigate, run, close) so the palette is easier to learn, and it adapts to each screen, whether you are picking a branch or typing a stash message. The file search and GitHub repository search dialogs share the same refreshed look, so every palette in the app now feels consistent.

### Up-To-Date AI Model List From Your Provider

The model picker in AI settings now pulls the current model list straight from your provider (OpenAI or Anthropic) instead of a fixed built-in list, so new models show up without waiting for an app update. The list refreshes when you open settings, switch provider, or save a key, and a Refresh button pulls it on demand.

### Keyboard Shortcuts On The Terminal And Agent Buttons

The Terminal and Agent toggle buttons on the Git page now show their keyboard shortcuts, the same as the command palette and commit buttons, so the toggles are easier to learn.

### Open A Repository From The Repository Selector

The repository selector on the Git page now includes an "Open repository…" option, so you can pick a new folder to work on right from the dropdown. Opening a repository was previously only reachable through the `cmd-o` shortcut.

## 0.17.0

### Stack Review Comments Before Submitting

Review a pull request the way GitHub does: start a review and stack several comments as drafts instead of posting each one the moment you write it. Every inline comment now offers "Add single comment" (posts immediately, as before) or "Start a review" (holds it as a pending draft, marked "Pending"). Drafts stay private until you submit the review as Approve, Request changes, or Comment, and they sync with github.com, so a review you began in the browser shows up in Reviu and vice versa. You can edit, delete, and reply to drafts before submitting, and the submit dialog shows how many pending comments will be published.

### Consistent Interface Font On Linux And Windows

Reviu now ships with its interface font, so the app looks the same on every platform. Previously the interface only picked up the intended font on macOS and fell back to a typewriter-style monospace font on Linux and Windows; text throughout the app now renders in a clean proportional font, with code and diffs in the bundled monospace font.

### Long File Paths Keep The Diff Toolbar In View

When a file has a long name or deep path, diff and file headers now keep their controls (whitespace, split/inline, preview) pinned and reachable instead of pushing them off the edge. The folder path truncates from the start so the most specific part stays visible. File names in the change lists and headers across the Git, pull request, commit, and repository views also render at a consistent, more compact size.

## 0.16.0

### Generate Commit Messages With AI

A Generate button beside the commit message box writes a commit message from your changes. It reads your staged diff (or all uncommitted changes when nothing is staged), sends it to your configured AI provider, and fills the box with a Conventional Commits message you can edit before committing.

### Add Context With @ In The Agent Panel

Type `@` in the Agent panel to pull repository context into your message. Pick a file by name to reference it, or choose `@diff`, `@staged`, or `@branch` to attach your uncommitted changes, staged changes, or the diff against the base branch. The diff is sent to the agent with your message, so you can ask things like "review @diff" without copying anything by hand. Use arrow keys and Enter to pick, Escape to dismiss.

### Send A Diff Selection To The Agent

Select lines in a diff and press the Send Selection To Agent shortcut (`cmd-shift-l`, configurable in Settings) to drop an `@selection` reference into the Agent panel. Your selected code rides along with the next message as context, so you can ask the agent to explain or rework exactly what you highlighted.

### Open Agent File References

File locations the agent touches (the path shown on Read, Edit, and other tool calls) are now clickable. Click one to open that file in the diff view and jump straight to the referenced line, so you land on what the agent changed without hunting for it in the file list.

## 0.15.0

### Terminal On Git Page

Open a terminal next to your diff on the Git page. It launches in the selected repository so `git`, build commands, and scripts run with the right working directory without leaving Reviu. Use the Terminal button in the header or the keyboard shortcut from Settings. If the shell exits or fails to start, a banner shows what happened with a Restart button.

### Selectable Text In Agent Panel

Text in the Agent panel is now selectable. Drag to select, double-click for a word, triple-click for a line. Releasing the drag copies the selection to your clipboard.

### Calmer Editor Hover

Hunk actions and review comment overlays no longer flicker when the mouse leaves the diff or hovers over the Agent or Terminal panels.

### Compact Review Comments On Git Page

Review comments on the Git page now use a smaller card with a shorter input, leaving more room for the diff.

## 0.14.0

### Syntax Highlighting In Agent Tool Output

File contents shown by the agent (Read, Edit, Write) now display with syntax colors based on the file path. Diff hunks keep their added/removed background while tokens follow the language's color scheme, so code in the Agent sidebar reads like it does in the editor.

### Agent Sidebar Finds Node In Packaged Builds

The Agent sidebar now locates `npx` (used to launch Claude and Codex) when Reviu is started from the dock or Finder on macOS and from desktop launchers on Linux. Packaged builds no longer fall back to a minimal system PATH that missed Node installed via nvm or Homebrew, so the agents start without manual PATH tweaks.

### Accurate Contributors On GitHub Repo Page

The Contributors section on a GitHub repository overview now lists the people who actually committed to the repo, matching GitHub's own sidebar. Previously, repositories owned by an organization showed mentionable org members instead of contributors, so the avatar list and total count did not match GitHub.

## 0.13.0

### Agent Sidebar

The Git page now has an Agent sidebar that runs Claude Code or Codex directly inside Reviu, using your existing subscription. Pick the model, mode, reasoning effort, or thinking budget that the backend advertises. The execution plan and the agent's reasoning surface inline as the work progresses. Open with the Agent button or Cmd-Shift-J. Send local review comments straight to the agent with Cmd-Shift-A.

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
