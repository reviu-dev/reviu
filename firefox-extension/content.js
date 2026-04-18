const SUPPORTED_PATTERNS = [
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/pull\/\d+/,
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/issues\/\d+/,
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/pulls\/?/,
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/issues\/?/,
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/?$/,
];

const IGNORED_OWNERS = new Set([
  "apps",
  "marketplace",
  "settings",
  "notifications",
  "organizations",
  "orgs",
  "new",
  "topics",
  "trending",
  "collections",
  "events",
  "sponsors",
  "about",
  "pricing",
  "features",
  "security",
  "enterprise",
  "customer-stories",
  "team",
  "login",
  "logout",
  "join",
  "signup",
  "explore",
  "watching",
  "stars",
  "issues",
  "pulls",
  "codespaces",
  "discussions",
  "search",
  "dashboard",
  "account",
  "readme",
  "contact",
  "site",
  "pricing",
]);

function isSupportedGithubUrl(url) {
  try {
    const parsed = new URL(url);
    const firstSegment = parsed.pathname.split("/")[1];
    if (firstSegment && IGNORED_OWNERS.has(firstSegment.toLowerCase())) {
      return false;
    }
  } catch (_) {
    return false;
  }
  return SUPPORTED_PATTERNS.some((pattern) => pattern.test(url));
}

function buildReviuUrl(githubUrl) {
  return `reviu://open?url=${encodeURIComponent(githubUrl)}`;
}

let lastKnownUrl = "";

function syncButton() {
  const url = window.location.href;

  if (url === lastKnownUrl) return;
  lastKnownUrl = url;

  const existing = document.getElementById("reviu-open-btn");

  if (!isSupportedGithubUrl(url)) {
    if (existing) existing.remove();
    return;
  }

  if (existing) {
    existing.href = buildReviuUrl(url);
    return;
  }

  createReviuButton(url);
}

function createReviuButton(url) {
  const btn = document.createElement("a");
  btn.id = "reviu-open-btn";
  btn.href = buildReviuUrl(url);
  btn.title = "Open in Reviu";
  btn.setAttribute("aria-label", "Open in Reviu");

  btn.innerHTML = `<svg width="12" height="14" viewBox="0 0 315.6 361.1">
    <path fill="#2563eb" d="M225.5,272.1c52.7-20.4,90-71.5,90-131.3S252.5,0,174.8,0H0v361.1h315.6l-90-89Z"/>
  </svg>
  <span>Open in Reviu</span>`;

  // Insert into the page header actions area
  const headerActions = document.querySelector(
    ".gh-header-actions, .pagehead-actions"
  );
  if (headerActions) {
    const li = document.createElement("li");
    li.appendChild(btn);
    headerActions.prepend(li);
    return;
  }

  // Fallback: floating button
  btn.classList.add("reviu-floating");
  document.body.appendChild(btn);
}

// Run on initial load
syncButton();

// Re-run on SPA navigation (GitHub uses turbo/pjax)
const observer = new MutationObserver(() => syncButton());
observer.observe(document.body, { childList: true, subtree: true });
