# Changelog

All notable changes to Reviu are documented here.

## 1.5.0

### Git Credentials In The App

When Git or SSH needs a password, token, or key passphrase during a user-started fetch, pull, or push, Reviu now asks in the app instead of failing because there is no visible terminal prompt.

### Agents Run Commands In Parallel

While an agent waits for a long command such as a dev server, it can now start and read other commands, keep streaming its reply, and stop the long command itself. Waiting on a permission prompt no longer freezes the rest of the session either.

### Smoother Busy Terminals

Terminals in background tabs no longer slow Reviu down while their commands print a lot of output, and moving the mouse stays smooth while a visible terminal is flooded with output.

### Terminal Startup And Locale

Opening or restoring terminals no longer blocks Reviu while the shell starts, and anything typed during that moment still reaches the shell. When Reviu is opened from the Finder or the Dock, terminals now get a UTF-8 locale, so accents and non-ASCII paths display correctly.

### Running Commands In Terminal Tabs

Terminal tabs now show the command running in them, such as `cargo test` or `npm run dev`, and fall back to the folder once the shell is back at its prompt. Closing a terminal that is still running a command asks first, so a dev server or a long build is never stopped by accident.

### Cleaner Command Output For Agents

Agents now read their commands' output as plain text, without the color codes Reviu adds for the chat, and with progress lines already settled, so they spend fewer tokens and read errors more reliably.

### Agent Commands Run In A Real Terminal

Commands an agent runs now see a real terminal, so tools keep their own colors and progress output in the chat cards without Reviu forcing them, and wrapped long lines come back whole. Stopping an agent's command now also stops everything it started, such as the processes behind a dev server.

## 1.4.0

### Terminals In Split Panes

Open a terminal directly beside or below your current chat, file, or diff from the command palette or the new configurable shortcuts, without creating a tab and dragging it into place.

### Keyboard Split Pane Navigation

Focus the pane beside, above, or below your current split from the command palette or the new configurable directional shortcuts.

### Split Pane Launcher

Create an empty split pane from the command palette or keyboard, then choose whether to fill it with a terminal, chat, file, diff, or project search.

### Open Files And Diffs In Splits

Open File in Split and Open Diff in Split commands now create the target pane first, then let you choose the file or changed file to place there.

### Main Checkout Sidebar Cleanup

The project sidebar no longer shows a duplicate HEAD checkout when an old chat binding points at the main checkout instead of a real worktree. Detached worktrees now use their checkout folder name instead of the generic HEAD label.

### Agent Sign-In In Reviu

When Claude or another ACP agent asks you to sign in again, Reviu now offers the login command directly in the chat and can open it in a Reviu terminal. Claude logins reconnect the chat automatically once the terminal reports success.

### Switch Between A File And Its Diff

Open files now share one tab for code and diff. Press `cmd-shift-d`, use the File / Diff button, or choose Open file from a diff to switch views without losing your edits, undo history, or position.

### Git Changes In The File Gutter

Code files now show added, modified, and deleted-line markers beside line numbers and in the scrollbar. Jump between changes with Next Change and Previous Change, or open a marker to review, stage, or restore that hunk inline. Turn it off in Settings > Editor with Git Changes in Gutter.

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
