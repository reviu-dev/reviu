# Changelog

All notable changes to Reviu are documented here.

## 1.0.1

### Editor Find Options

File search now uses smart case when case sensitivity is off and remembers the case, whole-word and regex toggles for future editors.

### Editor Find Highlights

File search now highlights every match in the editor and uses a stronger active-match color, so Command-F behaves more like a code editor search instead of only selecting the current result.

### Project Search Shortcut

Command-Shift-F now opens project-wide text search with file, line and preview results, while the dock panel shortcuts move to clearer defaults for Changes and Files.

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
