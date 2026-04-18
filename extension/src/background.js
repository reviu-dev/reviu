const api = globalThis.browser ?? globalThis.chrome;

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

api.action.onClicked.addListener((tab) => {
  if (tab.url && isSupportedGithubUrl(tab.url)) {
    api.tabs.update(tab.id, { url: buildReviuUrl(tab.url) });
  }
});

api.tabs.onUpdated.addListener((_tabId, changeInfo, tab) => {
  if (changeInfo.status === "complete" && tab.url) {
    const enabled = isSupportedGithubUrl(tab.url);
    if (enabled) {
      api.action.enable(tab.id);
    } else {
      api.action.disable(tab.id);
    }
  }
});

api.tabs.onActivated.addListener(async ({ tabId }) => {
  const tab = await api.tabs.get(tabId);
  if (tab.url && isSupportedGithubUrl(tab.url)) {
    api.action.enable(tabId);
  } else {
    api.action.disable(tabId);
  }
});
