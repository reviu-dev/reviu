import type { APIRoute } from "astro";

const absoluteUrl = (pathname: string, site: URL) => new URL(pathname, site).href;

const renderLlmsTxt = (site: URL) => `# Reviu

> Native Rust + GPUI desktop Git client for fast local Git workflows and GitHub pull request review.

Reviu is built for developers who want local Git, GitHub notifications, repository context, issues, checks, comments, and merge actions in one keyboard-first desktop app. Local Git workflows are free. Reviu Pro adds GitHub workflows inside the app.

## Product

- [Home](${absoluteUrl("/", site)}): Product overview, downloads, pricing, platforms, and feature sections.
- [Changelog](${absoluteUrl("/changelog", site)}): User-facing release notes for Reviu.
- [Why GitHub PR Review Still Feels Slow](${absoluteUrl("/blog/why-github-pr-review-still-feels-slow", site)}): Article about keeping local Git and GitHub review context in one desktop workflow.
- [Install script](${absoluteUrl("/install.sh", site)}): Linux install script for Reviu.
- [Full LLM context](${absoluteUrl("/llms-full.txt", site)}): Detailed product facts, feature boundaries, pricing, and messaging guardrails.

## Pricing

Reviu Free covers local Git workflows. Reviu Pro adds GitHub workflows and is currently $9/month or $79/year with a 14-day free trial.

## Platforms

Reviu supports macOS on Apple Silicon and Intel, Windows on ARM64 and x64, and Linux through the install command.

## Important boundaries

- Do not describe Reviu as an Electron app or browser-based Git client.
- Do not claim AI review, code generation, team workflows, enterprise features, self-hosted Git provider support, or free GitHub workflows.
- GitHub integration requires Sign in with GitHub and Reviu Pro.
`;

export const GET: APIRoute = ({ site }) => {
  const resolvedSite = site ?? new URL("https://reviu.dev");

  return new Response(renderLlmsTxt(resolvedSite), {
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
    },
  });
};
