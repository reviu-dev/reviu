# Desktop Shortcuts Plan

## Objective

Build a proper desktop shortcut architecture for Reviu that:

- keeps `cmd-k` as the command palette shortcut
- makes existing product shortcuts context-aware
- removes hardcoded shortcut labels from the UI
- adds a real desktop UI to inspect shortcuts
- keeps a clean path toward user-remappable shortcuts later

This plan is intentionally focused on product shortcuts, not default text-editing shortcuts inside editors and inputs.

## Explicit Decisions

- Keep `cmd-k` for the command palette.
- Do not scope this work to standard editor shortcuts like text selection, copy, paste, or cursor movement.
- Phase 1 is about architecture, context-aware dispatch, and shortcut discoverability.
- User remapping is a follow-up phase, but Phase 1 must not block it.

## Why This Needs a Proper Architecture

Current behavior works, but the shortcut system is still too flat:

- most bindings are registered globally in `desktop/crates/reviu/src/main.rs`
- page roots listen for actions, but they do not define explicit shortcut contexts
- several UI labels render hardcoded keystrokes instead of resolved bindings
- command availability is often contextual in behavior, but not in shortcut definition
- there is no central registry of product shortcuts, categories, or ownership

That makes three things harder than they should be:

- understanding what shortcuts Reviu actually supports
- adding new shortcuts without creating conflicts or drift
- shipping a shortcut settings UI later

## Current Shortcuts In Scope

Phase 1 should migrate the currently shipped product shortcuts first:

- `cmd-k`: command palette
- `cmd-p`: file search
- `cmd-o`: open repository
- `cmd-enter`: commit changes
- `cmd-w`: close workspace page
- `cmd-f`: find in file/diff where supported
- `escape`: close find where supported

These are the main current product-level bindings:

- `desktop/crates/reviu/src/main.rs`
- `desktop/crates/workspace/src/workspace.rs`
- `desktop/crates/workspace/src/git_page.rs`
- `desktop/crates/workspace/src/github_repo_page.rs`
- `desktop/crates/workspace/src/github_pr_details_page.rs`

## Phase 1 Deliverable

Ship a first shortcut system with:

- a central registry for desktop shortcuts
- explicit GPUI key contexts for workspace pages and sub-areas
- bindings registered from the central registry instead of inline scattered constants
- a dedicated read-only "Keyboard Shortcuts" UI in desktop settings
- all visible shortcut chips rendered from resolved bindings instead of hardcoded strings

Phase 1 is successful when:

- current shortcuts still work
- each shortcut only applies in the contexts where it makes sense
- the UI can show the effective binding for an action
- adding a new shortcut only requires touching the central registry plus the target action/page

## Proposed Architecture

### 1. Shortcut Registry

Add a central registry in the workspace crate, for example:

- `desktop/crates/workspace/src/shortcuts.rs`

The registry should define:

- `ShortcutId`
- `ShortcutCategory`
- `ShortcutDefinition`
- default keystroke
- owning action
- context predicate
- display metadata for UI

Suggested fields:

- `id`: stable internal identifier
- `title`: user-facing label
- `description`: short explanation
- `category`: Navigation, Search, Git, Workspace, etc.
- `keystroke`: default serialized keystroke string
- `context`: GPUI context predicate string
- `when_route`: optional workspace route scoping helper if needed

This registry becomes the source of truth for:

- GPUI key binding registration
- shortcut labels in buttons and empty states
- the future settings/remapping UI

### 2. Explicit Context Model

Introduce clear workspace shortcut contexts instead of relying on flat global bindings.

Suggested initial contexts:

- `Workspace`
- `WorkspaceGit`
- `WorkspaceGithubHome`
- `WorkspaceGithubRepo`
- `WorkspaceGithubRepoCode`
- `WorkspaceGithubPr`
- `WorkspaceGithubPrChanges`
- `WorkspaceSettings`
- `WorkspaceBilling`
- `WorkspaceAbout`
- `WorkspaceGitConfig`

Important rule:

- keep context names stable and product-oriented
- avoid overfitting them to implementation details

Examples:

- `cmd-k` should stay available at the workspace level
- `cmd-p` should only bind in `WorkspaceGit`, `WorkspaceGithubRepoCode`, and `WorkspaceGithubPrChanges`
- `cmd-enter` should only bind in `WorkspaceGit`
- `cmd-w` should only bind on closable secondary pages
- `cmd-f` and `escape` should only bind where an actual searchable editor/diff is active

### 3. Page-Level Context Wiring

Each top-level page should declare its key context on the tracked root element.

Examples of pages that should own explicit contexts:

- `GitPage`
- `GithubPage`
- `GithubRepoPage`
- `GithubPrDetailsPage`
- `SettingsPage`
- `BillingPage`
- `AboutPage`
- `GitConfigPage`

Sub-contexts should be added where the active tab changes shortcut meaning:

- GitHub repo page `Code` tab
- GitHub PR page `Changes` tab

This is the first step that turns current shortcuts into truly contextual shortcuts.

### 4. Central Binding Installation

Replace the current inline product shortcut bindings in `desktop/crates/reviu/src/main.rs` with a call that installs bindings from the registry.

Keep non-product editor/input bindings where they are for now.

Recommended split:

- editor/input bindings remain in app bootstrap
- product/workspace bindings come from the workspace shortcut registry

That separation will make the shortcut system easier to reason about.

### 5. Resolved Shortcut Labels

Current UI still hardcodes visible keystrokes in multiple places.

Phase 1 should replace those hardcoded strings with resolved bindings from GPUI, using the effective binding for the action in the relevant focus/context.

This matters for:

- top bar buttons
- empty states
- commit action buttons
- any tooltip or helper text that renders a shortcut label

If the app later supports remapping, these labels will already stay correct.

## Desktop UI Plan

Phase 1 should add a dedicated desktop UI for shortcut discovery.

Recommended shape:

- add a "Keyboard Shortcuts" section or page inside desktop settings
- searchable list
- grouped by category
- each row shows action name, description, and effective shortcut
- show context tags such as Git, Repo Code, PR Changes, Workspace

Phase 1 UI should be read-only.

That keeps the first implementation focused while still delivering a real user-facing shortcut surface.

Recommended minimum UX:

- search by action title
- search by keystroke text
- clear grouping by category
- shortcut chip rendered from the actual resolved binding

Optional but valuable in Phase 1:

- badge for "Current page only"
- badge for "Global"

## Phase 1 Implementation Order

### Step 1. Add the shortcut registry

- create `shortcuts.rs`
- define `ShortcutId`, metadata, categories, defaults
- add tests for registry completeness and uniqueness

### Step 2. Add explicit contexts to workspace pages

- wire `key_context(...)` on page roots
- add tab-specific context on GitHub repo code and PR changes surfaces
- keep focus routing stable
- add tests for context-aware availability where practical

### Step 3. Migrate current shortcuts into the registry

- `cmd-k`
- `cmd-p`
- `cmd-o`
- `cmd-enter`
- `cmd-w`
- `cmd-f`
- `escape`

At this step, behavior should remain functionally the same from a user perspective.

### Step 4. Replace hardcoded shortcut labels in the UI

- workspace top bar
- Git empty state
- commit button area
- any other visible shortcut hints found during migration

### Step 5. Add the "Keyboard Shortcuts" settings UI

- read-only list first
- hook it up to the central registry
- render resolved key text, not static strings

### Step 6. Add coverage

- tests for context gating
- tests for shortcut label rendering where already covered by existing UI tests
- tests ensuring registry defaults match rendered settings rows

## Follow-Up Phase: User Remapping

This is intentionally out of scope for the first implementation, but Phase 1 should prepare for it.

When remapping starts, the next layer should add:

- persisted user shortcut overrides
- conflict detection
- unbind support
- reset to default
- key-recording UI

Recommended persistence model for that later phase:

- a dedicated config table for shortcut overrides
- one row per shortcut override
- do not pack shortcut overrides into the current single-row `settings` table

## Important Constraint From Keeping `cmd-k`

Because Reviu keeps `cmd-k` as a single-stroke command palette shortcut, Phase 1 should not plan around `cmd-k`-prefixed chords.

That means:

- no `cmd-k cmd-s` editor-style shortcut entry point
- shortcut settings should be opened through Settings navigation, command palette, or another dedicated binding later

This avoids building an architecture around a chord family that conflicts with the product decision.

## Risks

### Context drift

If contexts are too generic, shortcuts will still feel global.

Mitigation:

- use explicit workspace/page names
- add tab-level contexts where behavior changes materially

### UI drift

If some labels still render hardcoded keystrokes, the system will immediately become inconsistent.

Mitigation:

- treat resolved shortcut labels as part of the Phase 1 definition of done

### Registry sprawl

If actions and shortcuts are modeled inconsistently, the registry will become hard to maintain.

Mitigation:

- keep one shortcut entry per user-facing action
- use stable IDs
- group by product category, not by file location

## Definition Of Done

Phase 1 is done when:

- current shipped product shortcuts come from one central registry
- current shortcuts are scoped by real GPUI contexts
- current visible shortcut labels are resolved dynamically
- desktop settings expose a real keyboard shortcuts UI
- the codebase has a clear extension point for adding more shortcuts later
