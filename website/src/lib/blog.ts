export interface BlogPost {
  title: string;
  slug: string;
  description: string;
  publishedAt: string;
  updatedAt?: string;
  readingTime: string;
}

export const blogPosts: BlogPost[] = [
  {
    title: "Introducing Reviu 1.0: The Review Workspace for Coding Agents",
    slug: "reviu-1-0",
    description:
      "Reviu started as a native Git client with an agent on the side. In 1.0, the agent session becomes the workspace.",
    publishedAt: "2026-09-15",
    readingTime: "7 min read",
  },
  {
    title: "Why GitHub PR Review Still Feels Slow",
    slug: "why-github-pr-review-still-feels-slow",
    description:
      "Pull request review slows down when code, GitHub comments, checks, notifications, and merge state live in separate tools.",
    publishedAt: "2026-04-29",
    updatedAt: "2026-09-15",
    readingTime: "7 min read",
  },
];

export const getBlogPostUrl = (slug: string) => `/blog/${slug}`;
