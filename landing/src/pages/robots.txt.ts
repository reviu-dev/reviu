import type { APIRoute } from "astro";

const getRobotsTxt = (site: URL) => `User-agent: *
Allow: /

# AI answer engines are welcome to read and cite Reviu.
User-agent: GPTBot
Allow: /

User-agent: OAI-SearchBot
Allow: /

User-agent: ChatGPT-User
Allow: /

User-agent: ClaudeBot
Allow: /

User-agent: Claude-Web
Allow: /

User-agent: PerplexityBot
Allow: /

User-agent: Google-Extended
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
