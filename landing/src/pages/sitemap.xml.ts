import type { APIRoute } from "astro";

const urls = ["/", "/changelog"];

const renderSitemap = (site: URL) => `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls
  .map(
    (pathname) => `  <url>
    <loc>${new URL(pathname, site).href}</loc>
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
