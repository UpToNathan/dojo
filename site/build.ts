// Builds hokum.app into site/dist from the repository: README.md, docs/theming.md,
// docs/themes.md and themes/*.kdl. The hero is story.js (the script) and hero.js (the cards).
//
// The site is one URL. dist/index.html is the page for browsers; dist/page.md is the same page
// as one self-contained markdown document, which worker.ts serves at / to agents.
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { join } from "node:path"

const site = import.meta.dir
const root = join(site, "..")
const dist = join(site, "dist")
const origin = "https://hokum.app"
const repo = "https://github.com/anomalyco/hokum"
const release = "https://anoma.ly/release/hokum/desktop/stable"
const prompt =
  "Read https://hokum.app and create a Hokum theme for me, then switch my config to it. I want it to look like: "

const read = (path: string) => readFileSync(join(root, path), "utf8")
const readme = read("README.md")
const themesDoc = read("docs/themes.md")
const themingDoc = read("docs/theming.md")
const version = read("Cargo.toml").match(/^version = "(.+)"/m)?.[1] ?? ""

const escape = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;")

// GitHub's heading anchors: lower case, punctuation dropped, spaces to dashes.
const slug = (s: string) =>
  s
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/&[a-z]+;/g, "")
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .trim()
    .replace(/\s/g, "-")

// README sections by `## ` heading, plus the text before the first one.
function sections(md: string) {
  const parts = md.split(/^## /m)
  const out: Record<string, string> = { "": parts[0] }
  for (const part of parts.slice(1)) {
    const nl = part.indexOf("\n")
    out[part.slice(0, nl).trim()] = part.slice(nl + 1).trim()
  }
  return out
}

const readmeParts = sections(readme)
const description = "A notification daemon for Wayland. Themes, per-app rules and Do not disturb, all in one config file."
const intro = readmeParts[""].split("\n\n").slice(2).join("\n\n").trim()
const use = readmeParts["Use"]
const installMd = readmeParts["Install"]

// Built-in themes, in the order README lists them, the default first.
const listed = readme.match(/built in \(([^)]+)\)/)?.[1].split(/,\s*/) ?? []
const defaultTheme = readme.match(/(\w+), with app icons, is the default/)?.[1] ?? "Capsule"
const themes = readdirSync(join(root, "themes"))
  .filter((f) => f.endsWith(".kdl") && f !== "default.kdl")
  .map((f) => {
    const id = f.replace(/\.kdl$/, "")
    const source = read(`themes/${f}`)
    const name = source.match(/^name "(.+)"/m)?.[1] ?? id
    const comment = source
      .split("\n")
      .filter((l) => l.startsWith("//"))
      .map((l) => l.replace(/^\/\/\s?/, ""))
      .join(" ")
      .replace(/^\S+:\s*/, "")
      .replace(/\s*The default\.$/, "")
      .replace(/^./, (c) => c.toUpperCase())
    return { id, name, comment, file: f, source }
  })
  .sort((a, b) => order(a.name) - order(b.name))
function order(name: string) {
  return name === defaultTheme ? -1 : listed.indexOf(name) >>> 0
}
const defaultCard = read("themes/default.kdl")

// "- **Label:** text" bullets in README's Use section are the feature list.
const features = [...use.matchAll(/^- \*\*(.+?):\*\* ([\s\S]+?)(?=\n- |\n\n|(?![\s\S]))/gm)].map((m) => ({
  title: m[1],
  md: m[2].replace(/\s*\n\s*/g, " ").replace(/^./, (c) => c.toUpperCase()),
}))
// ---------------------------------------------------------------------------------------------
// The markdown document: README, theming.md, themes.md and the built-in themes' sources, with
// headings moved under one title and every link between them pointing at a heading.

type Doc = { key: string; title: string; md: string }

const builtInTitle = "Built-in theme sources"
const themeHeading = (name: string, file: string) => `${name} (${file})`
const docs: Doc[] = [
  { key: "readme-install", title: "Install", md: `# Install\n\n${installMd}` },
  { key: "readme-use", title: "Use", md: `# Use\n\n${use}` },
  { key: "theming", title: "Configuration", md: themingDoc },
  { key: "themes", title: "Writing a theme", md: themesDoc },
]

// Walk markdown lines outside code fences.
function mapLines(md: string, f: (line: string) => string) {
  let fence = false
  return md
    .split("\n")
    .map((line) => {
      if (/^\s*```/.test(line)) {
        fence = !fence
        return line
      }
      return fence ? line : f(line)
    })
    .join("\n")
}

// Final anchors in document order, deduplicated the way GitHub does (`-1`, `-2`).
const used = new Map<string, number>()
const anchors = new Map<string, string>() // `${doc}#${original slug}` → final slug
function claim(text: string) {
  const base = slug(text)
  const n = used.get(base) ?? 0
  used.set(base, n + 1)
  return n ? `${base}-${n}` : base
}
claim("Hokum")
for (const doc of docs) {
  mapLines(doc.md, (line) => {
    const m = line.match(/^(#{1,6})\s+(.+)$/)
    if (!m) return line
    const text = m[1].length === 1 ? doc.title : m[2].replace(/`/g, "")
    anchors.set(`${doc.key}#${m[1].length === 1 ? "" : slug(m[2].replace(/`/g, ""))}`, claim(text))
    return line
  })
}
anchors.set("builtin#", claim(builtInTitle))
for (const t of [...themes.map((t) => ({ name: t.name, file: t.file })), { name: "The built-in card", file: "default.kdl" }])
  anchors.set(`kdl#${t.file}`, claim(themeHeading(t.name, t.file)))

function mdHref(href: string, doc: string) {
  if (href.startsWith("#")) return `#${anchors.get(`${doc}#${href.slice(1)}`) ?? href.slice(1)}`
  if (/^[a-z]+:/i.test(href)) return href
  const [path, hash] = href.split("#")
  const name = path.split("/").pop()
  if (name === "themes.md") return `#${anchors.get(`themes#${hash ?? ""}`)}`
  if (name === "theming.md") return `#${anchors.get(`theming#${hash ?? ""}`)}`
  const kdl = path.match(/themes\/([\w-]+\.kdl)$/)
  if (kdl && anchors.has(`kdl#${kdl[1]}`)) return `#${anchors.get(`kdl#${kdl[1]}`)}`
  return github(href)
}

function github(href: string) {
  const [path, hash] = href.split("#")
  const clean = path.replace(/^(\.\.\/)+/, "").replace(/^\.\//, "")
  return `${repo}/${clean.includes(".") ? "blob" : "tree"}/dev/${clean}${hash ? `#${hash}` : ""}`
}

function embed(doc: Doc) {
  return mapLines(doc.md, (line) => {
    const h = line.match(/^(#{1,6})\s+(.+)$/)
    if (h) return h[1].length === 1 ? `## ${doc.title}` : `#${line}`
    return line.replace(/\]\(([^)\s]+)\)/g, (_, href: string) => `](${mdHref(href, doc.key)})`)
  }).trim()
}

// The headline under the hero, and the start of the markdown.
const headline = "Notifications for Wayland."

const markdown = `# Hokum

${headline}

${intro}

This is ${origin}, the whole site as one markdown document: installing and using Hokum, the
configuration reference, the complete guide to writing a theme, and the source of every
built-in theme. Browsers get the same page as HTML, which opens on a crime story told entirely
in notification cards, stacking up in the corner of a desktop, with one of four endings picked at
random. It opens on Gary's $40 Venmo request and waits for the visitor to press Pay or Decline (either way he declines: Pay fails for insufficient funds). Below the story is one picture of the settings window, where you pick a theme. If you are a coding agent asked to make a
Hokum theme, read [Writing a theme](#${anchors.get("themes#")}) and
[Checking your work](#${anchors.get("themes#checking-your-work")}), then write the theme file
and set \`theme "<id>"\` in the user's config.

${embed(docs[0])}

## Features

${features.map((f) => `- **${f.title}:** ${f.md}`).join("\n")}

${docs.slice(1).map(embed).join("\n\n")}

## ${builtInTitle}

The built-in themes, as Hokum ships them. A theme of your own can start from one with
\`theme "<id>"\`, or copy and change it.

${[...themes.map((t) => ({ name: t.name, file: t.file, source: t.source })), { name: "The built-in card", file: "default.kdl", source: defaultCard }]
  .map((t) => `### ${themeHeading(t.name, t.file)}\n\n\`\`\`kdl\n${t.source.trimEnd()}\n\`\`\``)
  .join("\n\n")}

## License

MIT. Source: ${repo}
`

// ---------------------------------------------------------------------------------------------
// The HTML page: just the first screen and the footer. Agents get everything else (above).

// Install: one tab per way, as the README gives them. Each "On …:" / "From …:" paragraph and the
// code under it is a tab.
const installTabs: { id: string; label: string; code: string }[] = []
for (const m of installMd.slice(0, installMd.indexOf("Hokum starts through D-Bus") >>> 0).matchAll(/^(.+?):\n\n```sh\n([\s\S]+?)```/gm)) {
  const lead = m[1].replace(/\s*\n\s*/g, " ")
  const [id, label] = /AUR/.test(lead)
    ? ["arch", "Arch"]
    : /Debian/.test(lead)
      ? ["debian", "Debian / Ubuntu"]
      : /Fedora/.test(lead)
        ? ["fedora", "Fedora / openSUSE"]
        : [slug(lead), lead]
  installTabs.push({ id, label, code: m[2].trimEnd() })
}
// No Hokum logo yet: the favicon is a plain lettermark.
const favicon = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#111114"/><text x="16" y="22.5" text-anchor="middle" font-family="Inter,system-ui,sans-serif" font-size="19" font-weight="700" fill="#f2f2ef">H</text></svg>`
// The one picture: the settings window, rendered by Hokum itself (scripts/settings-shot.sh).
// WebP headers carry the canvas size; read it so the image reserves its space.
function webpSize(file: string) {
  const b = readFileSync(join(site, "assets", file))
  const fourcc = b.toString("ascii", 12, 16)
  if (fourcc === "VP8X") return [1 + b.readUIntLE(24, 3), 1 + b.readUIntLE(27, 3)]
  if (fourcc === "VP8L") {
    const bits = b.readUInt32LE(21)
    return [(bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1]
  }
  return [b.readUInt16LE(26) & 0x3fff, b.readUInt16LE(28) & 0x3fff]
}
const [shotW, shotH] = webpSize("settings.webp")
const avif = existsSync(join(site, "assets", "settings.avif"))
const settingsShot = `<figure class="shot">
    <picture>
      ${avif ? `<source type="image/avif" srcset="/assets/settings.avif ${shotW}w">` : ""}
      <img src="/assets/settings.webp" srcset="/assets/settings-1x.webp ${shotW / 2}w, /assets/settings.webp ${shotW}w" sizes="(max-width: 1240px) calc(100vw - 40px), 1160px" width="${shotW / 2}" height="${shotH / 2}" loading="lazy" decoding="async" alt="Hokum's settings window: Appearance, with the theme gallery and a preview of the cards">
    </picture>
  </figure>`

// The Anomaly signature behind the copyright line, as on opentunnel.xyz (and the opencode console).
const signature = readFileSync(join(site, "footer-signature.svg"), "utf8")
const githubIcon = `<svg viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>`


// The install row: a tab per platform and one `$` command, as opentunnel does it. site.js picks
// the visitor's platform (Arch unless the browser says Ubuntu, Debian or Fedora) and switches
// x86_64/ARM64; without JavaScript the radios still switch platforms.
const tabCode = (id: string) => installTabs.find((t) => t.id === id)!.code.split("\n").join(" && ")
const binPath = "~/.local/bin/hokum"
const installs: { id: string; label: string; x64: string; arm64?: string; link?: string }[] = [
  { id: "arch", label: "arch", x64: tabCode("arch"), link: `<a href="https://aur.archlinux.org/packages/hokum-bin" rel="noopener">aur ↗</a>` },
  { id: "debian", label: "debian", x64: tabCode("debian") },
  { id: "fedora", label: "fedora", x64: tabCode("fedora") },
  { id: "binary", label: "other", x64: `curl -fLo ${binPath} ${release}/linux-x64 && chmod +x ${binPath}`, link: `<a href="${repo}/releases" rel="noopener">releases ↗</a>` },
]
for (const i of installs) if (i.id !== "arch") i.arm64 = i.x64.replace(/linux-x64/g, "linux-arm64")
const copyIcon = `<svg class="i-copy" viewBox="0 0 16 16" aria-hidden="true"><rect x="5.5" y="5.5" width="8" height="8" rx="1.6" fill="none" stroke="currentColor" stroke-width="1.2"/><path d="M10.5 3.5v-.2c0-1-.8-1.8-1.8-1.8H3.3c-1 0-1.8.8-1.8 1.8v5.4c0 1 .8 1.8 1.8 1.8h.2" fill="none" stroke="currentColor" stroke-width="1.2"/></svg><svg class="i-done" viewBox="0 0 16 16" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>`
const cmd = (code: string, cls: string) =>
  `<span class="inst-cmd ${cls}"><code>${escape(code)}</code><button type="button" class="inst-copy" data-copy="${escape(code)}" aria-label="Copy command">${copyIcon}</button></span>`
const installRow = `<div class="inst" role="group" aria-label="Install">
${installs.map((t, i) => `    <input class="sr" type="radio" name="platform" id="p-${t.id}"${i === 0 ? " checked" : ""} aria-label="${t.label}">`).join("\n")}
    <input class="sr" type="checkbox" id="arm" aria-label="ARM64 build">
    <div class="inst-tabs">${installs.map((t) => `<label for="p-${t.id}">${t.label}</label>`).join("")}</div>
${installs.map((t) => `    <div class="inst-line" id="panel-${t.id}"><span class="inst-p" aria-hidden="true">$</span>${t.arm64 ? cmd(t.x64, "x64") + cmd(t.arm64, "arm64") : cmd(t.x64, "")}<span class="inst-meta">${t.arm64 ? `<label for="arm" class="cpu"><span class="x64">x86_64</span><span class="arm64">arm64</span></label>` : ""}${t.link ?? ""}</span></div>`).join("\n")}
  </div>`

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Hokum — notifications for Wayland</title>
<meta name="description" content="${escape(description)}">
<meta name="color-scheme" content="dark">
<meta name="theme-color" content="#000000">
<meta property="og:type" content="website">
<meta property="og:title" content="Hokum — notifications for Wayland">
<meta property="og:description" content="${escape(description)}">
<meta property="og:url" content="${origin}/">
<meta property="og:site_name" content="Hokum">
<meta property="og:image" content="${origin}/assets/og.png">
<meta property="og:image:type" content="image/png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="Notifications for Wayland. A stack of Hokum notification cards: Gary requests $40 on Venmo, texts about the lake, and Amazon ships a shovel, a tarp and garden lime.">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="Hokum — notifications for Wayland">
<meta name="twitter:description" content="${escape(description)}">
<meta name="twitter:image" content="${origin}/assets/og.png">
<meta name="twitter:image:alt" content="Notifications for Wayland. A stack of Hokum notification cards.">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="alternate" type="text/markdown" href="/?format=md" title="This page as markdown, for agents">
<link rel="preload" href="/assets/valkyrie-regular.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="/style.css">
</head>
<body>
<div class="first">
  <div class="wall" aria-hidden="true"><div class="stars"></div></div>
  <header class="top">
    <a class="wordmark" href="/">Hokum</a>
    <a class="top-github" href="${repo}" rel="noopener">${githubIcon}<span>github</span></a>
  </header>
  <div class="split">
    <section class="intro" id="intro">
      <h1>${escape(headline)}</h1>
      ${installRow}
    </section>
    <div class="desk" id="desk">
      <div class="clock" id="clock" aria-hidden="true"></div>
      <button class="sound" id="sound" type="button" aria-pressed="true">sound on</button>
      <div class="stack" id="stack" aria-live="polite" aria-label="Notifications"></div>
      <button id="replay" class="replay" type="button">replay</button>
    </div>
  </div>
  ${settingsShot}
</div>

<footer class="site-footer">
  <div class="copyright">
    ${signature}
    <p>©${new Date().getFullYear()} <a href="https://anoma.ly">anomaly</a>. all rights reserved</p>
  </div>
</footer>
<script src="/story.js"></script>
<script src="/sound.js"></script>
<script src="/hero.js"></script>
<script src="/fx.js"></script>
<script src="/site.js"></script>
</body>
</html>
`

rmSync(dist, { recursive: true, force: true })
mkdirSync(dist, { recursive: true })
writeFileSync(join(dist, "index.html"), html)
writeFileSync(join(dist, "page.md"), markdown)
writeFileSync(join(dist, "favicon.svg"), favicon)
cpSync(join(site, "style.css"), join(dist, "style.css"))
cpSync(join(site, "assets"), join(dist, "assets"), { recursive: true })
cpSync(join(site, "_headers"), join(dist, "_headers"))
for (const f of ["story.js", "sound.js", "hero.js", "fx.js", "site.js"]) cpSync(join(site, f), join(dist, f))
console.log(`built ${dist}: ${themes.length} themes, ${features.length} features, ${installTabs.length} install tabs, ${markdown.length} bytes of markdown`)
