# Reviu Articles Backlog

## Publishing Strategy

Publish the canonical article on `reviu.dev/blog` first, then use Medium as a distribution channel.

- Primary version: full article on the Reviu blog.
- Medium version: shorter adapted version, or imported full version with a canonical URL pointing to Reviu.
- Timing: publish on Reviu first, then repost on Medium 3 to 7 days later.
- Measurement: use UTM links from Medium back to the Reviu blog, download page, or pricing page.
- Language: write in English first. French versions can come later if there is a clear audience signal.

## Priority Articles

### 1. Why GitHub PR Review Still Feels Slow

- Goal: frame the problem Reviu solves without making the article feel like an ad.
- Audience: developers who review PRs every week and lose context between GitHub, their editor, and local Git.
- Angle: PR review is not slow because developers are careless; it is slow because the workflow is split across too many surfaces.
- Reviu tie-in: local Git, GitHub notifications, PR review, checks, comments, and merge context in one desktop app.
- Format: opinion + workflow breakdown.
- Status: Published draft on the Reviu blog.

### 2. A Local-First Workflow for Reviewing Large Pull Requests

- Goal: show a practical review process for large diffs.
- Audience: senior engineers, maintainers, and reviewers working on larger codebases.
- Angle: large PRs are easier to review when the diff, local branch, commits, and comments stay close together.
- Reviu tie-in: inline/split diffs, file navigation, local branch switching, pull request branch context, and keyboard navigation.
- Format: tactical guide.
- Status: Todo.

### 3. Git GUI vs CLI: Where Each One Actually Wins

- Goal: capture search traffic from developers comparing Git tools.
- Audience: developers who use the CLI but still want better visual review workflows.
- Angle: the CLI is great for precise commands, but visual review, staging, conflict resolution, and PR context deserve a focused interface.
- Reviu tie-in: Reviu complements Git knowledge instead of hiding Git.
- Format: comparison article.
- Status: Todo.

### 4. How to Stop Losing Context During Code Review

- Goal: connect emotional pain to concrete workflow changes.
- Audience: developers switching between IDE, browser, terminal, GitHub notifications, and desktop Git clients.
- Angle: context switching during review creates missed details, delayed replies, and slower merges.
- Reviu tie-in: GitHub notifications, saved PR lists, issue and repository browsing, browser extensions, and in-app review.
- Format: problem/solution article.
- Status: Todo.

### 5. Why Native Desktop Still Matters for Developer Tools

- Goal: position Reviu as a serious desktop app, not just another GitHub wrapper.
- Audience: developers skeptical of Electron-heavy tooling or slow browser workflows.
- Angle: native desktop tools can make review workflows faster, more reliable, and more keyboard-driven.
- Reviu tie-in: Rust + GPUI, cross-platform desktop app, keyboard-first workflow.
- Format: positioning article.
- Status: Todo.

## Secondary Articles

### Reviewing Pull Requests Without Living in the Browser

- Goal: speak directly to GitHub-heavy teams.
- Angle: GitHub is the source of truth, but the browser does not need to be the only review surface.
- Reviu tie-in: GitHub repository browsing, PR details, comments, checks, merge, issues, and browser extensions.
- Status: Todo.

### The Hidden Cost of Small Git Interruptions

- Goal: turn small local Git workflow annoyances into a larger productivity story.
- Angle: staging, restoring, rebasing, stashing, resolving conflicts, and jumping between hunks add up when the interface is not built for flow.
- Reviu tie-in: local Git workflows in Free.
- Status: Todo.

### What a Modern Git Client Should Do in 2026

- Goal: create a broader category-defining article.
- Angle: a modern Git client should handle local Git deeply and connect GitHub review workflows without becoming a browser clone.
- Reviu tie-in: full product overview, but written as criteria rather than a sales page.
- Status: Todo.

### How to Review Code Faster Without Rushing

- Goal: avoid speed-only messaging and emphasize quality.
- Angle: fast review means less friction, clearer context, and fewer repeated actions, not lower standards.
- Reviu tie-in: diff navigation, comments, checks, saved PR lists, keyboard shortcuts.
- Status: Todo.

### Building a Native Git Client with Rust and GPUI

- Goal: attract technical interest and founder/build-in-public audience.
- Angle: what it takes to build a cross-platform desktop Git client with a native UI stack.
- Reviu tie-in: engineering story, not the main conversion article.
- Status: Todo.

## Distribution Checklist

- Add canonical article to `landing/src/pages/blog/`.
- Add title, description, Open Graph image, canonical URL, and structured article metadata.
- Link from the landing page only when the article is strong enough to support conversion.
- Share adapted versions on Medium, Hacker News, Reddit, X, and relevant dev communities.
- Use direct calls to action: download Reviu, try Reviu Pro, or read the next workflow article.
- Track links with UTM parameters per channel.
