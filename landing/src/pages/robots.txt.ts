import type { APIRoute } from "astro";

const getRobotsTxt = (site: URL) => `User-agent: *
Allow: /

Sitemap: ${new URL("/sitemap.xml", site).href}
`;

export const GET: APIRoute = ({ site }) => {
  const resolvedSite = site ?? new URL("https://reviu.dev");

  return new Response(getRobotsTxt(resolvedSite), {
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
    },
  });
};
