export interface ComparisonRow {
  id: string;
  label: string;
}

// Order controls table row order.
export const comparisonRows: ComparisonRow[] = [
  { id: "engine", label: "Rendering engine" },
  { id: "platforms", label: "Platforms" },
  { id: "price", label: "Pricing" },
  { id: "source", label: "Source available" },
  { id: "palette", label: "Keyboard-first command palette" },
  { id: "agent", label: "AI agent panel (review-to-agent)" },
  { id: "prReview", label: "In-app GitHub PR review" },
];

type RowId = (typeof comparisonRows)[number]["id"];

// Reviu column is the same on every comparison page.
export const reviuColumn: Record<RowId, string> = {
  engine: "Native Rust + GPUI, GPU-accelerated",
  platforms: "macOS, Windows, Linux",
  price: "Free local Git, Pro $9/mo or $79/yr",
  source: "Yes, FSL-1.1 (Apache-2.0 after 2 years)",
  palette: "Yes, every Git action",
  agent: "Yes - Claude Code, Codex, and 20+ ACP agents, free",
  prReview: "Yes, inline/split diff, comments, checks, merge (Pro)",
};

export interface Comparison {
  slug: string;
  name: string;
  title: string;
  metaDescription: string;
  intro: string[];
  columns: Record<RowId, string>;
  whenCompetitor: string[];
  whenReviu: string[];
  faq: { question: string; answer: string }[];
}

export const comparisons: Comparison[] = [
  {
    slug: "github-desktop",
    name: "GitHub Desktop",
    title: "GitHub Desktop Alternative",
    metaDescription:
      "Reviu vs GitHub Desktop: a native Rust Git client with keyboard-first workflows, an agent review panel, and in-app GitHub pull request review, comments, checks, and merge in Reviu Pro.",
    intro: [
      "GitHub Desktop is a free, open-source Electron app that makes it easy to clone a repo, commit, and open a pull request branch locally. It stays deliberately simple.",
      "Reviu is a native Rust desktop client built around the review loop: read the diff your agent wrote, comment inline, then review and merge GitHub pull requests in the same app with Reviu Pro. This page compares the two honestly so you can pick the right tool.",
    ],
    columns: {
      engine: "Electron (web tech)",
      platforms: "macOS, Windows (Linux via community fork)",
      price: "Free",
      source: "Yes, open source (MIT)",
      palette: "Limited",
      agent: "No",
      prReview: "Basic, opens the PR branch, no inline review",
    },
    whenCompetitor: [
      "You want a completely free, fully open-source app and only work with GitHub.",
      "You prefer the simplest possible interface and rarely touch advanced Git.",
    ],
    whenReviu: [
      "You want to review your agent's diff and GitHub PRs in a fast native app, not a webview.",
      "You need Linux support, keyboard-first commands, or deep in-app PR review with comments, checks, and merge.",
    ],
    faq: [
      {
        question: "Is Reviu free like GitHub Desktop?",
        answer:
          "Reviu is free for all local Git workflows and the built-in Claude or Codex agent panel, with no account required. GitHub notifications, repositories, issues, and pull request review are part of Reviu Pro ($9/month or $79/year).",
      },
      {
        question: "Does Reviu run on Linux?",
        answer:
          "Yes. Reviu ships official builds for macOS, Windows, and Linux. GitHub Desktop does not officially support Linux; Linux builds only exist through a community fork.",
      },
      {
        question: "Is Reviu built with Electron like GitHub Desktop?",
        answer:
          "No. GitHub Desktop is an Electron app. Reviu is a fully native desktop app built with Rust and GPUI, a GPU-accelerated UI framework, so there is no webview.",
      },
    ],
  },
  {
    slug: "fork",
    name: "Fork",
    title: "Fork Alternative",
    metaDescription:
      "Reviu vs Fork: both are native, fast Git clients. Reviu adds an agent review panel, keyboard-first commands, Linux support, and in-app GitHub pull request review in Reviu Pro.",
    intro: [
      "Fork is a fast, native Git client for macOS and Windows with a clean interface and a one-time purchase. It is a strong local Git tool that many developers like.",
      "Reviu is also native, built with Rust and GPUI, but focused on the review loop: read the diff your agent produced, comment inline, then review and merge GitHub pull requests inside the app with Reviu Pro.",
    ],
    columns: {
      engine: "Native (platform UI toolkits)",
      platforms: "macOS, Windows",
      price: "$59 one-time (1 user, up to 3 machines)",
      source: "No (proprietary)",
      palette: "Limited",
      agent: "No",
      prReview: "Limited",
    },
    whenCompetitor: [
      "You want a one-time purchase with no subscription.",
      "You are happy doing all GitHub pull request review in the browser.",
    ],
    whenReviu: [
      "You want GitHub notifications, pull request review, and merge inside the client, not the browser.",
      "You want an agent panel with review-to-agent on local diffs, or you need Linux support.",
    ],
    faq: [
      {
        question: "Is Reviu a one-time purchase like Fork?",
        answer:
          "No. Local Git in Reviu is free forever with no account. Reviu Pro, which adds GitHub notifications, pull request review, issues, and repository context, is a subscription at $9/month or $79/year with a 14-day free trial.",
      },
      {
        question: "Is Reviu native like Fork?",
        answer:
          "Yes. Both are native apps rather than Electron. Fork uses platform UI toolkits; Reviu is built with Rust and GPUI for a GPU-accelerated, keyboard-first interface.",
      },
      {
        question: "Does Reviu support Linux?",
        answer:
          "Yes. Reviu runs on macOS, Windows, and Linux. Fork is available for macOS and Windows only.",
      },
    ],
  },
  {
    slug: "tower",
    name: "Tower",
    title: "Tower Alternative",
    metaDescription:
      "Reviu vs Tower: Tower is a mature native Git client with power-user features. Reviu adds an agent review panel, source-available code, Linux support, and in-app GitHub PR review in Reviu Pro.",
    intro: [
      "Tower is a mature, native Git client for macOS and Windows with strong power-user features like drag-and-drop interactive rebase, unlimited undo, and a merge conflict wizard, sold as an annual subscription.",
      "Reviu is a native Rust client focused on the agent-to-GitHub review loop: read the diff, comment inline, send fixes back to the agent, then review and merge pull requests in Reviu Pro.",
    ],
    columns: {
      engine: "Native",
      platforms: "macOS, Windows",
      price: "Paid annual subscription",
      source: "No (proprietary)",
      palette: "Limited",
      agent: "No (has AI commit messages)",
      prReview: "Limited",
    },
    whenCompetitor: [
      "You want mature power-user Git features like drag-and-drop interactive rebase and unlimited undo.",
      "You review pull requests entirely in the browser and do not need an agent panel.",
    ],
    whenReviu: [
      "You want in-app GitHub pull request review, checks, and merge, plus an agent review loop.",
      "You want a source-available native Rust client, keyboard-first commands, or Linux support.",
    ],
    faq: [
      {
        question: "How does Reviu pricing compare to Tower?",
        answer:
          "Reviu is free for local Git and the agent panel. Reviu Pro adds GitHub workflows for $9/month or $79/year. Tower is a paid subscription billed annually.",
      },
      {
        question: "Does Reviu have an agent panel?",
        answer:
          "Yes. Reviu runs Claude or Codex from the sidebar so you can review the diff it produced and send inline comments back. Tower offers AI-generated commit messages but not a review-to-agent panel.",
      },
      {
        question: "Is Reviu source-available?",
        answer:
          "Yes. The Reviu desktop client is source-available on GitHub under FSL-1.1 and converts to Apache-2.0 two years after each release. Tower is proprietary.",
      },
    ],
  },
  {
    slug: "gitkraken",
    name: "GitKraken",
    title: "GitKraken Alternative",
    metaDescription:
      "Reviu vs GitKraken: GitKraken is an Electron DevEx platform. Reviu is a lean native Rust client with an agent review panel, keyboard-first commands, and in-app GitHub PR review in Reviu Pro.",
    intro: [
      "GitKraken Desktop is a cross-platform Electron Git client and part of a broader DevEx platform with boards, GitLens, and team features. It has a free tier and paid subscriptions.",
      "Reviu takes the opposite approach: a lean, native Rust client focused on the review loop, from your agent's diff to a merged GitHub pull request, without the weight of an Electron platform.",
    ],
    columns: {
      engine: "Electron (web tech)",
      platforms: "macOS, Windows, Linux",
      price: "Free tier + paid subscription",
      source: "No (proprietary)",
      palette: "Limited",
      agent: "No",
      prReview: "Yes, create and view PRs",
    },
    whenCompetitor: [
      "You want an all-in-one DevEx platform with boards, GitLens, and team collaboration features.",
      "You are invested in the GitKraken ecosystem across editor and CLI.",
    ],
    whenReviu: [
      "You want a focused, native, keyboard-first review surface instead of an Electron platform.",
      "You want to review your agent's code before pushing, with review-to-agent comments on local diffs.",
    ],
    faq: [
      {
        question: "Is Reviu lighter than GitKraken?",
        answer:
          "Reviu is a native Rust and GPUI app rather than an Electron app, so it avoids the memory footprint of a bundled browser runtime. It focuses on Git and GitHub review rather than a full DevEx platform.",
      },
      {
        question: "Does Reviu do GitHub pull request review?",
        answer:
          "Yes. Reviu Pro reviews pull requests in-app with inline and split diffs, review comments, replies, resolved threads, checks, merge readiness, and merge actions.",
      },
      {
        question: "Does Reviu run on Linux like GitKraken?",
        answer:
          "Yes. Both run on macOS, Windows, and Linux. Reviu adds a native Rust interface and a built-in Claude or Codex agent panel.",
      },
    ],
  },
];

export const getComparisonUrl = (slug: string) => `/vs/${slug}`;
