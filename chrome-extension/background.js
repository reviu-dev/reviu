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

chrome.action.onClicked.addListener((tab) => {
  if (tab.url && isSupportedGithubUrl(tab.url)) {
    chrome.tabs.update(tab.id, { url: buildReviuUrl(tab.url) });
  }
});

chrome.tabs.onUpdated.addListener((_tabId, changeInfo, tab) => {
  if (changeInfo.status === "complete" && tab.url) {
    const enabled = isSupportedGithubUrl(tab.url);
    if (enabled) {
      chrome.action.enable(tab.id);
    } else {
      chrome.action.disable(tab.id);
    }
  }
});

chrome.tabs.onActivated.addListener(async ({ tabId }) => {
  const tab = await chrome.tabs.get(tabId);
  if (tab.url && isSupportedGithubUrl(tab.url)) {
    chrome.action.enable(tabId);
  } else {
    chrome.action.disable(tabId);
  }
});
