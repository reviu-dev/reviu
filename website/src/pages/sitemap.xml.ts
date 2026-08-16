import type { APIRoute } from "astro";
import { blogPosts, getBlogPostUrl } from "../lib/blog";
import { comparisons, getComparisonUrl } from "../lib/comparisons";

const buildDate = new Date().toISOString().slice(0, 10);

interface SitemapUrl {
  path: string;
  lastmod: string;
}

const staticUrls: SitemapUrl[] = [
  { path: "/", lastmod: buildDate },
  { path: "/vs", lastmod: buildDate },
  { path: "/blog", lastmod: buildDate },
  { path: "/changelog", lastmod: buildDate },
  { path: "/privacy", lastmod: buildDate },
  { path: "/terms", lastmod: buildDate },
];

const comparisonUrls: SitemapUrl[] = comparisons.map((comparison) => ({
  path: getComparisonUrl(comparison.slug),
  lastmod: buildDate,
}));

const postUrls: SitemapUrl[] = blogPosts.map((post) => ({
  path: getBlogPostUrl(post.slug),
  lastmod: post.publishedAt,
}));

const urls = [...staticUrls, ...comparisonUrls, ...postUrls];

const renderSitemap = (site: URL) => `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls
  .map(
    ({ path, lastmod }) => `  <url>
    <loc>${new URL(path, site).href}</loc>
    <lastmod>${lastmod}</lastmod>
  </url>`,
  )
  .join("\n")}
</urlset>
`;

export const GET: APIRoute = ({ site }) => {
  const resolvedSite = site ?? new URL("https://reviu.dev");

  return new Response(renderSitemap(resolvedSite), {
    headers: {
      "Content-Type": "application/xml; charset=utf-8",
    },
  });
};
