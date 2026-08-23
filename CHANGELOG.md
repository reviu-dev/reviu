# Changelog

All notable changes to Reviu are documented here.

## 1.0.0

### Errors Reach The Log File Instead Of Nowhere

When saving a conversation, a draft, or a review batch failed, nothing said so. The app wrote its diagnostics to a stream that only exists when Reviu is started from a terminal, so a failed write was invisible in a normally launched app.

Those failures now go to a log file, with the file and line that reported them. Logging is on by default in dev builds, and `REVIU_LOG=1` turns it on for a release build.

### Discarding A Review Only Takes Back The Drafts

`Discard` deleted the whole batch, comments already handed to the agent included. Deleting our copy of a comment the agent is working from takes nothing back, it just loses the record of what was asked. It now deletes the comments you have not sent yet, says how many in its confirmation, and greys out when there are none left to take back.

### Deleting A Pull Request Comment Asks The Same Question Everywhere

The trash of a row in the Review panel deleted straight away, while the same deletion from the diff asked first when the comment was already on GitHub. Both go through the same question now: a comment of a review you have not submitted goes without a word, one that is already public asks, wherever you delete it from.

### One Footer In The Review Panel

Reviewing for the agent and for a pull request at the same time stacked two action bars at the bottom of the panel, each with its own border, and neither said which comments it was talking about.

There is one footer now, and it acts on the destination your rows point at: walk into the agent's comments and it offers `Discard` and `Send`, walk into the pull request's and it offers `Submit review`. When both destinations have comments, the footer names the one it acts on.

### Review Rows Are The Height Of Every Other Row

The rows of the Review tab were 8px taller than the rows of Changes, Files, History and the files of a pull request: they padded themselves on top of the padding every row of the dock already pays. A comment now takes exactly the height of any other row, so the panel shows more of the review without scrolling.

### The Title Of A Review Section Ticks Its Comments

Only the small tick box of `To the agent` selected the whole batch. Its title now does too, so the whole header row is the target instead of a 14px square.

### The Palette Submits A Pull Request Review

Sending a review to the agent had a command, submitting one to GitHub did not: the only way in was the button at the bottom of the Review panel. `Submit pull request review` is in the palette now, offered only while you have comments waiting on a pull request, and it opens the same decision dialog.

The three review commands also left the `Changes` group, where they sat between commit and cherry-pick. They have a `Review` group of their own, right after `Changes`.

### One Step Of Spacing In A Comment Card

The spacing inside a comment card followed no rhythm. A reply's separator sat against the text above it and held all of its air underneath, and the reply box put its own space above `You` and almost none below. The first comment, its replies and the reply box now breathe the same.

### Only The Buttons That Do Something

`Resolve conversation` was there on conversations nobody could resolve, greyed out and taking the room a crowded header did not have: a comment of a review you have not submitted, or a pull request you only have read access to. The button now shows only when it does something, and a conversation someone else resolved says so with a `Resolved` tag instead of a dead button.

### A Pull Request Comment Says Its Line Only When It Has To

A comment on a pull request carried its line number even when it sat on the line it was talking about, which the diff already shows. Only a range says both of its ends now, and a comment GitHub calls outdated still says where it hangs, because that is no longer where it was written. Ranges read the same everywhere: `L10-L12` in the diff as in the Review panel.

### Tab Reaches The Panel Now

Tab and Shift-Tab moved between the controls of the window without ever stopping on the lists of the right panel, which were not in the order at all. They are now the first stop of their tab, so Tab takes you from the file list to the commit message box and Shift-Tab brings you back, the way a form does.

The commit button and its menu come after the message box rather than before it, which is the order you use them in.

In the terminal, Tab goes back to being Tab. It used to walk the focus out of the panel, which meant no shell completion: pressing it once sent you somewhere else in the window. The terminal now claims the key for itself, Shift-Tab included.

### A Panel Key That Brings You Back

The key of a panel used to mean "toggle". Once Enter had moved the keyboard to the file, pressing `cmd-shift-e` to get back to the Changes list closed the panel instead, and you had to press it twice.

It now means "take me there", and only means "get out of the way" when the keyboard is already in that surface. Typing a commit message and reaching for the file list works too: the box is in the panel, but it is not the list, so the key moves you rather than closing anything.

Escape completes the loop. From any list of the panel it hands the keyboard back to the file you were reading, without closing the panel and without losing your place in the list. The terminal keeps its own Escape, as a shell must.

### The Terminal And The Pull Request Files Answer Too

`cmd-j` opened the terminal and left you to click in it before you could type. It hands the keyboard over now, as every other tab of the right panel does.

The files a pull request proposes are a real list at last: arrows walk them and show each diff, Enter opens one and gives the editor the focus, and asking for the tab while the files are still loading now lands the keyboard on them the moment they arrive.

### The File Trees Answer The Keyboard Too

Files and History now behave like the Changes and Review lists: arrows show what the row holds and leave the keyboard where it is, Enter opens the file and hands the editor the focus.

History gains more than that. Pressing Enter on a file of a commit did nothing at all, and walking through them showed nothing either, so the only way in was the mouse. Both work now.

Reviu also stopped guessing what you did in the Files tab. It used to compare the selected row against the previous one on every repaint to notice a change; the tree now says so itself, which is why walking it no longer drags the editor along.

### Walking A List Is Not Choosing From It

Moving through the Changes list or the Review list with the arrow keys used to open the row and hand the keyboard to the editor, so the next arrow key went into the file instead of the list. One keypress of browsing, then you were somewhere else.

The two gestures are now told apart. Arrows show what the row holds and leave the keyboard where it is, so you can walk a whole review without touching the mouse. Enter, or a click, opens the file and gives the editor the focus, which is the moment you actually wanted to be there. The shortcut of the panel brings you back.

Crossing a long list no longer loads everything on the way either: a row has to be the one you stopped on before its file is read.

In the Review panel, left and right now fold and unfold a file, the same keys the Files and History trees use. Enter keeps folding a file row too, since there is nothing to read behind it.

### The Right Panel Answers The Keyboard

Opening a panel with its key used to be as far as the keyboard went. Files, Review and History now hand it over properly: arrows walk the rows, Enter opens what is selected, and left/right fold a folder or a file.

History was the surprise. Its key opened the tab but the arrows went nowhere, because the focus landed next to the commit tree rather than on it. It has always been that way; it works now.

Review gained the most. Its comments are a real list: walk them with the arrows and each one opens on its lines as you pass, fold a file with Enter on its row, and the list scrolls to follow the selection instead of leaving it off-screen. The Send and Submit buttons now stay at the bottom of the panel when both destinations have comments, rather than sitting between the two groups where a long review pushed them out of view.

### Review A Pull Request You Have Not Checked Out

The Open in Reviu button of the browser extension used to say no whenever the pull request was not the one of the branch you had open. It now does the work: Reviu names the branch that carries the pull request, asks before touching your repository, checks it out (fetching it first when only the remote has it), and opens the Pull request panel on it once git has moved.

It asks first because moving your branch is your call, and it will not do it while the agent is mid-turn, for the same reason switching branch from the palette waits. A working tree holding changes that the checkout would overwrite stops it, as any branch switch does, and says so instead of stashing behind your back.

A link to a pull request of another repository still goes to github.com, and now says which repository Reviu has open rather than leaving you to guess.

### A Key For Every Surface Of The Right Panel

The right panel holds six surfaces and only three answered to the keyboard. Files, Review and Pull request now have a key of their own: `cmd-shift-f`, `cmd-shift-r`, `cmd-shift-p`, next to the Changes, History and Terminal keys that were already there. They behave the same way: the key opens the panel on its surface, and pressing it again while that surface is showing closes the panel.

### Every Shortcut In Settings Does Something

The keyboard shortcuts page listed ten keys that had stopped working when the pull request page went away: its two tabs, its commit-by-commit navigation, its jump from one review comment to the next, its file tree, and Switch to PR branch. They are gone from the list, and Switch to PR branch leaves the command palette with them. Checking out the branch of a pull request comes back later, from the link rather than from a key.

`cmd-r` goes too. Nothing in the workspace refreshed a whole page any more, so the key did nothing wherever you pressed it, and the refresh button it belonged to never appeared in the top bar.

A shortcut you had rebound to one of those keys is simply ignored, and the key is free again for whatever you want to put on it.

### One Place To Review, Instead Of Two

Reviewing a pull request used to mean leaving the workspace for a page of its own, with its own file tree, its own diff and its own comment cards. That page is gone. Everything it did happens in the workspace now: the branch's pull request in the right panel, its files opened in the centre, its comments in the diff and in the Review tab, its checks, its reviewers and its merge button in one collapsible block.

What that changes for you: a link to a pull request (from the browser extension, the command palette, or the GitHub inbox) opens the panel when it is the pull request of the branch you have open. When it is not, it opens on github.com, and the extension button says which branch to check out to review it here. The workspace keeps its keyboard shortcuts, the pull request page's own ones are gone with it.

The Files tab of the right panel now opens only the folders holding uncommitted work, instead of the whole repository at once.

### Surfaces That Say What They Are For

The Pull request tab and the GitHub inbox used to be empty, or absent, for anyone without GitHub access. They now say what they would hold and how to get there: sign in, or start the Reviu Pro trial. Someone already signed in is asked to subscribe rather than to sign in again.

The Pull request icon also stays in the rail whatever the repository is: a rail whose icons come and go with your remote is a rail you cannot learn.

### A Comment You Wrote Does Not Get Lost

Writing on a pull request file puts the comment in the Review tab, which is not the tab you were working in. The Pull request panel now says so above the file list ("3 comments waiting in Review") and takes you there in one click. It disappears when nothing is waiting.

### One Review Panel, Two Destinations

The Review panel now says where each comment is going. Comments written on your working tree sit under "To the agent" and behave as before. Comments belonging to a pull request review you have not submitted yet sit under "To this pull request", read back from GitHub rather than kept locally, so a review you started in the browser is there when you come back to the app.

Clicking one of those rows opens the file as the pull request proposes it, on the comment's line, not the working tree file that may hold something else entirely on that line. Dropping a comment you no longer want deletes it from your unsubmitted review straight away.

Ticking comments stays what it was, an agent thing: GitHub submits a review whole, so its section has no checkboxes rather than promising a choice the API cannot honour. Clicking a comment row also lands on the right line now, instead of one line above it.

Finishing the review happens from that section: Submit review asks for the decision (comment, approve, request changes) and its message, tells you how many comments go out with it, and refuses an empty message where GitHub would. Approving a pull request you have nothing to say about works too, from a Review button next to Merge. Reviu will not let you approve your own pull request, which GitHub would refuse anyway.

Commenting happens on the diff itself. Open a file of the pull request, write on a line, and the comment joins the review you are building (or goes out on its own if that is what you pick, as on GitHub). Existing conversations show on their lines, with replies, resolving and unresolving, and editing or deleting your own words. Deleting a comment already published asks first; a draft of your unsubmitted review does not.

Not there yet on those cards: the composer's preview tab, dropping images into a comment, applying a suggested change as a commit, and following a link out of a comment body.

That block of the Pull request tab is now always there, even for a pull request with no CI and no reviewers: it holds what you can do to the pull request, so a small project no longer loses its merge button for lack of a test suite.

### The Pull Request You Are On, In The Right Panel

Checking out a branch that has a pull request now fills the Pull request tab with the whole picture, without leaving the workspace: the title and number, who opened it, the branches it goes between, and the list of files it proposes. Clicking a file opens the diff of the pull request itself, base against head, so you read what the branch adds rather than what your working tree happens to hold.

Under an expandable Details block are the CI checks, one row per check with its status and how long it took, and the reviewers with where each one stands. The block leads with the bad news: a failing suite or a requested change is what you see first, without expanding anything.

Merging happens there too. The button names the method your repository actually uses, so "Squash and merge" says what will happen, and it stays disabled until Reviu knows the pull request can merge, not merely when it knows it cannot. A confirmation names the method again before anything is pushed, because merging is not a gesture to make by accident from a narrow column.

### See Your Review Before You Send It

The comments you leave on a diff now have a home: a Review tab in the right panel lists the whole batch, grouped by file, with each file collapsible. Click a comment to open its file and land on the lines it is about, delete one you changed your mind about, and see at a glance which ones the agent already addressed and which ones the code moved under.

Sending happens from that panel too, next to a Discard button that clears the review after a confirmation. The diff header keeps only the tools you use while reading. When the panel is closed a dot on its rail icon says a review is waiting, `cmd-shift-a` still sends, and the command palette gained "Send review to agent" and "Discard review". The Changes icon wears the same dot when the working tree has something in it.

### Send Part Of A Review

A review no longer has to go out all at once. Tick the comments you want in the Review panel, or a whole file in one click, and Send carries only those. Leave everything unticked and it still sends the whole batch, so nothing is ever lost by not choosing; "Select all" gives you the other way round, starting from everything so you can untick the two you want to keep for later.

Single comments also carry a send arrow, both on their row in the panel and on their card in the diff, for putting the agent back on one comment without resending the other four. Whatever stays behind stays a draft and goes with the next send.

### A Review That Waits For You

A review you have not sent yet no longer dies with the session. Close the app, switch to another repository, come back a week later: the comments you were writing are still there. Each repository keeps its own batch, so moving between projects no longer costs you the eight comments you had just written.

The batch is saved next to that repository's conversations, and it is written the moment anything changes, so there is no window where a crash costs you a comment. Discarding a review removes it for good. A batch nobody touches for a month is cleaned up along with the old conversations.

### Comments That Do Not Pile Up

Review comments are instructions to the agent, not a record to keep like a pull request's comments. So they now last exactly as long as they need to: you write them, you send them, and when the agent finishes that turn they leave the panel. Nothing to tidy up afterwards, and no growing list of comments you can no longer act on.

Two things follow from that. What you write while the agent is working stays: only the comments that actually went out leave. And a turn you stop, or one that fails, changes nothing, because nothing was done with them: they are still there, ready to go again.

Reviu no longer guesses whether the agent honoured each comment by watching the diff, which it often got wrong: a comment written in prose was marked "Outdated" whether the agent had done the work or ignored it. If the agent missed something, you see it in the diff and say so again. The text you sent stays in the conversation either way.

### A Calm CPU While The Agent Works

A busy agent session no longer drives the app to 100% CPU. Reviu used to rewrite the whole conversation to disk and re-read every conversation in the sidebar for each streamed chunk; transcripts now save on a short throttle (and always at turn boundaries, on switch and on quit), the sidebar updates from memory, and bursts of agent output collapse into a single UI update. Long sessions stay smooth from start to finish.

The screen now also repaints at a steady beat while the agent streams instead of once per chunk: text, thoughts and terminal output gather for a fraction of a second and land together, so a fast provider or a chatty build no longer turns into a redraw storm. Nothing is lost or reordered, and the first word of a reply still appears instantly.

The pulsing "thinking" indicator used to quietly force the whole window to redraw at display refresh rate for the entire turn; it now animates on the same beat as the stream, which removes the last constant CPU drain while the agent works.

Scrolling a conversation full of edits is much lighter too: the small diffs and numbered outputs inside tool cards are now drawn as a single block instead of a stack of nested rows, cutting most of the layout work a busy screen used to redo on every frame. Text in outputs stays selectable, and selecting can now sweep across lines in one drag instead of stopping at each line.

### Instant Conversation Switching

Opening another conversation from the sidebar no longer freezes the app while the transcript loads: the current conversation stays on screen, the target row shows a small spinner, and the switch lands as soon as the transcript is ready. Saving also moved fully off the main thread, and the sidebar now lists conversations from a small index instead of re-reading every transcript, so repositories with a long history open their session list instantly. Each sidebar row also shows a one-line preview of the conversation's last message.

### Drafts That Wait For You

A half-typed message now stays where you left it: each conversation keeps its own composer draft, restored when you come back to it, after switching sessions or restarting the app. Sending the message clears its draft.

### Conversations Reopen Where You Left Them

Switching back to a conversation returns you to the exact spot you were reading instead of jumping to the bottom. A conversation left at the bottom still opens at the bottom and keeps following new messages.

### Terminal Colors

Command output in tool cards now shows the colors the tools emit: cargo's greens and reds, test runner highlights, bold and underlined text all come through, adapted to light and dark themes. Escape sequences that used to leak as garbage characters are cleaned away, progress bars settle on their final state, and copied text stays clean.

Agent backends and terminals Reviu owns now ask common CLI tools to keep color on even though their output is captured, covering Cargo, pnpm, npm, Vitest, Jest and other color-aware commands without needing extra flags.

### Select Text In Terminal Output

The live output of commands running in tool cards is now selectable like everything else in the conversation: sweep across lines, double-click a word, triple-click a line, and the text lands in your clipboard on release. Long lines also wrap now instead of being cut off at the edge, matching how file reads and diffs behave.

### Select Text In Diffs

The small diffs inside tool cards are now selectable: sweep across added and removed lines, double-click a word or triple-click a line, and the selection lands in your clipboard on release, same as tool outputs.

### Line Numbers In Read Results

Read tool results in the agent conversation now show file line numbers, so you can refer back to exact lines without reopening the file. When the agent reads from an offset deep in a file, the numbers now start at the real file line instead of restarting at 1.

### Push With Saved Git Credentials

Pushing and fetching over HTTPS now use the credentials already saved in your Git credential helper, such as the macOS keychain. If `git push` works in your terminal, the same remote can now work from Reviu too.

### See The App Through The Driver

On macOS, the Reviu driver can now run the real UI through the visual renderer and save screenshots from JSON-lines commands. Agents can click, type, wait and capture the off-screen app while debugging, while the existing selector-based test backend remains available everywhere.

### Line Numbers In Agent Diffs

The small edit diffs inside the agent conversation now show old and new line numbers, so you can point the agent at the exact line you are reviewing without opening the full diff.

### Every ACP Agent, Not Just Three

The agent picker is now served by the official ACP registry instead of a list baked into the app. Twenty-three agents are available out of the box, Gemini, Copilot, Qwen, Cline, Kilo, Grok and the rest alongside Claude, Codex and Pi, each launched with the version the registry publishes. The list scrolls rather than running off the screen, and refreshes itself in the background, so new agents and new versions arrive without an update to Reviu. A copy ships with the app and another is cached on disk, so the picker is fully populated on first launch and stays populated offline, and the refresh only asks the network when the cached list has aged. Your saved agent and its selected model carry over. When an agent will not start, the message now points at that agent's own page instead of a generic one. Every agent shows its own icon in the picker: Reviu fetches them alongside the list and keeps them on disk, and the agents you already know keep their mark even before anything is downloaded.

### Pi Joins Claude And Codex

Pi is now available in the agent picker alongside Claude and Codex, with its own icon throughout the conversation. Pi needs its CLI installed (`npm install -g @earendil-works/pi-coding-agent`); if it is missing, Reviu says so up front instead of failing on the first prompt. Claude and Codex also move up to their latest agent releases.

Steering now follows what the agent can actually do. Claude and Codex take a message mid-turn as before; with an agent that cannot, Cmd+Enter simply queues like Enter and the queued message's steer button is gone, instead of trying and coming back with a refusal.

### Approve For Me

A new Auto-approve toggle in the composer answers the agent's permission requests for you, always picking the allow option, so long unattended runs stop parking on a question. Flipping it on also releases a request already waiting. Cards answered this way say "Auto-approved" so you can tell them apart later, a request with no allow option still waits for you, and the choice sticks with the conversation.

### Watch Commands Run

The agent's shell commands now run in terminals Reviu owns, and their output streams live into the conversation: a compact block under the tool step shows the command, the tail of its output as it arrives, and the exit code when it finishes (in red when it failed). A running command carries a stop button, so a hung build no longer holds the turn hostage.

### Steer The Turn, Don't Wait For It

Cmd+Enter (Ctrl+Enter on Linux/Windows) now sends your message straight into the running turn instead of queueing it, so you can redirect the agent mid-flight: "actually, skip the tests" lands immediately. Queued messages gain a steer button to send them into the current turn too. If the agent refuses the injection, the message safely returns to the queue with a notice, and the turn keeps running. Plain Enter still queues, and outside a turn both keys just send.

### Your Prompt Stays Put

Sending a message now pins it to the top of the conversation, and the reply streams into the space below it: nothing scrolls under your eyes anymore. Scroll the wheel and the hold lets go so you can read freely; the jump-to-bottom pill brings you back to the held position. Once a reply grows past the reserved space, reading continues as plain scrolling.

### Catch the Tail of a Long Reply

The jump-to-bottom pill now sticks: one click and the conversation follows the streaming reply until you scroll up to read, instead of dropping you the moment the next words arrive. Scrolling back down to the end re-engages the follow on its own, and Cmd+Shift+J (customizable in Settings) jumps to the latest message from the keyboard, so a long answer never turns into a clicking exercise.

### The Turn Folds Into Its Card

A finished turn now tucks its work away: the thinking, the tool steps and the in-between narration fold behind the turn's summary card, leaving only your question, the agent's final answer and the card itself. A "Worked for 2m 5s · 8 steps" row on the card tells you what happened and how long it took; click it to unfold the full timeline for inspection, click again to tidy it back up. Turns that edited nothing keep their usual compact grouping.

### A Receipt For Every Turn

When the agent finishes a turn that touched files, the conversation now closes it with a summary card: how many files were edited, the added and removed line counts overall and per file. Each file row opens its diff, Review jumps straight into the diff view, and Undo reverts the turn's file changes while keeping the whole conversation, the card simply flips to "Undone". Undo is only offered on the latest turn, so it can never clobber work from the turns that came after. Rewinding both files and conversation stays available on the checkpoint divider and message editing. Long file lists fold behind a "Show more" toggle, and the card is part of the transcript, so it is still there when you come back.

### Small Alignments

The sessions list and the GitHub inbox show a scrollbar while scrolling. File search results use the same text size as the command palette instead of a larger one, and the right panel's titles match the sidebar's. In the sessions list, the timestamp now sits flush right; the delete button appears in its place on hover instead of reserving an empty gap. The copy button under each message no longer reserves an invisible line either, so messages sit closer together.

### No More Deja Vu On Reopen

Coming back to a conversation no longer repeats its last thought or reply: the history an agent replays while resuming a session is now recognized and dropped, since the transcript already has it. The finished-turn popup also shows the icon of the agent that actually worked, Codex or Claude, and dragging an image over the composer now tints it so you can see it is a drop target.

### A Calmer Transcript

The conversation sheds its scaffolding: no more rail and colored dots down the left, no more amber "Working" flashes; the activity verb and timer sit quietly in gray. Tool steps get a small icon tile and read as one clean line; only a failure brings color. Thoughts become a single discreet line that still expands on click. Your messages align right as proper bubbles, and images you attach now show as real thumbnails in the message, with a clear gap between staged images and your text in the composer. Rolling back all the way no longer leaves a lonely "Checkpoint" divider floating in an empty conversation. The thinking glimpse no longer opens holes for blank lines or shows raw markdown marks, expandable lines brighten on hover instead of painting a gray bar, and a step whose title already says the verb, like "Editing files", stops stuttering it twice.

### A Cleaner Conversation

A polish pass across the transcript. Your prompts now read as tinted bubbles instead of look-alike input fields. Long commands truncate on one line with the full text a hover away instead of running off the edge. Tool steps and the thinking between them now fold together under one summary, so a working session collapses cleanly. The live thinking glimpse is properly dimmed. Messages sent with images say so. Creating an empty file shows "(empty file)" instead of a bare green band, tool titles like "Editing files" no longer lose their first word, permission cards stop repeating their own title and answers show the button you pressed. And the Roll back pill now sleeps while a turn is running instead of appearing clickable and silently refusing.

### Read The Diff Without Losing The Conversation

Opening a file no longer hides the conversation: the diff now opens beside it, with the conversation in a resizable column on the left. Watch the agent keep streaming while you review its edits, drag the divider to taste, and Escape still closes the file to give the conversation the full width back. The interactive rebase keeps the whole center, as before.

### Show The Agent A Screenshot

Paste an image into the composer or drop one onto it and it stages as a thumbnail, removable with one click, then rides along with your next message. Dropping a regular file inserts it as an @ mention instead. Image attachments appear only when the connected agent actually accepts images.

### Know When The Agent Needs You

Working in another app while the agent runs? Reviu now shows a small popup in the corner of your screen when a turn finishes or when the agent waits on a permission, and clicking it brings you straight back. It only appears while the window is inactive, never over your work in Reviu itself, and a queued message running on its own never fires one. A switch in Settings under Agent turns it off.

### Streaming That Stays Fast

Long replies no longer get slower as they grow: each streamed chunk now extends the rendered reply incrementally instead of re-reading the whole message every time. Tool updates got the same treatment, recomputing diffs and syntax colors only when their content actually changes; a status change costs nothing. A side effect you can see: re-sent tool updates no longer collapse the diff you had expanded.

### Tool Calls Fold Into One Line

Consecutive tool calls now group under a single summary line, like "Ran 2 commands · Edited 1 file". The group stays open while the agent works so you can watch, folds when the turn ends, and one click pins it the way you want, remembered across restarts. A failure shows as a count in the summary and in red on the step itself, without painting the whole group red.

### A Sturdier Conversation

A batch of fixes under the hood: words the agent streams right as a turn ends are kept instead of silently dropped; permission cards survive a reload, showing what was asked and answered; the plan checklist updates in place instead of duplicating when tool calls interleave; the file list behind @ mentions refreshes after every turn, so files the agent just created are mentionable; and two conversations created in the same second no longer collide on disk.

### Edit A Message And Replay From There

Hovering a message now reveals a copy button, and your own prompts gain an edit button: change the text, press Send, and Reviu restores the files to the checkpoint taken before that prompt, drops the turns after it, and replays the conversation from your new wording in a fresh session. Copying grabs the message as written, formatting included.

### Slash Commands From Your Agent

Typing "/" at the start of the composer now opens the commands your agent actually offers, from Claude's built-ins to your project's own, filtered as you type. Arrows navigate, Enter completes the command, and your arguments follow. The menu only appears for a leading slash: a path in the middle of a sentence stays a path.

### Type The Next Message While The Agent Works

You no longer wait for a turn to finish before writing the next instruction. Pressing Enter while the agent works queues the message in a card above the composer, ready to edit or remove; when the turn ends it is sent as the next one, and the following queued message after that. Stopping a turn holds the queue: nothing runs until you decide.

### Back To The Latest Message In One Click

Scrolling up through a conversation while the agent keeps writing no longer strands you: a small pill appears at the bottom of the transcript and one click glides back to the latest message. Stopping a turn now leaves a "Stopped" marker in the conversation instead of ending silently, and everything the agent had already streamed stays visible. And when the agent process dies, the error offers a Reconnect button right there.

### See What You Are Approving

Permission requests now show the thing being approved, not just a title: the full command for a shell run, the URL for a fetch, per-file added and removed counts for an edit, and the file being touched. Long commands scroll in place and can be selected and copied. The buttons were already reachable with the keyboard; now you know what you are pressing them for.

### Watch The Agent Think

While the agent reasons, the conversation now shows a live glimpse of its thinking: the latest lines, dimmed, fading out at the top as they scroll past. When the reasoning ends it folds into the usual collapsed thought, ready to expand. Before, a long think was just a spinner.

### The Conversation Reads In Order

The agent's narration now appears where it happened: an explanation before a file edit stays before the edit, thoughts close before the prose they led to, and commentary between two commands keeps its place. Before, everything the agent said during a turn was collected into a single block at the very end, after all the work.

### Code Blocks Worth Reading

Code blocks in the agent's replies are now syntax highlighted, using the same fifty-language engine as the rest of the app. Each block shows its language and carries a copy button that grabs the code without the fences.

### Enter Sends, Shift+Enter Starts A New Line

In the agent composer, Enter now sends the message and Shift+Enter inserts a line break. Before, every Enter variant sent at once and there was no way to write a multi-line prompt from the keyboard.

### Settings You Can Read And Edit

Reviu now keeps its settings in a plain `settings.json` file in its configuration folder, next to its other data. Existing settings move over automatically on first launch. The file is yours: read it, edit it by hand, keep it in your dotfiles. A hand-edited file never breaks the app - an unknown or invalid value simply falls back to its default, and Reviu tells you at startup if the file could not be read at all.

### Keyboard Shortcuts In A File

Custom shortcuts now live in a plain `keybindings.json` next to `settings.json`, one line per command. Existing overrides move over automatically. Edit it by hand or keep it in your dotfiles: an entry Reviu does not recognize is left untouched for the version that wrote it, an invalid one is simply skipped, and a file that no longer parses is reported at startup and never overwritten.

### One Workspace

The separate Git page is gone: everything it did happens in the Sessions workspace, which is now the only place Reviu opens. The changes list with hunk staging, conflicts, the history, the terminal, branches, stashes, cherry-pick, interactive rebase, the commit menu and every keyboard shortcut moved there, next to the agent and the diff. `cmd-1` still goes to Sessions, `cmd-2` no longer exists, and old links to the Git page land in the workspace. The Sessions/Git switch in the header is gone with it.

### The Right Panel Opens And Closes

The right panel's text tabs are gone: a permanent icon rail carries Changes, Files, History, Pull request and the Terminal, so nothing truncates however narrow the panel gets, and its header names the open surface instead. Clicking an icon opens its surface, clicking the active one closes the panel, exactly like the shortcuts: `cmd-j` on the terminal or `cmd-shift-e` on the changes toggle it. An open/close toggle sits at the top of the rail. The sessions sidebar collapses into a slim rail the same way, keeping New session at hand. Panels slide in and out, their edges drag to resize with a double-click returning the default width, and an expand button gives the right panel the whole window for a large diff or a long terminal session. Focus always lands somewhere alive: closing a panel hands it to the diff or the composer, and opening an empty changes tab keeps the keyboard working.

### Hunk Actions On Both Sides Of The Split

In split view, hovering a change on the right side did not show the stage and restore buttons; only the left side did. Both panes now light up the hunk actions.

### Clicks Beside The Editor Stay Beside The Editor

In split view, pressing a button at the edge of the workspace - a panel tab, for instance - could also start a text selection in the editor underneath, which then followed the mouse. The editor now only takes a press that visibly lands on it.

### A Clean File Never Splits

Opening an unmodified file while the split preference was on showed the same content twice, side by side. A file with no changes now always opens inline, and the split toggle stays quiet on it instead of silently flipping the preference. The open file follows along: saving away its last change drops the view back to inline, and a new change brings the split back.

### The Composer Keeps Your Words

Pressing Enter while the agent was still connecting, or after it had failed, silently emptied the message box and sent nothing. The message now stays in the composer until the agent actually receives it.

### Reviu Pro Offered Where It Makes Sense

Pushing a branch of a GitHub repository without Reviu Pro shows a single notification, once per session, explaining that Pro brings pull requests, reviews and notifications into the app, with a way straight to the plans. Nothing is shown to subscribers or to repositories that are not on GitHub.

### A Conflicted File Is Read Whole

Once you resolve the markers of a conflicted file by hand, there is no diff left to read, only the file. The Sessions workspace now shows such a file in full, as the Git page did, and goes back to a normal diff as soon as the resolution is staged.

### A Renamed File Says Where It Came From

Opening a renamed file in the Sessions workspace showed only its new name. The header now names both sides, old name struck through then the new one, the way the Git page did, and a deleted file keeps its struck-through title.

### Only Git Repositories Get Opened

Picking a folder that was not a git repository selected it anyway, remembered it, and reopened Reviu on it at the next launch with a git error in the panel. Reviu now says the folder is not a git repository and keeps the repository you had. Picking a folder inside a repository selects that repository instead of failing, and a folder that stopped being one drops out of the recent list.

### The Branch's Pull Request From The Command Palette

The pull request of the current branch was only reachable by opening the Pull request tab and clicking. The Sessions command palette now mirrors that tab: `Open pull request #n` when the branch already has one, `Create pull request` when it does not, and nothing at all without GitHub access.

### Publish A Branch And Open Its Pull Request In One Go

Creating a pull request for a branch that had never been pushed sent the request to GitHub for a branch it did not know, and it failed. The Pull request tab now says the branch is not on the remote yet and offers `Publish and create pull request`: Reviu pushes the branch first and only opens the pull request form once the push succeeded.

### Restore All From The Sessions Palette

Discarding every change at once was only reachable from the Git page toolbar. The Sessions command palette now carries `Restore all`, offered whenever the working tree has something to discard, with the same confirmation before anything is thrown away.

### The Sidebar Asks For A Repository

On a fresh install the Sessions sidebar showed nothing about repositories, so the only way to open one was the command palette. The bottom row of the sidebar now offers `Open repository` when none is selected, and turns back into the repository name, branch and ahead/behind counters once you pick one.

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

### The Git Keyboard Shortcuts Work In The Sessions Workspace

Every local-git shortcut now works where the work happens: `cmd-o` open a repository, `cmd-u` pull, `cmd-y` push, `cmd-shift-y` force push, `cmd-shift-b` switch branch, `cmd-j` terminal, `cmd-shift-h` history, `cmd-shift-e` changes, `cmd-enter` stage or unstage the open file, `cmd-backspace` discard it. The tab shortcuts open their tab in the right panel and put the focus there, so the keyboard keeps going. A shortcut whose command cannot run right now does nothing rather than failing.

### Send A Selection To The Agent From The Sessions Workspace

Selecting lines in the diff and pressing `cmd-shift-l` attaches them to your next message, without leaving the workspace: the agent is right there. `cmd-f` searches the open file from anywhere in the workspace, not only when the editor already has focus, and Escape closes the search before it closes the file.

### Branches, Stashes And Cherry-Pick In The Sessions Workspace

The palette of the Sessions workspace now carries the commands that need a list: switch, create and delete a branch, merge or rebase onto one, cherry-pick commits, stash (with or without untracked files), then apply, pop or drop a stash. Switching branch is refused while the agent is mid-turn, for the same reason switching repository is: the ground cannot move under a running turn.

### Amend, Undo And Stage The Open File From The Sessions Workspace

The commit button in the right panel gains the menu the Git page has: Amend, Undo last commit, Push and Force push (with lease), greyed out when they cannot run. The same commands join the palette, along with Checkout detached and Stage / Unstage the file you have open. Amending takes whatever is in the commit box, or keeps the previous message when the box is empty.

### Stage One Hunk At A Time In The Sessions Workspace

The Sessions workspace could only stage a whole file. Hovering a change in the diff now brings up Stage, Unstage and Restore for that hunk alone, exactly like the Git page, and `shift-enter` / `shift-backspace` do the same from the keyboard. On a conflicted file the same spot offers Accept Current, Accept Incoming and Accept Both per conflict block, with `cmd-shift-enter` for both sides. The Changes tab follows each of these without waiting for anything else to refresh it.

### Rewrite History From The Sessions Workspace

Interactive rebase is available in the Sessions workspace: pick it from the palette on a branch or on the last N commits, and the todo takes the whole center, where a table of commits belongs. Reorder, squash, fixup or drop, then apply; a rebase that stops on a conflict opens the file with the message git prepared. It is offered only with a clean working tree, and Force push joins the palette next to Push for the branch you just rewrote.

### Resolve Conflicts In The Sessions Workspace

A conflicted file opened in the Sessions workspace now carries the same tools as the Git page: Accept All Current and Accept All Incoming in the diff header, and the same two commands in the palette. The arrows next to them, and `cmd-alt-up` / `cmd-alt-down`, walk conflict by conflict on a conflicted file and change by change everywhere else, with a counter saying where you are.

### Finish A Rebase Or A Merge From The Sessions Workspace

A rebase stopped on a conflict used to leave the Sessions workspace with a Commit button that could not help. The right panel now says which operation is running, and Commit becomes Continue rebase, enabled once the conflicts are resolved and staged. The palette follows: Rebase continue, Rebase skip, Abort rebase and Abort merge appear only while there is something to continue or abort, and the commit message git prepared for the merge lands in the box on its own.

### A Comment The Code Moved Under Says So Instead Of Vanishing

When the agent rewrote the lines a review comment was anchored to, the comment disappeared from the diff without a word. It stays now, tagged Outdated, so you can see what you asked for and decide. It is left out of the batch sent to the agent: the code it talks about is gone.

### Browse The History From The Sessions Workspace

The right panel gains a History tab: your commits, expandable to the files each one touched. Click a file to read it as it was in that commit, right where the diff shows up, read-only so a snapshot can never overwrite your work. Opening the same file from the Changes tab brings the working-tree version back. The history loads the first time you open the tab, not before.

### A Command Palette That Only Offers What Works

The Sessions workspace listed Commit, Stage all, Unstage all, Push and Pull whether or not they could run: committing with nothing staged and no message, pushing a branch with nothing ahead. The palette now follows the same rules as the Git page, so a command is there when it does something.

### Conflicts Stop You On The File, Not On An Error

Git commands that stop on conflicts (merge, rebase, skip, pull) now say which file is waiting and put it on screen, in the Git page as before and in the Sessions workspace too. Every git command in the app now shares one implementation, so the same command reports the same thing wherever you run it.

### Jump Between Changes In The Sessions Workspace

Reviewing a long file in the Sessions workspace meant scrolling to hunt for the next change. `cmd-alt-down` and `cmd-alt-up` now walk the diff change by change, as on the Git page and the pull request diff, and wrap around at the ends. While a Markdown or SVG file is previewed there is no diff to walk, so the shortcuts stay out of the way.

### Task Lists Show Their Checkboxes

Markdown checklists came out as an empty list wherever they appeared: agent conversations, pull request descriptions, review comments and file previews. They now render every item with its checked state.

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
