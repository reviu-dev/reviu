# Changelog

All notable changes to Reviu are documented here.

## 1.0.0

The first release. A coding agent works, you review what it did, with a real git client underneath - in one window.

### One Window For The Whole Loop

Your agent sessions live in the left sidebar, the conversation and the diff share the centre, and the repository sits in the right dock: Changes, Review, Files, History, Pull request and a Terminal, on a permanent icon rail that never truncates however narrow the panel gets. Panels slide in and out, their edges drag to resize with a double-click returning the default width, and an expand button gives the right dock the whole window for a large diff or a long terminal session. Focus always lands somewhere alive: closing a panel hands the keyboard to the diff or the composer, never to nothing.

What Reviu does not rebuild, it hands to the browser: repositories, issues, profiles, releases and discussions open on github.com, where they are always up to date. What stays in Reviu is the loop you came for: the agent, the diff, the review, the pull request.

### Any Coding Agent, From The ACP Registry

The agent picker is served by the official ACP registry: twenty-three agents out of the box - Gemini, Copilot, Qwen, Cline, Kilo, Grok and the rest alongside Claude, Codex and Pi - each launched with the version the registry publishes, each with its own icon. A copy ships with the app and another is cached on disk, so the picker is populated on first launch and stays populated offline; the list refreshes itself in the background, so new agents arrive without an update to Reviu. Pick the model, mode, reasoning effort or thinking budget the backend advertises, and the choice is remembered per agent. When an agent will not start, the message points at that agent's own page instead of a generic one.

### A Conversation Built For Watching An Agent Work

The agent's narration appears where it happened: an explanation before an edit stays before the edit, thoughts close before the prose they led to. While the agent reasons, a live glimpse of its thinking scrolls by, dimmed, and folds into a discreet expandable line when it ends. Tool steps read as one clean line with a small icon tile; only a failure brings color. Consecutive tool calls group under a single summary like "Ran 2 commands · Edited 1 file", open while the agent works, folded when the turn ends. Your messages read as tinted bubbles on the right, images you attach show as real thumbnails, and code blocks in replies are syntax highlighted with a language label and a copy button.

Read results and edit diffs inside tool cards carry real file line numbers, their text is selectable across lines in one drag, and everything the agent shows of a file is colored by the same fifty-language engine as the editor.

### The Turn Closes With A Receipt

When a turn that touched files finishes, the conversation closes it with a summary card: how many files were edited, added and removed line counts overall and per file. Each file row opens its diff, Review jumps straight into the diff view, and Undo reverts the turn's file changes while keeping the conversation - the card simply flips to "Undone". Undo is only offered on the latest turn, so it can never clobber work from the turns that came after.

The rest of the turn tucks itself away: the thinking, the tool steps and the in-between narration fold behind the card, leaving your question, the agent's final answer and a "Worked for 2m 5s · 8 steps" row. Click it to unfold the full timeline, click again to tidy it back up.

### Checkpoints: Roll Back Any Turn

Every prompt snapshots your working tree, untracked files included, before the agent starts, shown as a discreet checkpoint line in the conversation. Roll back restores your files exactly as they were at that point - your branch, HEAD and staged state are never touched - and trims the conversation back to the checkpoint, so a bad turn is never more than one click from undone. The rollback itself takes a safety snapshot first, so even a rollback can be recovered. Checkpoints live under hidden git refs and are pruned automatically.

### Edit A Message And Replay From There

Hovering a message reveals a copy button, and your own prompts gain an edit button. The edit happens inside the bubble itself - same shape, same place, Enter sends, Shift+Enter adds a line, Escape cancels. Sending the edit restores the files to the checkpoint taken before that prompt, drops the turns after it, and replays the conversation from your new wording in a fresh session.

### Queue The Next Message, Or Steer The Turn

You never wait for a turn to finish before writing the next instruction. Enter queues the message in a card above the composer, ready to edit or remove; when the turn ends it is sent as the next one. Cmd+Enter (Ctrl+Enter on Linux/Windows) sends your message straight into the running turn instead: "actually, skip the tests" lands immediately. If the agent refuses the injection, the message safely returns to the queue and the turn keeps running. With an agent that cannot take mid-turn input, Cmd+Enter simply queues, instead of trying and coming back with a refusal. Stopping a turn holds the queue: nothing runs until you decide.

### Watch Commands Run

The agent's shell commands run in terminals Reviu owns, and their output streams live into the conversation: the command, the tail of its output as it arrives, and the exit code when it finishes, in red when it failed. A running command carries a stop button, so a hung build never holds the turn hostage. Output shows the colors the tools emit - cargo's greens and reds, test runner highlights - adapted to light and dark themes, and Reviu asks common CLI tools (Cargo, pnpm, npm, Vitest, Jest and other color-aware commands) to keep color on even though their output is captured. Everything is selectable and copies clean.

### See What You Are Approving

Permission requests show the thing being approved, not just a title: the full command for a shell run, the URL for a fetch, per-file added and removed counts for an edit. Long commands scroll in place and can be copied, answered cards show the button you pressed, and cards survive a reload.

An Auto-approve toggle in the composer answers the agent's permission requests for you, always picking the allow option, so long unattended runs stop parking on a question. Cards answered this way say "Auto-approved", a request with no allow option still waits for you, and the choice sticks with the conversation.

### Show The Agent A Screenshot

Paste an image into the composer or drop one onto it and it stages as a thumbnail, removable with one click, then rides along with your next message. Dropping a regular file inserts it as an @ mention instead. Image attachments appear only when the connected agent actually accepts images.

### Slash Commands And @ Context

Typing "/" at the start of the composer opens the commands your agent actually offers, from its built-ins to your project's own, filtered as you type. Type "@" to reference a file by name; the list refreshes after every turn, so files the agent just created are mentionable. And selecting lines in the diff and pressing `cmd-shift-l` attaches exactly what you highlighted to your next message.

### Know When The Agent Needs You

Working in another app while the agent runs, Reviu shows a small popup in the corner of your screen when a turn finishes or when the agent waits on a permission, and clicking it brings you straight back. It only appears while the window is inactive, never over your work in Reviu itself. A switch in Settings turns it off.

### A Failed Turn Is Loud, Whatever The Agent Hides

A failed turn shows a red error card in the conversation naming what happened (usage limit or credits exhausted, rate limited, provider unreachable), and marks the session's row as Failed until the next attempt. A turn that ends without any reply at all is flagged the same way, so an agent that swallows its provider's refusal cannot fail silently; an error the agent already printed itself is not shown twice. When the agent process dies, the error offers a Reconnect button right there, and a stopped turn leaves a "Stopped" marker instead of ending silently.

### Fast From The First Word To The Last

Each streamed chunk extends the rendered reply incrementally instead of re-reading the whole message, the screen repaints at a steady beat while the agent streams, and transcripts save on a short throttle off the main thread - so a busy session stays smooth and the CPU stays calm from start to finish. Switching conversations never freezes the app: the current one stays on screen, the target row shows a small spinner, and the switch lands as soon as the transcript is ready. The sidebar lists conversations from a small index, so repositories with a long history open their session list instantly, each row with a one-line preview of the last message.

### The Conversation Keeps Your Place

A half-typed message stays where you left it: each conversation keeps its own composer draft, restored after switching sessions or restarting the app. Switching back to a conversation returns you to the exact spot you were reading; one left at the bottom keeps following new messages. Sending a message pins it to the top of the conversation with the reply streaming below it, so nothing scrolls under your eyes, and the jump-to-bottom pill sticks: one click and the conversation follows the streaming reply until you scroll up to read. Cmd+Shift+J jumps to the latest message from the keyboard.

### Sessions From All Your Repositories, In One List

The repository is an attribute of each session, not a mode of the app. The sidebar shows every recent repository's sessions in one list, each repository a foldable section, newest sessions first inside it - and nothing reorders itself while agents work. Folding a section is the filter; a folded section shows how many sessions it holds. New Session lands in the repository of the session you are looking at, and every section header carries its own compose button for an explicit target.

Each row carries its live state as a small colored dot next to the time: amber working, blue waiting on you, red failed, with the word in its tooltip. Sessions in a worktree show their branch under the row, so you can tell at a glance where each agent is working.

### Agents Keep Running In The Background

Clicking another session never kills the running agent: each session keeps its own agent process alive while you look at another one, its reply keeps streaming into the transcript, and coming back shows the conversation exactly as it progressed. Sessions sharing the main checkout take turns, since two agents editing the same files would trample each other - sending a message while another one is working tells you so. The most recent idle sessions stay warm; older ones let their process go and reconnect when you return.

### Sessions In Their Own Worktree, In Parallel

The sidebar's second button starts a session in its own git worktree, created next to your repository on a fresh `reviu-` branch - from your default branch or any local branch you pick. Its agent reads and writes there, so it runs at the same time as agents in other worktrees, and at the same time as you working in the main checkout. Checkpoints, rollback and undo follow each session into its worktree.

The generated branch renames itself after the conversation's title once it has one - `reviu-fix-the-scroll-jump` instead of `reviu-swift-otter` - and once renamed it never renames again. A branch you checked out or renamed yourself is yours and never touched. Deleting a worktree session removes its worktree, its branch and its snapshots; opening a repository sweeps away `reviu-` worktrees no session references any more, so nothing accumulates behind your back.

### The Whole Window Follows The Session's Checkout

Selecting a worktree session points everything at its checkout: the changes panel shows the agent's edits there, the branch header names its branch, the history, the terminal and file search all read the same tree the agent writes. Coming back to a main-checkout session points everything home again. An agent busy in its own worktree never blocks switching branches in the main checkout.

### Review The Agent's Diff And Comment Back

Click a changed file - in the Changes list, in the agent's turn recap, or a file location on its tool calls - and its diff opens beside the conversation, in a resizable split; Escape closes it and gives the conversation the full width back. Comment on diff lines like a pull request review, then send the batch to the agent: comments arrive as a structured prompt with file, lines and side, and Reviu returns you to the conversation to watch the work. The Changes list always highlights the file on screen, wherever the open came from.

### A Review Panel For The Batch

The Review tab lists the comments you are building, grouped by file, each file collapsible. Click a comment to land on its lines, delete one you changed your mind about, and send from the panel - or send only part of it: tick the comments you want, or a whole file in one click, and Send carries only those. Single comments carry their own send arrow for putting the agent back on one thing without resending the rest. Whatever stays behind stays a draft and goes with the next send. When the panel is closed, a dot on its rail icon says a review is waiting; the Changes icon wears the same dot when the working tree has something in it.

A review you have not sent survives everything: close the app, switch repositories, come back a week later - each repository keeps its own batch, written the moment anything changes. Comments last exactly as long as they need to: you write them, you send them, and when the agent finishes that turn they leave the panel. A turn you stop, or one that fails, changes nothing: they are still there, ready to go again. When the agent rewrites the lines a comment was anchored to before you send it, the comment stays, tagged Outdated, and is left out of the batch.

### Git, The Whole Client

The Changes tab groups your files into Staged and Changes, refreshed after every agent turn. Each row stages, unstages or discards on hover; hovering a change in the diff brings up Stage, Unstage and Restore for that hunk alone, with `shift-enter` / `shift-backspace` from the keyboard and `cmd-enter` / `cmd-backspace` for the whole file. Write a commit message and commit without leaving the workspace; the commit button's menu carries Amend, Undo last commit, Push and Force push (with lease), greyed out when they cannot run. Staging is silent - the panel already shows the files move - and only a failure speaks.

The command palette carries the rest, each command offered only when it does something: switch, create and delete a branch, merge or rebase onto one, cherry-pick, stash with or without untracked files, then apply, pop or drop, checkout a commit or tag, push, pull and fetch. Switching branch or repository is refused while an agent is mid-turn in that checkout: the ground cannot move under a running turn. The sidebar shows ahead/behind counters that run their command when clicked.

### Rewrite History

Interactive rebase takes the whole centre, where a table of commits belongs: pick it from the palette on a branch or on the last N commits, reorder, squash, fixup or drop, then apply. A range containing merge commits says how many merges will be dropped and continues like `git rebase -i` does. It is offered only with a clean working tree, and Force push sits next to Push for the branch you just rewrote.

### Conflicts Stop You On The File, Not On An Error

Git commands that stop on conflicts - merge, rebase, skip, pull - say which file is waiting and put it on screen. A conflicted file carries Accept Current, Accept Incoming and Accept Both per conflict block (`cmd-shift-enter` for both sides), Accept All in the header, and `cmd-alt-up` / `cmd-alt-down` walk conflict by conflict with a counter. Once you resolve the markers by hand there is no diff left to read, so the file shows in full until the resolution is staged. The right dock says which operation is running, Commit becomes Continue rebase once conflicts are resolved and staged, the message git prepared lands in the box on its own, and Abort is in the palette while there is something to abort.

### The History, The Files, The Terminal

A History tab lists your commits, expandable to the files each one touched; click a file to read it as it was in that commit, read-only, so a snapshot can never overwrite your work. A Files tab shows the repository as a tree, opened on the folders holding uncommitted work, modified files marked - and files are editable right there: type and save (`cmd-s`), your manual edits land in the same changeset as the agent's work. A Terminal tab runs in the repository the session is working in; Tab stays Tab there, Shift-Tab included, as a shell must.

### A Diff Editor Built For Review

Inline or split, toggled with `cmd-/` and remembered; files with a single side stay inline, and a clean file never splits. Hide whitespace-only changes with `cmd-alt-/`. Diffs use the histogram algorithm with precise word-level highlights, syntax highlighting covers fifty languages, `cmd-alt-down` / `cmd-alt-up` walk the file change by change and wrap at the ends, and `cmd-f` searches the open file from anywhere in the workspace. A renamed file names both sides, old name struck through; a selection lands in your clipboard on release.

### Markdown, SVG And Images, Rendered

A Preview button in the diff header swaps the pane for the rendered Markdown or SVG file, and Code brings the diff back - a detour, not a mode, so the next file opens on its code. PNG, JPEG and the other image formats open as pictures wherever they show up, and unsupported binaries show a clear placeholder instead of unreadable content. Markdown everywhere - replies, descriptions, comments, previews - renders GitHub-flavored, checklists included.

### Push And Pull With Your Saved Credentials

Pushing and fetching over HTTPS use the credentials already saved in your Git credential helper, such as the macOS keychain. If `git push` works in your terminal, the same remote works from Reviu. Only git repositories get opened: picking a folder inside a repository selects that repository, and a folder that stopped being one drops out of the recent list.

### The Branch's Pull Request, In The Dock (Reviu Pro)

Checking out a branch that has a pull request fills the Pull request tab with the whole picture: title and number, who opened it, the branches it goes between, and the list of files it proposes. Clicking a file opens the diff of the pull request itself, base against head, so you read what the branch adds rather than what your working tree happens to hold. Each file carries a comment icon and count when comments hang on it - tinted when some are still unsubmitted, muted when everything is published. The panel rereads GitHub as soon as the branch under it changes, whoever moved it - from Reviu or from a terminal - and its refresh button spins until every read it triggered has landed.

For a branch with no pull request, the tab offers Create pull request - and when the branch is not on the remote yet, `Publish and create pull request` pushes it first and only opens the form once the push succeeded.

### Checks, Reviewers And Merge In One Block

A collapsible block holds what you can do to the pull request, and leads with the bad news: a failing suite or a requested change is what you see first, without expanding anything. Its header carries compact per-state check counts - failing first, then pending, skipped, successful - and the open list shows about six checks and scrolls for the rest, so a CI with thirty jobs never pushes the file list out of the panel. Hovering a reviewer shows the message they left with their decision. The block is always there, even for a pull request with no CI and no reviewers, so a small project never loses its merge button for lack of a test suite.

### Merge Like On GitHub

The merge button names the method it will apply and carries a chevron listing the other methods the repository allows - and only those: a method disabled in the repository settings is not offered at all. Your last choice is remembered per repository. Confirming opens a small form prefilled with exactly what GitHub would generate for the commit title and message, following the repository's merge-message settings, down to the squash bullet list and its Co-authored-by trailers. Edit them, or clear a field to let GitHub write it; rebase shows no fields, since it keeps the commits as they are. The button stays disabled until Reviu knows the pull request can merge, not merely when it knows it cannot.

### Review A Pull Request Without Leaving

The Review panel says where each comment is going: comments on your working tree sit under "To the agent", comments belonging to a pull request review you have not submitted sit under "To this pull request", read back from GitHub, so a review you started in the browser is there when you come back. Commenting happens on the diff itself: open a file of the pull request, write on a line, and the comment joins the review you are building - or goes out on its own if that is what you pick, as on GitHub. Existing conversations show on their lines, with replies, resolving and unresolving, and editing or deleting your own words; a resolve button shows only when you can actually resolve.

Submit review asks for the decision - comment, approve, request changes - and its message, says how many comments go out with it, and refuses an empty message where GitHub would. Approving a pull request you have nothing to say about works from a Review button next to Merge; Reviu will not let you approve your own. Your row in the reviewers block flips to your decision the moment you submit. And a review started by mistake has a way out: Discard deletes the pending review and its comments on GitHub after a confirmation that says exactly that.

Not there yet on those cards: the composer's preview tab, dropping images into a comment, applying a suggested change as a commit, and following a link out of a comment body.

### A Link To A Review Comment Opens The File It Is About

When a comment belongs to the pull request of the open branch, pasting its GitHub link in the palette, clicking its notification in the inbox, or pressing the extension button opens its file in the diff, on the line it was written against. Everything Reviu does not show - issues, releases, discussions, pull requests of other branches - opens on github.com, and Reviu says which repository it has open rather than leaving you to guess.

### Review A Pull Request You Have Not Checked Out

The Open in Reviu button of the browser extension does the work when the pull request is not the branch you have open: Reviu names the branch that carries it, asks before touching your repository, checks it out - fetching it first when only the remote has it - and opens the Pull request panel once git has moved. It asks first because moving your branch is your call, it will not do it while the agent is mid-turn, and a working tree holding changes the checkout would overwrite stops it, as any branch switch does. Extensions exist for Chrome and Firefox.

### The GitHub Inbox In The Sidebar

Your notifications live in the sessions sidebar: unread count, one click to open - pull requests in Reviu, everything else on github.com - and a check button to mark one as done without leaving your session. The unread count also shows in the macOS menu bar and the Windows and Linux system tray.

### A Command Palette That Finds What You Meant

Words are matched one by one, against the start of a word rather than its middle: `stash untracked`, `push force` and `branch delete` all land, `tag` does not answer with every command that mentions a s-**tag**-e, and abbreviations work - `swbr` for "Switch branch". Commands answer to the words you would actually use, not only the ones we wrote: `revert` reaches undoing a commit, `wip` reaches the stash, `squash` reaches the interactive rebase. Results are ordered by how well they answer; an empty palette shows sections and your recent commands. Destinations go by the name the app gives them, panels lead with what they hold, and verbs stay only where something changes.

### The Palette Teaches Its Shortcuts

Everything with a keyboard shortcut is a command, each showing its key on the right - the key that actually works, so a shortcut you rebound appears as you rebound it. A command that cannot run says why instead. Signing in, signing out and installing the browser extension are in there too.

### A Key For Every Surface, That Brings You Back

Every surface of the right dock has a key: `cmd-shift-e` Changes, `cmd-shift-f` Files, `cmd-shift-r` Review, `cmd-shift-h` History, `cmd-shift-p` Pull request, `cmd-j` Terminal. The key means "take me there" - it opens the panel and hands the keyboard over - and only means "get out of the way" when the keyboard is already in that surface. Escape completes the loop: from any list of the dock it hands the keyboard back to the file you were reading, without closing anything. Tab reaches the dock like a form: file list, commit message box, commit button, in the order you use them.

### Walking A List Is Not Choosing From It

Arrows show what a row holds and leave the keyboard where it is, so you can walk a whole review or a whole history without touching the mouse - and without loading every file on the way, since a row has to be the one you stopped on before its file is read. Enter, or a click, opens the file and gives the editor the focus, which is the moment you actually wanted to be there. Left and right fold and unfold a folder or a file, the same keys in every tree. Every shortcut is remappable in `keybindings.json`.

### Reviu Pro, A Dialog That Says What It Sells

Reviu Pro is a dialog, offered from the app menu, the user menu and the palette whoever you are, and Escape puts you back where you were. It names what Pro brings before asking for anything - pull requests for your branch in the dock, reviewing and submitting without leaving Reviu, GitHub notifications in the inbox - followed by the length of the free trial. Signed out, the same promise comes with the GitHub sign-in instead of the prices; subscribers see their plan, status and renewal date, with nothing to sell them. Coming back from the browser after paying lands you on your work with the confirmation over it.

The surfaces that need Pro say so themselves, where the gap is felt: the Pull request tab and the inbox carry the offer in place of an empty surface, each naming what it would itself show. The Pull request icon stays in the rail whatever the repository is: a rail whose icons come and go is a rail you cannot learn.

### Settings And Keybindings In Plain Files

Reviu keeps its settings in `settings.json` and your shortcut overrides in `keybindings.json`, in its configuration folder. The files are yours: read them, edit them by hand, keep them in your dotfiles. A hand-edited file never breaks the app - an unknown or invalid value falls back to its default, an entry Reviu does not recognize is left untouched, and a file that no longer parses is reported at startup and never overwritten. Dark and light themes with auto-switching, and a font size setting that scales the whole interface, live in Settings.

### macOS, Linux And Windows, Updating Themselves

Reviu runs on all three, checks for updates on launch, and installs them in-app after verifying their SHA-256 checksum. The About dialog opens over whatever you are doing, and when a new version is waiting it offers the download itself. A feedback dialog sends bug reports and feature requests without leaving the app, and a recovered crash offers to send a report on the next launch.

### Errors Reach The Log File

Failures that have no surface to speak from go to a log file, with the file and line that reported them. Logging is on by default in dev builds, and `REVIU_LOG=1` turns it on for a release build. Everything else that goes wrong raises a toast: toasts are kept for what leaves no trace on screen - pushes and pulls, history rewrites, destructive acts, and anything that failed.
