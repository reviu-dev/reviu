const SUPPORTED_PATTERNS = [
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/pull\/\d+/,
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/issues\/\d+/,
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/pulls\/?/,
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/issues\/?/,
  /^https:\/\/github\.com\/[^/]+\/[^/]+\/?$/,
];

function isSupportedGithubUrl(url) {
  return SUPPORTED_PATTERNS.some((pattern) => pattern.test(url));
}

function buildReviuUrl(githubUrl) {
  return `reviu://open?url=${encodeURIComponent(githubUrl)}`;
}

browser.action.onClicked.addListener((tab) => {
  if (tab.url && isSupportedGithubUrl(tab.url)) {
    browser.tabs.update(tab.id, { url: buildReviuUrl(tab.url) });
  }
});

browser.tabs.onUpdated.addListener((_tabId, changeInfo, tab) => {
  if (changeInfo.status === "complete" && tab.url) {
    const enabled = isSupportedGithubUrl(tab.url);
    if (enabled) {
      browser.action.enable(tab.id);
    } else {
      browser.action.disable(tab.id);
    }
  }
});

browser.tabs.onActivated.addListener(async ({ tabId }) => {
  const tab = await browser.tabs.get(tabId);
  if (tab.url && isSupportedGithubUrl(tab.url)) {
    browser.action.enable(tabId);
  } else {
    browser.action.disable(tabId);
  }
});
