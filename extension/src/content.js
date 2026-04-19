// GitHub Primer button inner structure. GitHub's buttons (class
// `prc-Button-ButtonBase-*`) expect this nested layout to render the
// icon + label correctly. Used when a config sets `className` so the
// Reviu button visually matches surrounding GitHub buttons.
const GITHUB_BUTTON_INNER = `
  <span data-component="buttonContent" data-align="center" class="prc-Button-ButtonContent-Iohp5">
    <span data-component="leadingVisual" class="prc-Button-Visual-YNt2F prc-Button-VisualWrap-E4cnq">
      <svg width="16" height="16" viewBox="0 0 315.6 361.1" aria-hidden="true" focusable="false">
        <path fill="#2563eb" d="M225.5,272.1c52.7-20.4,90-71.5,90-131.3S252.5,0,174.8,0H0v361.1h315.6l-90-89Z"/>
      </svg>
    </span>
    <span data-component="text" class="prc-Button-Label-FWkx3">Open in Reviu</span>
  </span>
`;

// Plain inner HTML for the fallback (floating) button, which relies on our
// own `content.css` baseline styling.
const DEFAULT_BUTTON_INNER = `
  <svg width="12" height="14" viewBox="0 0 315.6 361.1">
    <path fill="#2563eb" d="M225.5,272.1c52.7-20.4,90-71.5,90-131.3S252.5,0,174.8,0H0v361.1h315.6l-90-89Z"/>
  </svg>
  <span>Open in Reviu</span>
`;

// Each entry matches one GitHub page type and describes how to mount the
// button in that page's DOM.
//   - `pattern`: URL regex the entry applies to.
//   - `selector`: where the button is inserted. If null or the element is
//     missing, the button falls back to a floating overlay on <body>.
//   - `wrap`: optional tag to wrap the button in before insertion (e.g. "li"
//     for a <ul> host).
//   - `position`: "prepend" (default) or "append" inside the host.
//   - `className`: space-separated class list applied to the button. When
//     set, the button renders GitHub's Primer button structure so it inherits
//     their styling. When absent, our baseline `content.css` styles apply.
//   - `dataSize`: value for the Primer `data-size` attribute ("small",
//     "medium", "large"). Defaults to "medium". Only used with `className`.
const PAGE_CONFIGS = [
  {
    name: "pull-details",
    pattern: /^https:\/\/github\.com\/[^/]+\/[^/]+\/pull\/\d+/,
    selector: "[data-component='PH_Actions'] > div.d-flex.gap-1",
    position: "prepend",
    className: "prc-Button-ButtonBase-9n-Xk",
  },
  {
    name: "issue-details",
    pattern: /^https:\/\/github\.com\/[^/]+\/[^/]+\/issues\/\d+/,
    selector:
      "[data-component='PH_Actions'] [class*='HeaderMenu-module__menuActionsContainer']",
    position: "prepend",
    className: "prc-Button-ButtonBase-9n-Xk",
  },
  {
    name: "commit-details",
    pattern: /^https:\/\/github\.com\/[^/]+\/[^/]+\/commit\/[0-9a-f]+/i,
    selector: "[data-component='PH_Actions'] > div.d-flex",
    position: "prepend",
    className:
      "prc-Button-ButtonBase-9n-Xk CommitHeader-module__browseFilesButton__nELIN prc-Link-Link-9ZwDx",
  },
  {
    name: "pull-list",
    pattern: /^https:\/\/github\.com\/[^/]+\/[^/]+\/pulls\/?/,
    selector: null,
  },
  {
    name: "issue-list",
    pattern: /^https:\/\/github\.com\/[^/]+\/[^/]+\/issues\/?/,
    selector: null,
  },
  {
    name: "repo",
    pattern: /^https:\/\/github\.com\/[^/]+\/[^/]+\/?$/,
    selector: ".gh-header-actions, .pagehead-actions",
    wrap: "li",
    className:
      "prc-Button-ButtonBase-9n-Xk NotificationsSubscriptionsMenu-module__ActionMenuButton__FVE3w",
    dataSize: "small",
  },
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

function pageConfigForUrl(url) {
  try {
    const parsed = new URL(url);
    const firstSegment = parsed.pathname.split("/")[1];
    if (firstSegment && IGNORED_OWNERS.has(firstSegment.toLowerCase())) {
      return null;
    }
  } catch (_) {
    return null;
  }
  return PAGE_CONFIGS.find((config) => config.pattern.test(url)) ?? null;
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
  const config = pageConfigForUrl(url);

  if (!config) {
    if (existing) existing.remove();
    return;
  }

  if (existing) {
    existing.href = buildReviuUrl(url);
    return;
  }

  createReviuButton(url, config);
}

function createReviuButton(url, config) {
  const btn = document.createElement("a");
  btn.id = "reviu-open-btn";
  btn.href = buildReviuUrl(url);
  btn.title = "Open in Reviu";
  btn.setAttribute("aria-label", "Open in Reviu");

  if (config.className) {
    btn.className = config.className;
    // GitHub Primer buttons look at these data attributes for sizing/variant.
    btn.setAttribute("type", "button");
    btn.setAttribute("data-size", config.dataSize ?? "medium");
    btn.setAttribute("data-variant", "default");
    btn.setAttribute("data-loading", "false");
    btn.innerHTML = GITHUB_BUTTON_INNER;
  } else {
    btn.classList.add("reviu-default-style");
    btn.innerHTML = DEFAULT_BUTTON_INNER;
  }

  const host = config.selector ? document.querySelector(config.selector) : null;
  if (host) {
    let node = btn;
    if (config.wrap) {
      const wrapper = document.createElement(config.wrap);
      wrapper.appendChild(btn);
      node = wrapper;
    }
    if (config.position === "append") {
      host.appendChild(node);
    } else {
      host.prepend(node);
    }
    return;
  }

  // Fallback: floating button (always uses our baseline style).
  btn.className = "reviu-default-style reviu-floating";
  btn.innerHTML = DEFAULT_BUTTON_INNER;
  document.body.appendChild(btn);
}

// Run on initial load
syncButton();

// Re-run on SPA navigation (GitHub uses turbo/pjax)
const observer = new MutationObserver(() => syncButton());
observer.observe(document.body, { childList: true, subtree: true });
