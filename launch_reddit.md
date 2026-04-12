# Launch Reddit

## Positioning

Reddit should be treated as feedback, not a launch broadcast.

The strongest angle for Reviu is:

```text
I built a native desktop Git client because I wanted local Git and GitHub PR review in one focused workflow.
```

Avoid leading with pricing, Product Hunt, or a launch discount. Mention the website once, near the end, and ask for specific feedback.

## Subreddits To Consider

- `r/SideProject`: best first post. Builder story, honest context, screenshots/video, direct feedback ask.
- `r/indiehackers`: good for launch/pricing/product feedback if the community allows launch posts.
- `r/SaaS`: useful for pricing and positioning feedback, but less focused on developer users.
- `r/rust`: only if the post is technical and mostly about Rust/GPUI implementation. Do not lead with the product pitch.
- `r/git`: only if rules allow tool posts. Keep it focused on Git workflow feedback.
- `r/github`: only if rules allow GitHub workflow tools. Focus on PR review pain.
- `r/macapps`: good if macOS is ready and rules allow app launches.
- `r/linux`: good for the Linux release angle, but avoid pricing and keep it practical.

Before posting, check each subreddit rules and pinned weekly threads. If they have a feedback/showcase thread, use that instead of a standalone post.

## Main Reddit Post

Good for `r/SideProject`, `r/indiehackers`, or a feedback thread.

**Title**

```text
I built a native desktop Git client for local Git and GitHub PR review
```

**Text**

```text
Hey everyone,

I'm building Reviu, a native desktop Git client focused on fast review workflows.

The free version covers local Git:

- reviewing diffs
- staging and restoring changes
- committing
- branching
- rebasing
- stashing
- cherry-picking
- resolving conflicts

Reviu Pro adds GitHub inside the app:

- notifications
- repository browsing
- pull request review
- comments
- checks
- issues

I built it because I wanted local Git and GitHub review in one place instead of constantly switching between desktop Git tools, the terminal, and the browser.

It's built with Rust and GPUI.

I'd really appreciate feedback on:

1. the local Git workflow
2. the pull request review flow
3. whether the Free vs Pro split makes sense

Site: https://reviu.dev
```

## More Casual Version

Use this if the subreddit is founder/build-in-public oriented.

**Title**

```text
I got tired of switching between Git tools, the terminal, and GitHub, so I built Reviu
```

**Text**

```text
Hey,

I've been building Reviu, a native desktop Git client for local Git and GitHub review.

The basic idea is simple: local Git workflows should be fast, and PR review should not require constantly jumping between a desktop Git client, the CLI, and the browser.

Right now Reviu supports local Git workflows for free:

- diffs
- staging
- commits
- branches
- rebase
- stash
- cherry-pick
- conflict resolution

The paid version adds GitHub context directly in the app:

- notifications
- repositories
- pull requests
- review comments
- checks
- issues

It's built with Rust + GPUI.

I'm mainly looking for feedback from people who review code often:

- what would make this useful in your daily Git workflow?
- what feels unnecessary?
- does the Free vs Pro split feel fair?

https://reviu.dev
```

## Technical Rust Version

Use this only for Rust/dev communities, and only if rules allow project posts.

**Title**

```text
I built a native Git client with Rust and GPUI
```

**Text**

```text
I've been building Reviu, a native desktop Git client written in Rust with GPUI.

The goal is to make local Git and GitHub pull request review feel like one workflow instead of switching between a Git GUI, the terminal, and the browser.

The local Git side includes diff review, staging, commits, branches, rebase, stash, cherry-pick, and conflict resolution.

The GitHub side adds notifications, repository browsing, pull request review, comments, checks, and issues.

The most interesting parts technically have been:

- rendering large diffs without making the UI feel slow
- keeping keyboard-first navigation consistent across local Git and GitHub PR review
- building a native desktop UI with GPUI
- keeping Git operations predictable instead of hiding too much behind abstraction

I'd be interested in feedback from Rust developers who review PRs often.

Site: https://reviu.dev
```

## Linux Version

Use this if posting in Linux-adjacent communities where app launches are allowed.

**Title**

```text
Reviu now runs on Linux: a native desktop Git client for local Git and GitHub review
```

**Text**

```text
I'm building Reviu, a native desktop Git client focused on fast review workflows, and it now runs on Linux.

The free version covers local Git workflows:

- reviewing diffs
- staging and restoring changes
- committing
- branching
- rebasing
- stashing
- cherry-picking
- conflict resolution

Reviu Pro adds GitHub notifications, repositories, pull request review, checks, comments, and issues in the app.

I built it because I wanted local Git and GitHub review in one place instead of switching between multiple tools.

I'd like feedback from Linux users especially:

- does the install/update flow feel right?
- are there desktop integration issues?
- does the Git workflow match how you work?

https://reviu.dev
```

## Pricing Feedback Version

Use this for `r/SaaS`, `r/indiehackers`, or specific feedback threads.

**Title**

```text
Looking for feedback on the Free vs Pro split for my desktop Git client
```

**Text**

```text
I'm launching Reviu, a native desktop Git client for local Git and GitHub review.

I'm trying to make the pricing split simple:

Free:

- local Git workflows
- diffs
- staging
- commits
- branches
- rebase
- stash
- cherry-pick
- conflict resolution

Pro:

- GitHub notifications
- repository browsing
- pull request review
- comments
- checks
- issues

Launch week pricing is $9/month until April 19, 2026, then new subscriptions return to $19/month.

My question: does this Free vs Pro split feel fair for a developer tool, or would you expect some GitHub features to be free too?

Site for context: https://reviu.dev
```

## Short Comment Version

Use this in weekly feedback/showcase threads.

```text
I'm building Reviu, a native desktop Git client for local Git and GitHub PR review.

Free covers local Git workflows. Pro adds GitHub notifications, repositories, PR review, comments, checks, and issues in the app.

I'm mainly looking for feedback on the local Git UX, PR review flow, and whether the Free vs Pro split feels fair.

https://reviu.dev
```

## First Reply

Post this as a comment after the main post if you want to add context without bloating the original.

```text
A bit more context:

I built Reviu because my own workflow kept bouncing between a Git GUI, terminal commands, GitHub notifications, PR pages, and issue pages.

The goal is not to replace the CLI for everything. It is to make review-heavy workflows faster: inspect changes, understand the branch, review comments/checks, and move through files quickly.
```

## If Someone Asks About Pricing

```text
Local Git workflows are free.

Reviu Pro is for the GitHub integration: notifications, repositories, pull request review, comments, checks, and issues.

During launch week it is $9/month until April 19, 2026, and active subscribers keep that price. After that, new subscriptions are $19/month.
```

## If Someone Says “Why Not Just Use CLI/GitHub?”

```text
That's fair. Reviu is not trying to replace the CLI for every Git operation.

The main use case is review-heavy work: moving through diffs, staging selectively, checking branch context, reading PR comments/checks, and keeping GitHub review close to local Git.

For people who are happy with CLI + browser, it may not be necessary. I'm building it for the workflow where that context switching gets annoying.
```

## If Someone Asks About Open Source

```text
The app is not open source right now.

It is built with Rust and GPUI, and the free version is meant to keep the local Git workflow usable without requiring a subscription.
```

## Posting Notes

- Do not post the same text to many subreddits at once.
- Start with one or two communities where the post genuinely fits.
- Customize the title and first paragraph for each subreddit.
- Avoid emojis and hype language.
- Avoid "upvote", "support the launch", or Product Hunt links unless the subreddit explicitly allows launch promotion.
- Attach one screenshot or a short video. Reddit posts with a visible product are easier to understand.
- Be ready to answer critical comments directly. Reddit feedback is often blunt; that is useful if you reply calmly.
- If a post is removed, do not repost the same thing. Use the weekly thread or ask mods what format is allowed.

## Best First Post

Start with this:

```text
Title: I got tired of switching between Git tools, the terminal, and GitHub, so I built Reviu
```

Use the `More Casual Version` body, attach one strong screenshot or short video, and ask for feedback on the Git/PR workflow rather than asking for support.

## Style References

- Recent Reddit launch discussions recommend posting where the real users are, not blasting generic launch communities.
- Posts that ask for concrete feedback tend to fit Reddit better than polished ad copy.
- Many subreddits restrict promotion, so weekly feedback/showcase threads are often safer than standalone launch posts.

## Reference Links

- Reddit Help: https://support.reddithelp.com/hc/en-us/articles/15484256976148-Growing-your-community
- Reddit SaaS launch discussion: https://www.reddit.com/r/SaaS/comments/1rdql22/how_do_you_launch_your_saas/
