// hokum.app is one URL. `/` is the page: HTML for everyone, the same page as one markdown
// document (dist/page.md) for agents that ask for it or are known agents. Assets are served as
// they are; every other path, including the old /themes.md, /llms.txt and friends, redirects to `/`. www redirects to the apex.
interface Env {
  ASSETS: { fetch(request: Request | string): Promise<Response> }
}

const assets = /^\/(assets\/|(style\.css|favicon\.svg|story\.js|sound\.js|hero\.js|fx\.js|site\.js)$)/

// Known agents and command line tools. Everyone else, including unknown user agents, search
// crawlers and link-preview fetchers, gets the HTML page with its Open Graph tags.
const agents = new RegExp(
  [
    // HTTP clients and CLIs, which name themselves first.
    "^(curl|wget|httpie|python-requests|python-httpx|python-urllib|node-fetch|undici|axios|go-http-client)\\/|\\baiohttp\\/",
    // AI fetchers and crawlers.
    "\\b(claudebot|claude-user|claude-searchbot|anthropic-ai|gptbot|chatgpt-user|oai-searchbot|perplexitybot|perplexity-user)\\b",
    // Coding agents.
    "\\b(opencode|opencode-user|cursor|codex|codex_cli_rs|aider|gemini-cli)\\b",
  ].join("|"),
  "i",
)

// Link-preview fetchers and search crawlers always get the HTML, even when they also match an agent
// (iMessage says facebookexternalhit and Twitterbot).
const previews =
  /googlebot|google-inspectiontool|bingbot|applebot|twitterbot|facebookexternalhit|facebot|facebookcatalog|slackbot|slack-imgproxy|discordbot|linkedinbot|telegrambot|whatsapp|skypeuripreview|iframely|embedly|redditbot|mastodon|bluesky|cardyb|pinterestbot|vkshare|opengraph|notion/i

/** Quality of the most specific Accept entry matching `type`, or -1 if none matches. */
function quality(accept: string, type: string, exact = false) {
  let best = -1
  let specificity = -1
  for (const part of accept.split(",")) {
    const [range, ...params] = part.trim().toLowerCase().split(";")
    const q = Number(params.find((p) => p.trim().startsWith("q="))?.split("=")[1] ?? 1)
    const s =
      range === type ? 2 : exact ? -1 : range === `${type.split("/")[0]}/*` ? 1 : range === "*/*" ? 0 : -1
    if (s > specificity) {
      specificity = s
      best = Number.isNaN(q) ? 1 : q
    }
  }
  return best
}

export function wantsMarkdown(request: Request) {
  const format = new URL(request.url).searchParams.get("format")
  if (format === "md" || format === "markdown") return true
  if (format === "html") return false
  const accept = request.headers.get("Accept") ?? ""
  const ua = request.headers.get("User-Agent") ?? ""
  if (previews.test(ua)) return false
  const html = quality(accept, "text/html")
  // Only types named outright count: `*/*` alone is no request for markdown.
  const md = Math.max(quality(accept, "text/markdown", true), quality(accept, "text/x-markdown", true))
  const plain = quality(accept, "text/plain", true)
  if ((md > 0 && md >= html) || (plain > 0 && plain > html)) return true
  return agents.test(ua)
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url)
    if (url.hostname === "www.hokum.app") {
      url.hostname = "hokum.app"
      return Response.redirect(url.toString(), 301)
    }
    if (assets.test(url.pathname)) return env.ASSETS.fetch(request)
    if (url.pathname !== "/") return Response.redirect(`${url.origin}/`, 301)

    const markdown = wantsMarkdown(request)
    const asset = await env.ASSETS.fetch(new Request(new URL(markdown ? "/page.md" : "/", url), request))
    const headers = new Headers(asset.headers)
    headers.set("Content-Type", markdown ? "text/markdown; charset=utf-8" : "text/html; charset=utf-8")
    headers.set("Vary", "Accept, User-Agent")
    headers.set("Link", '<https://hokum.app/?format=md>; rel="alternate"; type="text/markdown"')
    if (markdown) headers.set("Access-Control-Allow-Origin", "*")
    return new Response(asset.body, { status: asset.status, headers })
  },
}
