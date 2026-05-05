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

## Packaging Direction

The preferred packaging model is a three-tier plan structure:

### Free

Price:

- `$0`

Includes:

- Local Git workflows.
- No GitHub integration.
- No AI features.

### Pro

Price:

- `$9/month` or `$79/year`

Includes:

- GitHub integration.
- Notifications, repositories, pull requests, issues, checks, comments, merge actions, and browser extensions.
- Bring-your-own-key AI support for users who connect their own OpenAI, Anthropic, or compatible provider account.

Notes:

- In this tier, AI provider usage is billed to the user's provider account.
- Reviu should still provide the product UI, prompt orchestration, schema validation, and safe posting controls.
- This keeps current Pro pricing intact while giving advanced users access to AI features.

### Pro AI

Price:

- `$19/month` or `$149/year`

Includes:

- Everything in Pro.
- Reviu-managed AI included.
- Higher monthly AI quota.
- No provider API key required for standard AI actions.

Notes:

- This tier is the cleanest landing-page story for AI because users can try AI features without setting up an external API key.
- The plan needs quota, rate limit, abuse protection, provider kill switches, and usage visibility before launch.
- AI costs should be measured in beta before finalizing included monthly quota.

## AI Provider Strategy

The app should support both user-owned and Reviu-managed AI, but they should be treated as different credential modes.

Credential modes:

- `user_key`: the user provides an OpenAI, Anthropic, or compatible API key.
- `reviu_managed`: Reviu pays the provider and gates usage through Pro AI quotas.
- `local`: the desktop app talks to a local OpenAI-compatible endpoint such as Ollama or LM Studio.

Recommended rollout:

1. Build the AI abstraction around provider + credential mode from the start.
2. Ship Pro with `user_key` support first if cost risk needs to stay low.
3. Add Pro AI with `reviu_managed` once usage and cost per action are measured.
4. Add `local` as an advanced mode for users who want local models and accept quality/performance tradeoffs.

For server-side GitHub AI features like AI PR Brief, the desktop should send only the PR identity and action request. The backend should fetch canonical PR context, build the prompt, call the selected provider, validate structured output, cache results, and return UI-ready JSON.

For local-model mode, the backend can expose canonical AI context while the desktop performs the local provider call, because a cloud backend cannot call the user's `localhost`.

## V1 Implementation Status

The first implementation is focused on AI PR Brief with BYOK credentials and an architecture that can later support Reviu-managed AI.

Implemented backend foundation:

- Pro-gated AI routes under `/ai`.
- `GET /ai/settings`, `PUT /ai/settings`, and `DELETE /ai/settings`.
- `POST /ai/github/pr/brief`.
- BYOK credential mode: `user_key`.
- Providers: OpenAI and Anthropic.
- Provider abstraction built on the AI SDK package.
- Encrypted API key storage using `AI_CREDENTIALS_SECRET`.
- No persisted API key hint. The backend derives `apiKeyHint` from the decrypted key when returning settings.
- PR brief cache keyed by user, repo, PR number, head SHA, and context hash.
- Usage events for generated briefs with provider, model, credential mode, token counts, and PR identity.
- Structured model output validation before returning data to the client.

Implemented desktop foundation:

- AI settings page for BYOK provider, model, and API key.
- API key input is masked and cleared after saving.
- Saved key display uses `apiKeyHint` only.
- PR Overview AI Brief panel with generate/refresh state.
- Loading skeleton for first-generation state.
- Brief sections: summary, review-first files, risks, and blockers.
- Structured file targets open the PR Changes tab and select the referenced file.

Current V1 behavior:

- The desktop sends only PR identity and `forceRefresh`.
- The backend fetches canonical PR details, files, commits, conversation, checks, and merge readiness.
- The backend builds provider prompts, calls the selected model, validates JSON output, resolves file targets, caches the brief, and returns UI-ready structured JSON.
- The response is not GFM. Links are structured targets so Reviu can navigate internally without parsing markdown links.

Intentionally deferred:

- Reviu-managed AI and Pro AI quotas.
- Local LLM mode.
- Command palette AI actions.
- Changed-since-last-review.
- Comment composer assistant.
- PR description generation.
- Usage/quota UI.
- Provider cost dashboards and kill switches.

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

- Model/provider integration. V1 supports OpenAI and Anthropic through a shared provider layer.
- Prompt construction with scoped GitHub and diff context. V1 PR Brief context is built server-side from canonical GitHub data.
- Subscription gating. V1 AI routes are Pro-gated.
- Usage limits and rate limiting. Still needed before Reviu-managed AI.
- Audit-friendly logging without storing sensitive prompts unnecessarily.
- Cache generated summaries per PR head SHA and conversation/check state. V1 uses PR head SHA plus a context hash.
- Encrypted BYOK credential storage. V1 stores only the encrypted key, not a persisted key hint.

Likely desktop responsibilities:

- Render briefs and changed-since summaries on the PR page. V1 renders AI PR Brief in the PR Overview tab.
- Add AI actions to PR, issue, and review composers.
- Add AI state, loading, retry, and error UI. V1 includes settings state, PR brief loading skeleton, refresh, and error copy.
- Preserve keyboard-first access through command palette actions.
- Make generated text editable before posting.

Privacy and trust requirements:

- Clearly explain when repository, diff, comment, or check data is sent to an AI provider.
- Never post AI-generated text without explicit user confirmation.
- Keep source links and evidence visible where possible.
- Prefer grounded summaries over unsupported conclusions.
