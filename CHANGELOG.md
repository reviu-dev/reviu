# Changelog

All notable changes to Reviu are documented here.

## 1.0.0

### Faster, Smarter File Search

Cmd-P now ranks fuzzy matches by filename and path, keeps exact filenames above incidental folder matches, and favors changed and recently viewed files. Repository files load in the background and stay cached, matches are highlighted and capped for responsive navigation, loading failures stay visible, and queries such as `session_page.rs:1520:8` open directly at the requested position.

### Editor Scrollbar Markers

The editor scrollbar now shows compact markers for changed lines, search matches, conflicts, and review comments, making large diffs easier to scan without leaving the scroll track. The active hunk accent now appears only when walking changes explicitly and stays pinned while clicking, selecting, or moving the cursor elsewhere. Walking to a hunk now also shows its floating actions, selected hunks and staged hunks get clearer outlines, split diffs get side-aware gutter markers and keep review comments on the side where the drag started, and the toolbar counter and tooltips name the hunk or conflict it is walking.

### One Workspace

The separate Git page is gone: everything it did happens in the Sessions workspace, now the only place Reviu opens. The changes list with hunk staging, conflicts, the history, the file tree, the terminal, branches, stashes, cherry-pick, the interactive rebase (its todo now takes the whole centre) and every keyboard shortcut moved there, next to the agent and the diff. Old links and shortcuts land in the workspace.

### GitHub Without Its Pages

The pull request page, the repository browser, profiles and the GitHub home are gone too. The branch's pull request lives in the right dock, notifications live in the sidebar (one click to open, a check to mark done), and everything else opens on github.com, where it is always up to date: repositories, issues, profiles, commits, discussions.

### One AI In The App, The Agent

Reviu no longer asks for an OpenAI or Anthropic key: the AI settings, the pull request brief and the generate-commit-message button are gone. Your agent already reads the repository and the diff, so a summary or a commit message is one prompt away, with the model you already pay for.

### Any Coding Agent, From The ACP Registry

The agent picker is served by the official ACP registry: twenty-three agents out of the box, from Claude, Codex and Gemini to Copilot and Cline, each launched with the version the registry publishes, each with its own icon. A bundled copy and a disk cache keep the picker populated offline, and background refreshes bring new agents without an update to Reviu.

### A Conversation Built For Watching An Agent Work

The agent's narration appears where it happened, thinking streams as a dimmed live glimpse that folds when it ends, and tool steps read as one clean line, grouped under a summary that folds when the turn ends. Registry agents keep their protocol tool identity when they provide one, so reads, edits and commands stay consistent even when titles vary. Read results and edit diffs carry real file line numbers, and a read that only reports a local file location still shows a bounded snapshot when Reviu can safely read it. Text is selectable everywhere, and terminal output keeps the colors tools emit.

### The Turn Closes With A Receipt

A turn that touched files ends with a summary card: files edited, added and removed lines, per file. Each row opens its diff, Review jumps into the diff view, and Undo reverts the turn's changes while keeping the conversation. The rest of the turn folds behind the card: your question, the final answer and a "Worked for 2m 5s · 8 steps" row that unfolds on click.

### Checkpoints: Roll Back Any Turn

Every prompt snapshots your working tree, untracked files included, before the agent starts. Roll back restores your files exactly and trims the conversation to the checkpoint; your branch, HEAD and staged state are never touched, and the rollback itself takes a safety snapshot first. Checkpoints live under hidden git refs and are pruned automatically.

### Edit A Message And Replay From There

Your prompts gain edit and copy buttons on hover. The edit happens inside the bubble; sending it restores the files to the checkpoint taken before that prompt, drops the turns after it, and replays from your new wording in a fresh session.

### Queue The Next Message, Or Steer The Turn

Enter queues your message while a turn runs, editable until it goes out. Cmd+Enter sends it straight into the running turn; a refused injection returns to the queue, and an agent that cannot take mid-turn input simply queues. Stopping a turn holds the queue.

### Watch Commands Run

The agent's shell commands run in terminals Reviu owns and stream live into the conversation: the command, the tail of the output, the exit code, and a stop button while it runs, so a hung build never holds the turn hostage. Reviu asks Cargo, pnpm, Vitest and other color-aware tools to keep color on even though their output is captured, and command output still keeps its colors when an agent reports it as regular tool text.

### See What You Are Approving

Permission cards show the thing being approved: the full command, the URL, per-file counts for an edit. An Auto-approve toggle answers requests for you, always picking the allow option; cards answered this way say "Auto-approved", and a request with no allow option still waits.

### Show The Agent A Screenshot

Paste or drop an image and it stages as a thumbnail riding along with your next message; a regular file drops as an @ mention. Image attachments appear only when the agent accepts images.

### Slash Commands From Your Agent

Typing "/" at the start of the composer lists the commands your agent actually offers, from its built-ins to your project's own, filtered as you type. A path in the middle of a sentence stays a path.

### Know When The Agent Needs You

A small popup in the corner of your screen says when a turn finishes or a permission waits, only while the window is inactive, and clicking it brings you back. A switch in Settings turns it off.

### A Failed Turn Is Loud, Whatever The Agent Hides

A failed turn shows a red card naming what happened (credits exhausted, rate limited, provider unreachable) and marks the session's row Failed; a turn that ends with no reply at all is flagged the same way, so nothing fails silently. A dead agent process offers Reconnect right there.

### Fast From The First Word To The Last

Streaming renders incrementally, repaints on a steady beat, and saves off the main thread, so long sessions stay smooth and the CPU calm. Closed dock panels stop rendering their hidden contents, and settled streamed markdown replies keep their incremental parse state instead of starting over at the end of the turn. Settled chat messages reuse shared text when they repaint, keeping long transcripts lighter. Switching conversations never freezes the app, and the sidebar lists sessions from an index, instantly, with a one-line preview per row.

### The Conversation Keeps Your Place

Each conversation keeps its composer draft and its scroll position across switches and restarts. Sending pins your message to the top with the reply streaming below; when the reply grows past that anchored view, the jump-to-bottom pill lets you rejoin live scrolling. Cmd+Shift+J jumps to the latest message.

### Sessions From All Your Repositories, In One List

The repository is an attribute of each session, not a mode. The sidebar shows every repository as a foldable section, newest sessions first, and nothing reorders while agents work. Each row carries a live state dot (amber working, blue waiting on you, red failed), and worktree sessions show their branch.

Cmd+T starts a session in the repository you are looking at, Cmd+Shift+T starts one in its own worktree, and both live in the command palette too, named after the repository they will create in.

### Agents Keep Running In The Background

Switching sessions never kills a running agent: its reply keeps streaming into the transcript, and coming back shows the conversation as it progressed. Sessions sharing the main checkout take turns; recent idle sessions stay warm, older ones reconnect when you return.

### Sessions In Their Own Worktree, In Parallel

Each repository section in the sidebar carries two buttons: one starts a session in that repository, the other starts it in its own git worktree, on a fresh `reviu-` branch from the base you pick, so its agent runs in parallel with other worktrees and with you in the main checkout. The branch renames itself after the conversation's title; a branch you checked out or renamed yourself is never touched. Deleting the session removes worktree, branch and snapshots, and opening a repository sweeps unreferenced `reviu-` worktrees.

### The Whole Window Follows The Session's Checkout

Selecting a worktree session points everything at its checkout: changes, branch header, history, terminal and file search. Coming back points everything home, and an agent busy in its worktree never blocks branch switches in the main checkout.

### Peek At Another Worktree Without Leaving

A selector in the dock header lists the repository's checkouts: the main one and every worktree, named by branch and by the session working there. Pick one and the changes, the diff and the file search show that checkout while your conversation stays put. The dock wears its pin until you click back to following the session, switch sessions, or the worktree disappears.

### Review The Agent's Diff And Comment Back

Click a changed file, in the Changes list, the turn recap or a tool call, and its diff opens beside the conversation; Escape gives the conversation the width back. Comment on diff lines and send the batch to the agent as a structured prompt with file, lines and side.

### A Quiet Diff For A Single Change

When a file has only one hunk, a brand-new file included, the diff stops pointing at it: no focus border around the change, no arrows and no "1/1" counter in the header. The comment-this-hunk shortcut still targets it, and a conflicted file keeps its counter even at one conflict, since watching it fall to zero is the point.

### A Review Panel For The Batch

The Review tab lists your comments grouped by file: click one to land on its lines, tick a selection or send everything, and single comments carry their own send arrow. A batch survives restarts and repository switches; sent comments leave the panel when the turn ends, a stopped or failed turn keeps them, and a comment whose lines were rewritten stays, tagged Outdated, left out of the batch. Dots on the rail icons say when a review or the working tree is waiting.

### The Branch's Pull Request, In The Dock (Reviu Pro)

Checking out a branch with a pull request fills the tab: title, author, branches, and the files it proposes, each opening the pull request's own diff, base against head. Files carry comment icons, tinted while some are unsubmitted. The panel rereads GitHub as soon as the branch changes, whoever moved it, and the refresh button spins until every read has landed. A branch without a pull request gets Create pull request, or `Publish and create pull request` when it is not on the remote yet.

### Checks, Reviewers And Merge In One Block

A collapsible block leads with the bad news: a failing suite or a requested change shows without expanding anything. Its header carries per-state check counts, the open list scrolls past six checks, and hovering a reviewer shows the message they left. The block is always there, so a small project never loses its merge button for lack of a CI.

### Merge Like On GitHub

The merge button names its method and lists only the methods the repository allows, remembered per repository. Confirming opens a form prefilled with exactly what GitHub would generate for the commit title and message, down to the squash bullets and Co-authored-by trailers; clear a field to let GitHub write it. The button stays disabled until Reviu knows the pull request can merge.

### Review A Pull Request

The Review panel splits comments by destination: "To the agent" and "To this pull request", the latter read back from GitHub, so a review started in the browser is there when you come back. Write on a line of the pull request's diff and it joins your review, or goes out on its own; existing conversations show on their lines, with replies, resolving, editing and deleting. Submit asks for the decision and message, says how many comments go out, refuses an empty message where GitHub would, and your row in the reviewers block flips immediately. Discard deletes the pending review and its comments on GitHub after a confirmation. Not there yet: the preview tab, images in comments, applying suggestions, links out of comment bodies.

### A Link To A Review Comment Opens The File It Is About

A link to a comment of the open branch's pull request, pasted in the palette, clicked in the inbox or sent by the extension, opens its file in the diff on the line it was written against. Everything Reviu does not show opens on github.com.

### Review A Pull Request You Have Not Checked Out

The extension's Open in Reviu button names the branch carrying the pull request, asks before touching your repository, checks it out (fetching first if needed) and opens the panel. It never moves your branch mid-turn, and a working tree the checkout would overwrite stops it.

### A Command Palette That Finds What You Meant

Words match one by one against word starts: `stash untracked`, `push force` and `swbr` all land. Commands answer to the words you would use (`revert`, `wip`, `squash`), and results are ordered by how well they answer. Destinations go by the name the app gives them, panels lead with what they hold.

### The Palette Teaches Its Shortcuts

Everything with a shortcut is a command, showing its key as you rebound it; a command that cannot run says why instead.

### A Key For Every Surface, That Brings You Back

`cmd-shift-e` Changes, `cmd-shift-f` Files, `cmd-shift-r` Review, `cmd-shift-h` History, `cmd-shift-p` Pull request, `cmd-j` Terminal. The key means "take me there", and only closes the panel when the keyboard is already in that surface; Escape hands the keyboard back to the file you were reading. Tab reaches the dock like a form.

### Walking A List Is Not Choosing From It

Arrows show what a row holds and leave the keyboard in the list; Enter or a click opens the file and hands the editor the focus. Left and right fold, in every tree, and files load only when you stop on them.

### Settings And Keybindings In Plain Files

`settings.json` and `keybindings.json` replace the internal storage, and existing settings and shortcut overrides move over on first launch. The files are yours: hand-editable, dotfiles-friendly. An invalid value falls back to its default, an unknown entry is left untouched, and a file that no longer parses is reported and never overwritten.
