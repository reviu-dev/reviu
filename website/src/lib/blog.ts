export interface BlogPost {
  title: string;
  slug: string;
  description: string;
  publishedAt: string;
  readingTime: string;
}

export const blogPosts: BlogPost[] = [
  {
    title: "Why GitHub PR Review Still Feels Slow",
    slug: "why-github-pr-review-still-feels-slow",
    description:
      "Pull request review slows down when code, GitHub comments, checks, notifications, and merge state live in separate tools.",
    publishedAt: "2026-04-29",
    readingTime: "7 min read",
  },
];

export const getBlogPostUrl = (slug: string) => `/blog/${slug}`;
