# Reviu – Specification Document

## Table of Contents
1. [Project Overview](#project-overview)
2. [Vision & Goals](#vision--goals)
3. [Technology Stack](#technology-stack)
4. [Architecture](#architecture)
5. [Authentication & Authorization](#authentication--authorization)
6. [Feature Roadmap](#feature-roadmap)
7. [Diff Engine](#diff-engine)
8. [Data Models](#data-models)
9. [API Design](#api-design)
10. [Security Considerations](#security-considerations)
11. [Open Questions](#open-questions)

---

## Project Overview

**Reviu** is a high-performance, keyboard-first desktop Git and GitHub PR client built for developers who demand speed, precision, and an IDE-like experience. It aims to be a premium alternative to GitHub Desktop with focus on:

- Ultra-fast diff rendering (GPU-accelerated)
- Efficient Pull Request management
- Local-first architecture
- Keyboard-driven workflow

**Target Audience:** Professional developers working with Git and GitHub who value performance and keyboard efficiency.

---

## Vision & Goals

### Core Principles
1. **Performance First** – Instant response times, GPU-accelerated rendering
2. **Keyboard-First UX** – Command palette, configurable shortcuts, minimal mouse usage
3. **Local-First** – All Git operations happen locally, no data sent to backend unnecessarily
4. **Respect User's Repos** – Non-destructive operations, safe defaults
5. **Modern SaaS Experience** – Seamless auth, subscription management for premium features

### Success Metrics
- Diff rendering: <50ms for files up to 10k lines
- Startup time: <500ms cold start
- Memory usage: <200MB idle
- Premium conversion: TBD (V2 metric)

---

## Technology Stack

### Desktop Application
- **Language:** Rust (edition 2024)
- **UI Framework:** GPUI (GPU-accelerated)
- **Git Backend:** `libgit2` via `git2` crate
- **HTTP Client:** `reqwest`
- **Local Storage:** SQLite via `rusqlite`
- **Secure Storage:** OS keychain via `keyring` crate
- **State Management:** GPUI native `Model<T>` + Context

**Why GPUI?**
- GPU-accelerated rendering for smooth 60fps+ scrolling
- Built-in text virtualization for large diffs
- Keyboard-first by design
- Modern Rust-native API

**Why libgit2?**
- Fast and efficient
- No subprocess overhead
- Direct programmatic access
- Mature and widely used

### Backend Service
- **Runtime:** Node.js 20+
- **Framework:** Hono
- **Authentication & Billing:** Better Auth 1.4+
- **Database:** PostgreSQL 18+
- **ORM:** Drizzle ORM
- **Logger:** Pino
- **Validation:** Zod
- **Deployment:** TBD (Docker-ready)

**Why Better Auth?**
- GitHub OAuth out of the box
- Session management
- Stripe billing integration built-in
- Secure by default
- TypeScript-native

---

## Architecture

### High-Level Overview

```
┌─────────────────────────────────────┐
│      Desktop App (Rust/GPUI)        │
│  ┌─────────────────────────────┐    │
│  │   UI Layer (GPUI)           │    │
│  └────────────┬────────────────┘    │
│               │                     │
│  ┌────────────▼────────────────┐    │
│  │   Application State         │    │
│  │   (Model<T> + Context)      │    │
│  └──┬──────────────────────┬───┘    │
│     │                      │         │
│  ┌──▼──────────┐  ┌────────▼─────┐  │
│  │  Git Engine │  │  API Client  │  │
│  │  (libgit2)  │  │  (HTTP/REST) │  │
│  └─────────────┘  └────────┬─────┘  │
│                            │         │
│  ┌─────────────────────────▼─────┐  │
│  │   Local Storage (SQLite)      │  │
│  └───────────────────────────────┘  │
└────────────────┬────────────────────┘
                 │ HTTPS/REST
                 │
┌────────────────▼────────────────────┐
│      Backend (Node.js/Hono)         │
│  ┌─────────────────────────────┐    │
│  │   Better Auth (OAuth)       │    │
│  │   GitHub OAuth Provider     │    │
│  └────────────┬────────────────┘    │
│               │                     │
│  ┌────────────▼────────────────┐    │
│  │   API Routes                │    │
│  │   - /api/auth/*             │    │
│  │   - /api/me                 │    │
│  │   - /api/github/* (V2)      │    │
│  └────────────┬────────────────┘    │
│               │                     │
│  ┌────────────▼────────────────┐    │
│  │   PostgreSQL (Drizzle)      │    │
│  │   - users                   │    │
│  │   - sessions                │    │
│  │   - subscriptions (V2)      │    │
│  └─────────────────────────────┘    │
└─────────────────────────────────────┘
```

### Desktop Application Architecture

#### State Management (Elm-style)

```rust
// Global application state
struct AppState {
    auth: AuthState,
    workspace: Workspace,
    ui: UIState,
    config: Config,
}

// Auth state
struct AuthState {
    token: Option<String>,
    user: Option<User>,
    premium: bool,
}

// Workspace (multi-repo support)
struct Workspace {
    repos: HashMap<PathBuf, Model<Repository>>,
    active_repo: Option<PathBuf>,
    recent_repos: Vec<PathBuf>,
}

// Repository state
struct Repository {
    path: PathBuf,
    git: GitRepository, // libgit2 wrapper
    status: GitStatus,
    diff: Option<DiffState>,
    selected_files: Vec<PathBuf>,
    staged_hunks: HashSet<HunkId>,
}

// UI state
struct UIState {
    command_palette_open: bool,
    active_panel: Panel,
    scroll_position: ScrollState,
}

// Actions/Messages
enum Action {
    // Repo operations
    LoadRepository(PathBuf),
    RefreshStatus,
    StageFile(PathBuf),
    UnstageFile(PathBuf),
    StageHunk(HunkId),
    Commit(String),
    Push,
    Pull,
    
    // Auth operations
    Login,
    Logout,
    RefreshPremiumStatus,
    
    // UI operations
    ToggleCommandPalette,
    SelectFile(PathBuf),
    ExpandContext(HunkId, usize),
}
```

#### Local Storage Schema (SQLite)

```sql
-- Auth tokens (encrypted)
CREATE TABLE auth (
    id INTEGER PRIMARY KEY,
    token TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    user_id TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- User preferences
CREATE TABLE preferences (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- Recent repositories
CREATE TABLE recent_repos (
    path TEXT PRIMARY KEY,
    last_opened_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    name TEXT,
    UNIQUE(path)
);

-- Feature flags cache (TTL: 5 minutes)
CREATE TABLE feature_flags (
    key TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL,
    cached_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- Window state
CREATE TABLE window_state (
    id INTEGER PRIMARY KEY CHECK (id = 1), -- Single row
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    x INTEGER,
    y INTEGER,
    maximized INTEGER NOT NULL DEFAULT 0
);
```

### Backend Architecture

#### Directory Structure

```
backend/
├── src/
│   ├── index.ts              # Entry point
│   ├── app.ts                # Hono app setup
│   ├── db/
│   │   ├── index.ts          # Database client
│   │   └── schema.ts         # Drizzle schema
│   ├── lib/
│   │   ├── auth.ts           # Better Auth config (includes Stripe)
│   │   ├── env.ts            # Environment validation
│   │   ├── logger.ts         # Pino logger
│   │   └── utils.ts          # Shared utilities
│   └── routes/
│       ├── me.ts             # GET /api/me
│       └── github.ts         # V2: GitHub integration
├── drizzle/                  # Generated migrations
├── drizzle.config.ts
├── docker-compose.yml
├── package.json
└── tsconfig.json
```

#### Database Schema (PostgreSQL + Drizzle)

```typescript
// V1 Schema (Better Auth tables)
export const user = pgTable('user', {
  id: text('id').primaryKey(),
  name: text('name').notNull(),
  email: text('email').notNull().unique(),
  emailVerified: boolean('email_verified').default(false).notNull(),
  image: text('image'),
  createdAt: timestamp('created_at').defaultNow().notNull(),
  updatedAt: timestamp('updated_at').defaultNow().notNull(),
  role: text('role'), // 'user' | 'admin'
  banned: boolean('banned').default(false),
  banReason: text('ban_reason'),
  banExpires: timestamp('ban_expires'),
})

export const session = pgTable('session', {
  id: text('id').primaryKey(),
  expiresAt: timestamp('expires_at').notNull(),
  token: text('token').notNull().unique(),
  createdAt: timestamp('created_at').defaultNow().notNull(),
  updatedAt: timestamp('updated_at').notNull(),
  ipAddress: text('ip_address'),
  userAgent: text('user_agent'),
  userId: text('user_id').notNull().references(() => user.id, { onDelete: 'cascade' }),
  impersonatedBy: text('impersonated_by'),
})

export const account = pgTable('account', {
  id: text('id').primaryKey(),
  accountId: text('account_id').notNull(), // GitHub user ID
  providerId: text('provider_id').notNull(), // 'github'
  userId: text('user_id').notNull().references(() => user.id, { onDelete: 'cascade' }),
  accessToken: text('access_token'),
  refreshToken: text('refresh_token'),
  idToken: text('id_token'),
  accessTokenExpiresAt: timestamp('access_token_expires_at'),
  refreshTokenExpiresAt: timestamp('refresh_token_expires_at'),
  scope: text('scope'),
  createdAt: timestamp('created_at').defaultNow().notNull(),
  updatedAt: timestamp('updated_at').notNull(),
})

// V2 Schema (GitHub)
// Note: Subscription data is managed by Better Auth's Stripe plugin
// Better Auth creates its own subscription tables automatically

export const githubInstallation = pgTable('github_installation', {
  id: text('id').primaryKey(),
  userId: text('user_id').notNull().references(() => user.id, { onDelete: 'cascade' }),
  installationId: text('installation_id').notNull().unique(),
  accountLogin: text('account_login').notNull(), // GitHub username/org
  accountType: text('account_type').notNull(), // 'User' | 'Organization'
  installedAt: timestamp('installed_at').notNull(),
  revokedAt: timestamp('revoked_at'),
})
```

---

## Authentication & Authorization

### V1: GitHub OAuth (Identity Only)

**Flow:**
1. User clicks "Login with GitHub" in desktop app
2. Desktop opens browser to `${BACKEND_URL}/api/auth/github/signin`
3. User authorizes Reviu (read:user scope)
4. Better Auth handles OAuth callback
5. Backend redirects to custom URL scheme: `reviu://auth?token=xxx`
6. Desktop intercepts URL, extracts token
7. Desktop stores token in OS keychain (via `keyring` crate)
8. Desktop calls `GET /api/me` to fetch user info

**Token Storage:**
- Desktop: OS keychain (`keyring` crate)
- Backend: PostgreSQL session table
- Token format: Better Auth session token (opaque string)
- Expiration: 7 days (Better Auth default)

### V2: GitHub App (Repository Access)

**Flow:**
1. User (already logged in) clicks "Connect GitHub Repos" in desktop
2. Desktop opens browser to GitHub App installation page
3. User selects repos and installs app
4. GitHub redirects to backend webhook
5. Backend stores installation in `github_installation` table
6. Desktop polls `GET /api/github/installations` to confirm
7. Desktop can now access GitHub features for installed repos

**Permissions (GitHub App):**
```yaml
# Minimum required permissions (to be refined)
permissions:
  contents: read         # Read repo contents for diffs
  pull_requests: write   # Create comments, approve, merge
  metadata: read         # Basic repo info
```

### Feature Gating

**Desktop checks premium status:**
```rust
// On startup and every 5 minutes
async fn refresh_premium_status(client: &ApiClient) -> Result<bool> {
    let response = client.get("/api/me").await?;
    let user: User = response.json().await?;
    Ok(user.premium)
}
```

**Backend `/api/me` response:**
```json
{
  "id": "user_123",
  "email": "user@example.com",
  "name": "John Doe",
  "premium": true,
  "subscription": {
    "status": "active",
    "plan": "pro",
    "currentPeriodEnd": "2024-12-31T23:59:59Z",
    "cancelAtPeriodEnd": false
  }
}
```

*Note: Subscription data comes from Better Auth's built-in Stripe plugin.*

**Feature flags:**
```rust
enum Feature {
    GitLocal,      // Always enabled
    GitHubPRs,     // Requires premium
    GitHubReviews, // Requires premium
    GitHubMerge,   // Requires premium
}

impl Feature {
    fn is_enabled(&self, user: &User) -> bool {
        match self {
            Feature::GitLocal => true,
            _ => user.premium,
        }
    }
}
```

---

## Feature Roadmap

### V1: Local Git (Free)

**Core Features:**
- [x] Repository detection and opening
- [x] File status detection (modified, staged, untracked)
- [x] Diff viewer (minimal context, expandable)
- [x] Stage/unstage files
- [x] Stage/unstage hunks (partial staging)
- [x] Commit with message
- [x] Push/pull
- [x] Command palette (Cmd+K / Ctrl+K)
- [x] Recent repositories list
- [x] Basic keyboard shortcuts

**Git Operations:**
- `git status` → File list
- `git diff` → Diff view
- `git add <file>` / `git add -p` → Stage operations
- `git commit -m` → Commit
- `git push` / `git pull` → Remote sync

**UI Components:**
- Repository selector (dropdown/command palette)
- File list (tree view)
- Diff viewer (side-by-side or unified)
- Commit message input
- Status bar

**Keyboard Shortcuts (Initial Set):**
```
Global:
  Cmd+K / Ctrl+K     : Open command palette
  Cmd+O / Ctrl+O     : Open repository
  Cmd+R / Ctrl+R     : Refresh status
  Cmd+, / Ctrl+,     : Open preferences

File List:
  ↑/↓                : Navigate files
  Space              : Stage/unstage file
  Enter              : View diff

Diff View:
  ↑/↓                : Navigate hunks
  Space              : Stage/unstage hunk
  E                  : Expand context
  Cmd+Enter / Ctrl+Enter : Commit

Commit:
  Cmd+Enter / Ctrl+Enter : Confirm commit
  Esc                : Cancel
```

### V2: GitHub Integration (Premium)

**Phase 1: PR Viewing**
- List PRs for connected repos
- View PR details (title, description, status)
- View PR diff (using same diff engine as local)
- Filter PRs (open, closed, mine, assigned)
- Search PRs

**Phase 2: PR Reviews**
- Add inline comments
- Reply to comments
- Approve / Request changes
- View review status

**Phase 3: PR Management**
- Create PR from branch
- Merge PR (merge, squash, rebase)
- Close/reopen PR
- Assign reviewers
- Add labels

**Phase 4: Notifications & Sync**
- Desktop notifications for PR events
- Real-time updates (webhooks → backend → desktop polling)
- Offline mode (cache PR data locally)

**V2 Deferred Items:**
- GitHub App installation flow
- Stripe billing integration
- Subscription management
- Webhooks handling
- Rate limiting
- Error monitoring (Sentry)
- Auto-update system
- Offline mode implementation details

---

## Diff Engine

### Core Philosophy
- **Git calculates, Reviu renders**
- Minimal diff by default (only changed lines)
- User can expand context on demand
- Same rendering engine for local Git and GitHub PRs
- GPU-accelerated text rendering via GPUI

### Diff Calculation (libgit2)

```rust
use git2::{Repository, Diff, DiffOptions};

struct DiffEngine {
    repo: Repository,
}

impl DiffEngine {
    /// Get diff for working directory vs index (unstaged changes)
    fn diff_workdir_to_index(&self) -> Result<Diff> {
        let mut opts = DiffOptions::new();
        opts.context_lines(0); // Minimal context
        opts.interhunk_lines(0);
        
        self.repo.diff_index_to_workdir(None, Some(&mut opts))
    }
    
    /// Get diff for index vs HEAD (staged changes)
    fn diff_index_to_head(&self) -> Result<Diff> {
        let mut opts = DiffOptions::new();
        opts.context_lines(0);
        
        let head = self.repo.head()?.peel_to_tree()?;
        self.repo.diff_tree_to_index(Some(&head), None, Some(&mut opts))
    }
}
```

### Diff Data Structure

```rust
struct FileDiff {
    path: PathBuf,
    old_path: Option<PathBuf>, // For renames
    status: FileStatus, // Added, Modified, Deleted, Renamed
    hunks: Vec<Hunk>,
}

struct Hunk {
    id: HunkId,
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
    lines: Vec<Line>,
    context_expanded: bool,
}

struct Line {
    origin: LineOrigin, // Context, Addition, Deletion
    content: String,
    old_lineno: Option<u32>,
    new_lineno: Option<u32>,
}

enum LineOrigin {
    Context,
    Addition,
    Deletion,
}

enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
}
```

### Context Expansion

**Default:** Show only changed lines (context_lines=0)

**User action:** Click "Expand 3 lines above/below" button

```rust
fn expand_context(hunk: &mut Hunk, direction: Direction, lines: usize) -> Result<()> {
    // Re-run git diff with larger context around this specific hunk
    let mut opts = DiffOptions::new();
    opts.context_lines(lines as u32);
    // ... re-fetch diff for this file and merge context
}
```

### Diff Rendering (GPUI)

```rust
impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_stack()
            .child(self.render_file_header())
            .children(
                self.hunks.iter().map(|hunk| {
                    self.render_hunk(hunk, cx)
                })
            )
    }
    
    fn render_hunk(&self, hunk: &Hunk, cx: &mut Context<Self>) -> impl IntoElement {
        v_stack()
            .child(self.render_hunk_header(hunk))
            .children(
                hunk.lines.iter().map(|line| {
                    self.render_line(line)
                })
            )
            .child_if(!hunk.context_expanded, || {
                button("Expand context...")
                    .on_click(|_| Action::ExpandContext(hunk.id, 3))
            })
    }
    
    fn render_line(&self, line: &Line) -> impl IntoElement {
        let bg_color = match line.origin {
            LineOrigin::Addition => rgb(0x1a3a1a), // Dark green
            LineOrigin::Deletion => rgb(0x3a1a1a), // Dark red
            LineOrigin::Context => rgb(0x2a2a2a),  // Neutral
        };
        
        h_stack()
            .bg(bg_color)
            .child(text(line.old_lineno.map(|n| n.to_string()).unwrap_or_default()))
            .child(text(line.new_lineno.map(|n| n.to_string()).unwrap_or_default()))
            .child(text(&line.content))
    }
}
```

### Performance Optimizations

1. **Virtualization:** GPUI's built-in list virtualization for large diffs
2. **Lazy Loading:** Load hunks on demand for very large files
3. **Caching:** Cache parsed diffs until next git operation
4. **Incremental Updates:** Only re-diff changed files

**Target Performance:**
- Files <1k lines: <10ms render
- Files 1k-10k lines: <50ms render
- Files >10k lines: Virtualized, lazy context loading

---

## Data Models

### Desktop Models (Rust)

```rust
// User identity
struct User {
    id: String,
    email: String,
    name: String,
    avatar_url: Option<String>,
    premium: bool,
}

// Git repository
struct GitRepository {
    path: PathBuf,
    name: String,
    head: Option<String>, // Current branch
    remote: Option<String>, // Origin URL
}

// File status
struct FileStatus {
    path: PathBuf,
    status: FileStatusKind,
    staged: bool,
}

enum FileStatusKind {
    Untracked,
    Modified,
    Added,
    Deleted,
    Renamed { from: PathBuf },
    Copied { from: PathBuf },
}

// Configuration
struct Config {
    theme: Theme,
    keybindings: HashMap<String, String>,
    editor: EditorConfig,
}

struct EditorConfig {
    tab_size: usize,
    font_family: String,
    font_size: f32,
    line_numbers: bool,
}

enum Theme {
    Light,
    Dark,
    System,
}
```

### Backend Models (TypeScript)

```typescript
// API Response types
interface MeResponse {
  id: string
  email: string
  name: string
  image?: string
  premium: boolean
  subscription?: {
    status: 'active' | 'cancelled' | 'past_due'
    plan: 'free' | 'pro'
    currentPeriodEnd?: string
  }
}

// V2: GitHub models
interface GitHubInstallation {
  id: string
  userId: string
  installationId: string
  accountLogin: string
  accountType: 'User' | 'Organization'
  installedAt: Date
  revokedAt?: Date
}

interface PullRequest {
  id: string
  number: number
  title: string
  body: string
  state: 'open' | 'closed' | 'merged'
  author: GitHubUser
  createdAt: Date
  updatedAt: Date
  headRef: string
  baseRef: string
  repoFullName: string
}

interface GitHubUser {
  login: string
  avatarUrl: string
}
```

---

## API Design

### Base URL
```
Development: http://localhost:3000
Production: https://api.reviu.app (TBD)
```

### Authentication
All authenticated requests must include session token:
```
Cookie: better-auth.session_token=<token>
```

### Endpoints

#### V1 Endpoints

**POST /api/auth/github/signin**
```
Description: Initiate GitHub OAuth flow
Auth: None
Response: Redirect to GitHub OAuth
```

**GET /api/auth/github/callback**
```
Description: OAuth callback handler
Auth: None (handled by Better Auth)
Response: Redirect to reviu://auth?token=xxx
```

**GET /api/me**
```
Description: Get current user info and premium status
Auth: Required
Response: MeResponse
Example:
{
  "id": "user_123",
  "email": "user@example.com",
  "name": "John Doe",
  "image": "https://avatars.githubusercontent.com/u/123",
  "premium": false,
  "subscription": null
}
```

**POST /api/auth/signout**
```
Description: Sign out (invalidate session)
Auth: Required
Response: { "success": true }
```

#### V2 Endpoints (GitHub)

**GET /api/github/installations**
```
Description: List user's GitHub App installations
Auth: Required + Premium
Response: GitHubInstallation[]
```

**GET /api/github/repos**
```
Description: List accessible repositories
Auth: Required + Premium
Query: ?installation_id=xxx
Response: Repository[]
```

**GET /api/github/repos/:owner/:repo/pulls**
```
Description: List pull requests
Auth: Required + Premium
Query: ?state=open&per_page=30&page=1
Response: PullRequest[]
```

**GET /api/github/repos/:owner/:repo/pulls/:number**
```
Description: Get PR details
Auth: Required + Premium
Response: PullRequest
```

**GET /api/github/repos/:owner/:repo/pulls/:number/files**
```
Description: Get PR file changes
Auth: Required + Premium
Response: FileDiff[]
```

**POST /api/github/repos/:owner/:repo/pulls/:number/reviews**
```
Description: Submit PR review
Auth: Required + Premium
Body: { event: 'APPROVE' | 'REQUEST_CHANGES' | 'COMMENT', body?: string }
Response: Review
```

#### V2 Endpoints (Billing)

*Note: Billing endpoints are provided by Better Auth's Stripe plugin. Routes are automatically mounted at `/api/auth/billing/*`*

**Better Auth Stripe Routes (auto-configured):**
- `POST /api/auth/billing/create-checkout` - Create Stripe checkout session
- `POST /api/auth/billing/create-portal` - Create customer portal session
- `POST /api/auth/billing/webhook` - Stripe webhook handler (auto-configured)

See Better Auth Stripe plugin documentation for details.

### Error Responses

```typescript
interface ErrorResponse {
  error: {
    code: string
    message: string
    details?: unknown
  }
}

// Standard error codes
const ErrorCodes = {
  UNAUTHORIZED: 'unauthorized',
  FORBIDDEN: 'forbidden',
  NOT_FOUND: 'not_found',
  INVALID_REQUEST: 'invalid_request',
  RATE_LIMITED: 'rate_limited',
  INTERNAL_ERROR: 'internal_error',
  GITHUB_ERROR: 'github_error',
} as const
```

**Example:**
```json
{
  "error": {
    "code": "forbidden",
    "message": "This feature requires a premium subscription"
  }
}
```

---

## Security Considerations

### Desktop Security

**Token Storage:**
- Use OS keychain via `keyring` crate
- Never store tokens in plain text files
- Clear keychain on signout

**Git Credentials:**
- Delegate to system Git credential helper
- Never intercept or store user's Git credentials
- Respect `.git/config` credential settings

**Local Database:**
- Encrypt sensitive preferences if needed
- Use prepared statements (SQLite) to prevent injection
- Regular cleanup of expired cache

**Network Security:**
- Pin TLS certificates for backend API
- Validate all API responses
- Timeout all HTTP requests (30s default)

### Backend Security

**Authentication:**
- Session tokens: 7-day expiration
- HTTPOnly, Secure, SameSite=Lax cookies
- CSRF protection (Better Auth built-in)
- Rate limiting on auth endpoints (V2)

**GitHub Integration:**
- Short-lived installation tokens (1 hour)
- Minimal permissions (principle of least privilege)
- Webhook signature validation
- Store GitHub tokens encrypted at rest

**Database:**
- Parameterized queries (Drizzle ORM)
- Connection pooling with limits
- Regular backups (V2)
- Encryption at rest (V2)

**API:**
- Input validation (Zod schemas)
- Output sanitization
- CORS: whitelist desktop app origin
- Rate limiting per user (V2)
- Request size limits (1MB default)

**Billing (V2):**
- Better Auth handles Stripe integration
- Webhook signature validation (built-in)
- Idempotent payment processing (built-in)
- Transaction isolation
- Audit logs for financial operations (via Better Auth)

---

## Open Questions

### Technical Decisions (V2)

1. **Auto-Update Strategy**
   - Options: Tauri updater, custom solution, manual downloads
   - Decision deferred until V2

2. **Offline Mode Implementation**
   - How much GitHub data to cache locally?
   - Sync strategy when coming back online?
   - Conflict resolution for local vs remote changes?

3. **Rate Limiting (GitHub API)**
   - Per-user quotas?
   - Queue system for background sync?
   - Graceful degradation when rate limited?

4. **Notifications**
   - WebSocket for real-time updates?
   - Polling with exponential backoff?
   - Native OS notifications or in-app only?

5. **Large Repository Handling**
   - Repos with 10k+ files?
   - Monorepos?
   - File size limits for diff?

### Product Decisions (V2)

1. **Pricing**
   - Free tier: Local Git only
   - Pro tier: GitHub features
   - Price point: $5/mo? $10/mo? Annual discount?
   - Trial period: 14 days? 30 days?

2. **GitHub Permissions**
   - Fine-tune required permissions
   - Per-repo vs org-wide installation?
   - Read-only mode for public repos?

3. **Supported Platforms**
   - V1: macOS only?
   - V2: Windows, Linux?
   - System requirements?

4. **Telemetry & Analytics**
   - Error reporting (Sentry)?
   - Usage analytics?
   - User consent & privacy?

### UX Decisions

1. **Diff Display**
   - Side-by-side vs unified view?
   - Syntax highlighting?
   - Word-level diff?

2. **Commit Message Editor**
   - Rich text or plain text?
   - Templates?
   - Emoji picker?

3. **Theme Customization**
   - Built-in themes only?
   - Custom theme support?
   - VS Code theme compatibility?

4. **Keyboard Shortcuts**
   - Vim mode?
   - Emacs mode?
   - Custom keybinding editor?

---

## Development Phases

### Phase 0: Foundation (Current)
- [x] Initialize monorepo structure
- [x] Backend: Hono + Better Auth + Drizzle
- [x] Desktop: GPUI hello world
- [x] Docker Compose for local PostgreSQL
- [x] Spec document (this file)

### Phase 1: Local Git Core (2-3 weeks)
- [ ] libgit2 integration
- [ ] Repository detection and opening
- [ ] File status tracking (working dir + index)
- [ ] Basic diff viewer (unified, minimal context)
- [ ] Stage/unstage files
- [ ] Commit functionality
- [ ] Push/pull operations
- [ ] SQLite local storage
- [ ] Basic UI layout (file list + diff view)

### Phase 2: UX & Polish (1-2 weeks)
- [ ] Command palette
- [ ] Keyboard shortcuts
- [ ] Recent repositories list
- [ ] Stage hunks (partial staging)
- [ ] Context expansion in diff
- [ ] Commit message validation
- [ ] Error handling & user feedback
- [ ] Preferences dialog

### Phase 3: Auth & Backend Integration (1 week)
- [ ] GitHub OAuth flow in desktop
- [ ] Token storage in keychain
- [ ] `/api/me` implementation
- [ ] Session refresh logic
- [ ] Premium status caching

### Phase 4: GitHub Integration (V2, 3-4 weeks)
- [ ] GitHub App creation & setup
- [ ] Installation flow
- [ ] List repos endpoint
- [ ] List PRs endpoint
- [ ] PR detail view
- [ ] PR diff view (reuse local diff engine)
- [ ] Backend webhook handlers
- [ ] Desktop polling for updates

### Phase 5: PR Reviews (V2, 2-3 weeks)
- [ ] Inline comments
- [ ] Review submission (approve/request changes)
- [ ] Comment threads
- [ ] Notifications
- [ ] Merge functionality

### Phase 6: Billing & Launch (V2, 2 weeks)
- [ ] Better Auth Stripe plugin setup
- [ ] Stripe webhook configuration
- [ ] Subscription management UI in desktop
- [ ] Checkout flow in desktop (redirect to Better Auth routes)
- [ ] Feature gating enforcement
- [ ] Marketing site
- [ ] Public beta launch

---

## Success Criteria

### V1 (Local Git)
- App starts in <500ms
- Diff rendering <50ms for typical files
- No data loss (commits, staging)
- Works offline
- Memory usage <200MB idle
- 10+ beta users providing feedback

### V2 (GitHub)
- GitHub App installation <2 minutes
- PR list loads in <1s
- PR diff loads in <1s
- Review submission <500ms
- 100+ paying users (target TBD)
- 95%+ uptime
- <1% error rate

---

## Appendix

### Useful Resources
- GPUI Docs: https://www.gpui.rs/
- libgit2 Rust bindings: https://docs.rs/git2/
- Better Auth: https://www.better-auth.com/
- Better Auth Stripe Plugin: https://www.better-auth.com/docs/plugins/stripe
- Drizzle ORM: https://orm.drizzle.team/docs/overview
- GitHub App docs: https://docs.github.com/en/apps

### Similar Projects (for inspiration)
- GitHub Desktop (Electron + TypeScript)
- GitKraken (Electron)
- Tower (native macOS, commercial)
- Fork (native, commercial)
- Sublime Merge (native, commercial)

### Competitive Advantages
1. **Performance:** GPU-accelerated, native Rust (vs Electron, Tauri)
2. **Keyboard-First:** Power users, IDE-like shortcuts
3. **Modern Stack:** Built for 2025+ workflows
4. **Premium PR Features:** Better than GitHub Desktop
5. **Local-First:** Privacy, speed, offline support

---

**Document Version:** 1.0
**Last Updated:** 2025-12-22
**Status:** Draft → Ready for Implementation
