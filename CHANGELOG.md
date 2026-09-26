# Changelog

All notable changes to Reviu are documented here.

## 1.4.0

### Switch Between A File And Its Diff

An open file now has a single tab that shows either its code or its changes. Press `cmd-shift-d`, or use the File / Diff button above the editor, to flip between the two without losing unsaved edits, undo history or your place in the file. Opening the file from Files shows its code, and opening it from Changes shows its diff, in the same tab. From the diff of a commit, a pull request or an agent edit, the same shortcut or the Open file button takes you to the file in your working tree at the line you were reading. A diff tab whose file has nothing left to show, once committed or restored, opens as the file the next time you select it or drag it into a split, unless it holds unsaved edits.

### Git Changes In The File Gutter

A file open as code now marks what changed since the last commit next to its line numbers, using the same colors as the diff: added, modified, and a small mark where lines were removed. Staged changes are outlined instead of filled, so you can tell them apart while you edit. The scrollbar shows the same marks across the whole file, and Next Change and Previous Change (Alt-F5 and Alt-Shift-F5) jump between them without leaving the code. Click a mark, or press `cmd-'` with the cursor in a change, to show the lines it removed right above it, with the stage and restore actions of the diff; do it again to fold them away. A staged hunk stays open so you can see the result. Turn it off in Settings > Editor with Git Changes in Gutter.

## 1.3.0

### Worktrees As Places To Work

A worktree is now a place in the sidebar that can hold several chats: a new chat opens in the checkout you are on, and chat history shows only that checkout's conversations. Deleting a chat never deletes its worktree. To remove a worktree, right-click it in the sidebar; Reviu asks first and shows how many uncommitted files and commits on no other branch would be lost.

### Archive Worktrees

Right-click a worktree in the sidebar and choose Archive to put it away without losing anything: its uncommitted and staged changes, its branch and its chats are kept, and the folder leaves your disk. Archived worktrees stay listed under their project; click one to restore it exactly as it was, or delete it permanently from its menu.

### Fresher Pull Counters

Reviu now refreshes remote tracking information in the background and lets the pull counter check for updates even when it currently shows zero, so incoming commits appear without changing your working tree.

### Drag Files Into Splits

Drag files from the Files panel into a center edge to open code beside your current work, or drag changed files from Changes to open their diff in a split.

### Outdated Pull Request Comment Context

Outdated GitHub review comments now show the original diff hunk inside the card, so the code context stays readable even when the anchor line moved in the current file.

### Closing The Window On Linux

Closing Reviu with the window's close button on Linux (X11) no longer crashes the app on its way out.

### Finished Agent Attention

When an agent finishes away from the visible chat, Reviu now keeps a green attention dot on the chat tab, history, and checkout row until you open the conversation.

### Session Usage In Composer

Chat context and cost usage now appear with the composer controls, so the current session usage stays visible without opening a split chat header.

## 1.2.0

### Reorder Your Workspace Tabs

Drag tabs to rearrange files, conversations, terminals and split groups without losing your place. Move Tab Left and Move Tab Right also work from the keyboard with Ctrl-Shift-PageUp and Ctrl-Shift-PageDown. Tab order and split proportions are restored per checkout, and closing a group now checks every unsaved file before removing anything.

### Soft Wrap Preference

Choose whether long lines wrap by default in Settings > Editor, without changing file contents. Use Alt-Z or Toggle Soft Wrap in the command palette for a temporary override in the active editor. The editor header no longer needs a Wrap button.

### Live Configuration Files

Find your settings and keyboard shortcut files from Settings or the command palette. Manual edits now apply without restarting Reviu, including symlinked dotfiles. Invalid JSON keeps your last working configuration and shows an error that clears when the file is corrected.

### Untitled Draft Recovery

Untitled tabs now keep their contents, selection and split placement across restarts, with automatic local backups for crash recovery. Quit without naming your drafts and pick up where you left off; saving or explicitly discarding a tab removes its backup without creating placeholder files in your project.

### Dock Shortcut Hints

Hovering a dock tab now shows its keyboard shortcut alongside its name, including any shortcut you have customized in Settings.

### Compact Permission Actions

Permission prompts now keep approval buttons short and stable, with long agent-provided labels available as tooltips so the primary action no longer shifts around.

### Softer Ignored Files

The Files panel now keeps visible gitignored files visually quieter, so generated folders stay available without competing with regular project files.

### Review Feedback Stays In Split View

Sending review comments to an agent now keeps the diff beside its conversation when both are open in a split. Focus returns to the existing chat without opening a separate conversation tab.

## 1.1.0

### Editor Find and Replace

File search now behaves more like a code editor: every match is highlighted, the active result has a stronger color, smart case works when case sensitivity is off, and case, whole-word and regex options are remembered for future editors. The find field also remembers submitted queries, and replace controls can update the current match or all matches with replace-all grouped into one undo step.

### Project Search Shortcut

Cmd-Shift-F now opens project-wide text search as a center tab with grouped, collapsible file results, line and preview matches, and quick keyboard access. Dock panel shortcuts move to clearer defaults for Changes and Files.

### Split Tab Dragging

Dragging a tab that already represents a split layout into another center pane now keeps every pane from the dragged layout and gives the resulting panes an even initial size, so chat, terminal and file combinations move together instead of dropping only the focused pane.

### New Chat In Split Panes

Split chat panes now include a new chat button that replaces only that conversation pane, keeping adjacent terminals, files or diffs in place for the next task.

### Terminal File Drops

Dropping files or folders onto the integrated terminal now inserts their shell-escaped paths without running the command, so paths can be used as arguments just like in native terminals.

### Search Agents During Setup

The onboarding agent picker now includes a search field so you can quickly filter the registry by name, description or id before choosing which agents Reviu should show.

## 1.0.0

### Reviu, Rebuilt Around The Agent

Reviu 1.0 turns the Git client with an agent on the side into one workspace for the entire agent review loop. Projects and running sessions live on the left, editor-style tabs and split panes for conversations, files, diffs and terminals fill the center, and the repository dock keeps Changes, Files, History, Review and Pull Request context on the right. The workspace restores its layout for each checkout, so the context for a task is still there when you return.

[Read the story behind Reviu 1.0](https://reviu.dev/blog/reviu-1-0).

### Run Agents In Durable, Parallel Sessions

Reviu now launches agents from the official ACP registry, including Claude Code, Codex, Gemini, Copilot, Cline and many more, using the CLI and subscription already installed on your machine. Sessions keep running when you switch away, carry their live status in the sidebar and can work in isolated Git worktrees so several tasks move at once without sharing a dirty checkout.

Every prompt creates a checkpoint of the working tree. You can undo a turn, roll the files and conversation back to an earlier prompt, or edit a message and replay from there without moving HEAD or losing the staged state.

### Review Code, Not Just A Conversation

The conversation shows streaming replies, thinking, tool activity, commands and permission requests as the agent works. Completed turns leave a receipt of the files and lines changed, with direct actions to open the diff, review the result or undo the turn. Messages can be queued for later or sent into a running turn, and images and file references can travel with the prompt.

Open any changed file beside the conversation, comment on exact diff lines and send selected comments back to the agent as a structured review. Reviu keeps the review batch across sessions and marks comments outdated when their lines change, so feedback stays attached to the code it describes.

### A Real Editor, Terminal And Git Workflow

Files, diffs, conversations and terminals now share the same center tab model. Tabs can be reordered, restored and dragged to an edge to create resizable split panes. The project explorer supports search, Git status decorations and everyday file operations, while the editor includes inline and split diffs, previews, hunk actions, conflict controls, search and scrollbar markers. The integrated terminal keeps its scrollback, links, search and working directory.

The local Git workflow remains built in: stage by file or hunk, commit and amend, branch, stash, cherry-pick, merge, rebase interactively, resolve conflicts, inspect history, fetch, pull and push. The command palette and keyboard shortcuts reach the same actions from anywhere in the workspace.

### GitHub, Focused On The Branch You Are Shipping

Reviu Pro puts the current branch's pull request in the dock with its files, review threads, checks, reviewers, merge readiness and allowed merge methods. The Review panel keeps local feedback for the agent separate from comments going to GitHub, while the sidebar inbox brings in notifications and opens the relevant pull request or github.com destination. The browser extension can hand a pull request to Reviu and check out its branch locally after confirmation.

The separate Git page, GitHub Home, repository browser, profiles, commit pages and pull request pages have been removed. Their local Git actions now live in the workspace, the active pull request stays in the dock and broader GitHub content opens on github.com. Reviu's former BYOK AI brief and generated commit message features are also gone: the coding agent you already use can produce either without another model configuration.
