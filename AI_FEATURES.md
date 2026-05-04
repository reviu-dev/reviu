# Reviu Pro AI Feature Ideas

This document is exploratory product guidance. These features are not current product claims until they are designed, implemented, verified, and reflected in the app, landing copy, terms, and support materials where needed.

## Product Fit

Reviu is already positioned around reducing review friction: local Git, GitHub notifications, pull request context, checks, comments, and merge actions in one native desktop app.

The most relevant AI features for Reviu Pro should support that promise. They should help developers understand, prioritize, resume, and write during review. They should not reposition Reviu as an autonomous code generator or generic chatbot.

Good AI framing:

- AI review context.
- AI-assisted triage.
- AI-assisted writing.
- AI summaries grounded in PR, diff, check, notification, and comment data.

Avoid broad claims like "AI code review" unless the product can make that specific behavior reliable and explainable.

## Recommended Pro Features

### AI PR Brief

Generate a short pull request brief before the reviewer opens the full diff.

Useful inputs:

- PR title and description.
- Changed files and diff stats.
- Commits.
- Review comments and unresolved threads.
- Checks and merge readiness.
- Labels, assignees, reviewers, draft state, and branch metadata.

Output should include:

- What changed.
- Files or areas to review first.
- Potential risk areas.
- Open questions or unresolved discussions.
- Check and merge blockers.

Landing angle:

"Start every pull request with the context that matters."

Why it fits:

Reviu already gathers the GitHub and diff context needed for a useful brief. This is a strong Pro feature because it makes the current PR page more valuable without replacing the reviewer.

### What Changed Since Last Review

Help reviewers resume a pull request without rereading everything.

Output should summarize:

- New commits since the last visit.
- Files changed since the last review pass.
- New comments and replies.
- Resolved and unresolved threads.
- Check status changes.
- Merge readiness changes.

Landing angle:

"Resume a review from what changed, not from the top of the diff."

Why it fits:

Reviu already has pull request commits, comments, checks, and navigation state. This is especially useful for large or long-running PRs.

### AI Review Queue Triage

Turn GitHub notifications and saved PR lists into an actionable review queue.

Useful groupings:

- Ready to review.
- Waiting on your reply.
- Blocked by failing checks.
- Waiting on author.
- Merge-ready.
- Draft or not ready.

Output should explain why a PR needs attention and summarize the latest important activity.

Landing angle:

"Turn a noisy GitHub inbox into a review queue you can act on."

Why it fits:

Reviu Pro already owns the GitHub home, notifications, saved PR lists, filters, and unread state. AI can make that surface more useful without changing the core workflow.

### Comment Composer Assistant

Assist with review comments, replies, and issue comments inside existing composers.

Useful actions:

- Rewrite for clarity.
- Make a comment more direct or more diplomatic.
- Turn feedback into a GitHub suggestion block.
- Draft a reply from the current thread and diff context.
- Summarize a long thread before replying.

Landing angle:

"Write clearer review comments without leaving the diff."

Why it fits:

Reviu already has PR and issue composers, Markdown previews, replies, suggestions, reactions, and image upload. This feature strengthens an existing workflow rather than adding a separate AI surface.

### PR Description Generator

Generate a pull request title and description when creating a PR from the Git page.

Useful inputs:

- Current branch name.
- Commit messages.
- Local diff or pushed branch diff.
- Pull request template.
- Linked issue references when present.

Output should include:

- Suggested title.
- Template-aware description.
- Summary of changes.
- Testing notes placeholder or detected test signal.
- Draft mode support.

Landing angle:

"Draft pull request descriptions from your branch, commits, and repository template."

Why it fits:

This bridges Free local Git workflow and Pro GitHub workflow. It is a natural upgrade moment when a local branch becomes a pull request.

### AI Suggested Review Checklist

Generate a focused review checklist for a pull request.

Examples:

- Review authentication or permission changes.
- Check database migration behavior.
- Verify empty, loading, and error states.
- Inspect tests around changed modules.
- Confirm feature flag or configuration behavior.

Landing angle:

"Get a focused review checklist before you dive into the diff."

Why it fits:

This keeps judgment with the human reviewer while using AI to direct attention to likely risk areas.

### Failed Check Explainer

Summarize failed checks from the pull request page.

Best version:

- Fetch CI logs where available.
- Identify failing test, lint, typecheck, build, or infrastructure failure.
- Link back to the relevant check job.
- Suggest where the reviewer should look next.

Fallback version:

- Summarize available check names, statuses, durations, provider images, and required status.

Landing angle:

"Understand failed checks from the PR page."

Why it fits:

Reviu already surfaces checks, required checks, skipped checks, provider images, and merge readiness. AI can make failures easier to scan.

## Best Initial Bundle

The strongest first Reviu Pro AI bundle would be:

1. AI PR Brief.
2. What Changed Since Last Review.
3. PR Description Generator.
4. Comment Composer Assistant.

Suggested product name:

"Reviu Pro AI Review Context"

This is more precise than "AI Review" and fits Reviu's current product promise: reduce fragmented review work without claiming to replace human judgment.

## Landing Copy Direction

Primary message:

"Review faster without reviewing less carefully."

Feature bullets:

- Summarize pull requests before you open the diff.
- Resume reviews from what changed since your last pass.
- Draft PR descriptions from local branches.
- Rewrite review comments and turn feedback into suggestions.

Short section copy:

"Reviu Pro AI Review Context helps you understand pull requests, resume long reviews, and write clearer comments from the same native app where you already review diffs, checks, threads, and merge readiness."

## Claims To Avoid

Avoid these until the product can support them reliably:

- "AI reviews your code."
- "AI approves pull requests."
- "AI fixes your PR automatically."
- "AI replaces reviewers."
- "AI catches every bug."
- "Autonomous code review."
- "Enterprise AI workflows."

Better alternatives:

- "AI-generated pull request briefs."
- "AI-assisted review context."
- "AI-assisted comment writing."
- "AI summaries grounded in your PR, checks, and threads."

## Implementation Notes

Likely backend responsibilities:

- Model/provider integration.
- Prompt construction with scoped GitHub and diff context.
- Subscription gating.
- Usage limits and rate limiting.
- Audit-friendly logging without storing sensitive prompts unnecessarily.
- Cache generated summaries per PR head SHA and conversation/check state.

Likely desktop responsibilities:

- Render briefs and changed-since summaries on the PR page.
- Add AI actions to PR, issue, and review composers.
- Add AI state, loading, retry, and error UI.
- Preserve keyboard-first access through command palette actions.
- Make generated text editable before posting.

Privacy and trust requirements:

- Clearly explain when repository, diff, comment, or check data is sent to an AI provider.
- Never post AI-generated text without explicit user confirmation.
- Keep source links and evidence visible where possible.
- Prefer grounded summaries over unsupported conclusions.
