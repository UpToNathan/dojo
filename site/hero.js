;(() => {
  // The trunk, then one of the branches: random per visit, a different one on each replay,
  // or the one `?branch=` names (1–4 or its key). `?branch=pay` presses Pay at the opening
  // choice (it fails), then plays a random ending.
  const TRUNK = window.STORY_TRUNK
  const BRANCHES = window.STORY_BRANCHES
  const KEYS = Object.keys(BRANCHES)
  let STORY, FINAL, START, branch
  const PAY = new URLSearchParams(location.search).get("branch") === "pay"
  const sound = window.hokumSound
  const seen = new Set()
  function choose(avoid) {
    const asked = new URLSearchParams(location.search).get("branch")
    const named = asked && (KEYS[Number(asked) - 1] ?? (KEYS.includes(asked) ? asked : null))
    if (named && !avoid) return named
    // Replays go through the ones not seen yet, then start over (never the same twice in a row).
    if (KEYS.every((k) => seen.has(k))) seen.clear()
    let pool = KEYS.filter((k) => !seen.has(k) && k !== avoid)
    if (!pool.length) pool = KEYS.filter((k) => k !== avoid)
    return pool[Math.floor(Math.random() * pool.length)]
  }
  function load(key) {
    branch = key
    seen.add(key)
    const base = TRUNK[TRUNK.length - 1].at
    STORY = [...TRUNK, ...BRANCHES[key].cards.map((c) => ({ ...c, at: base + c.at }))].map((c) => ({ ...c }))
    stamp()
    document.getElementById("desk").dataset.branch = key
  }
  function stamp() {
    FINAL = STORY.find((s) => s.final)
    STORY.forEach((s) => (s.ms = parse(s.time)))
    START = STORY[0].ms
    STORY.forEach((s, i) => (s.label = label(s, STORY[i - 1])))
  }
  const reduce = matchMedia("(prefers-reduced-motion: reduce)")
  const stack = document.getElementById("stack")
  const clock = document.getElementById("clock")
  // The story's stage: the hero section (scene and end classes live here, not on <body>).
  const body = document.getElementById("desk")

  // Capsule's spring: duration 0.38 s, bounce 0.12 (as SwiftUI/rune define them), mass 1.
  const K = (2 * Math.PI / 0.38) ** 2
  const C = (4 * Math.PI * (1 - 0.12)) / 0.38
  const GAP = 12

  // ---- icons: a glyph on a disc, drawn in code ----
  const W = (d) => `<path d="${d}" fill="none" stroke="#fff" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/>`
  const ICONS = {
    Calendar: ["#fff", `<rect x="0" y="0" width="20" height="7" fill="#e5484d"/><text x="10" y="16.3" text-anchor="middle" font-size="8.5" font-weight="700" fill="#222" font-family="Inter,system-ui">17</text>`],
    Venmo: ["#3d95ce", W("M6.3 6.2l3 8h1.4c1.6-2.4 3-5 3-6.8 0-.8-.2-1.2-.5-1.5")],
    Messages: ["#30d158", `<path d="M10 4.8c3.2 0 5.6 2 5.6 4.5S13.2 13.8 10 13.8c-.6 0-1.2-.1-1.7-.2L5.6 15l.8-2.4C5 11.8 4.4 10.6 4.4 9.3 4.4 6.8 6.8 4.8 10 4.8z" fill="#fff"/>`],
    Health: ["#fff", `<path d="M10 15.2s-5.2-3.2-5.2-6.6c0-1.6 1.2-2.9 2.7-2.9 1.1 0 2 .6 2.5 1.6.5-1 1.4-1.6 2.5-1.6 1.5 0 2.7 1.3 2.7 2.9 0 3.4-5.2 6.6-5.2 6.6z" fill="#ff375f"/>`],
    Fitness: ["#111", `<circle cx="10" cy="10" r="6" fill="none" stroke="#fa114f" stroke-width="2"/><circle cx="10" cy="10" r="3.6" fill="none" stroke="#a6ff00" stroke-width="2"/><circle cx="10" cy="10" r="1.3" fill="#00e1ff"/>`],
    Amazon: ["#232f3e", `<path d="M5 10.5c3 2.2 7 2.2 10 0" fill="none" stroke="#ff9900" stroke-width="1.8" stroke-linecap="round"/><path d="M13.2 9.6l2 .8-.6 2" fill="none" stroke="#ff9900" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>`],
    Photos: ["#fafafa", ["#f9ab00", "#ea4335", "#4285f4", "#34a853"].map((c, i) => `<ellipse cx="10" cy="6.6" rx="2.4" ry="3.4" fill="${c}" opacity=".9" transform="rotate(${i * 90} 10 10)"/>`).join("")],
    Ring: ["#1c9ad6", `<circle cx="10" cy="10" r="4.4" fill="none" stroke="#fff" stroke-width="2.2"/>`],
    Maps: ["#34a853", `<path d="M10 15.6s-4.2-4.2-4.2-7.2a4.2 4.2 0 0 1 8.4 0c0 3-4.2 7.2-4.2 7.2z" fill="#fff"/><circle cx="10" cy="8.4" r="1.6" fill="#34a853"/>`],
    Facebook: ["#1877f2", `<path d="M11 16V10.6h1.8l.3-2.1H11V7.2c0-.6.2-1 1-1h1.1V4.3c-.2 0-.9-.1-1.6-.1-1.6 0-2.7 1-2.7 2.8v1.5H7v2.1h1.8V16" fill="#fff"/>`],
    "Find My": ["#30d158", `<circle cx="10" cy="10" r="5.2" fill="none" stroke="#fff" stroke-width="1.6"/><circle cx="10" cy="10" r="2" fill="#fff"/>`],
    "Emergency Alert": ["#ef6f6f", W("M10 5l5.6 9.6H4.4z") + `<path d="M10 8.6v2.6" stroke="#fff" stroke-width="1.6" stroke-linecap="round"/><circle cx="10" cy="12.9" r=".9" fill="#fff"/>`],
    Phone: ["#30d158", `<path d="M7.2 4.8l1.4 2.6-1 1.2c.6 1.4 1.6 2.4 3 3l1.2-1 2.6 1.4-.5 2c-.2.6-.8 1-1.4.9C8.6 14.4 5.6 11.4 5.1 7.5c-.1-.6.3-1.2.9-1.4z" fill="#fff"/>`],
    Waze: ["#33ccff", `<path d="M5 12.5c-.4-.6-.6-1.4-.6-2.3a5.6 5.6 0 1 1 11.2 0c0 3-2.5 4.6-5.6 4.6H7.7" fill="#fff"/><circle cx="8.2" cy="9.2" r=".8" fill="#123"/><circle cx="11.8" cy="9.2" r=".8" fill="#123"/><path d="M8.2 11.2c1 .8 2.6.8 3.6 0" fill="none" stroke="#123" stroke-width=".9" stroke-linecap="round"/>`],
    Life360: ["#8a5cf6", `<circle cx="10" cy="10" r="4.6" fill="none" stroke="#fff" stroke-width="1.8"/><circle cx="10" cy="10" r="1.4" fill="#fff"/>`],
    Duolingo: ["#58cc02", `<circle cx="7.6" cy="9.6" r="2.6" fill="#fff"/><circle cx="12.4" cy="9.6" r="2.6" fill="#fff"/><circle cx="8" cy="9.8" r="1.1" fill="#1a1a1a"/><circle cx="12" cy="9.8" r="1.1" fill="#1a1a1a"/><path d="M9.2 12.6h1.6l-.8 1.2z" fill="#ffc800"/>`],
    Bank: ["#475569", W("M4.8 8.2L10 5l5.2 3.2M6.3 9.6v4.2M10 9.6v4.2M13.7 9.6v4.2M4.8 15.2h10.4")],
    Mobile: ["#64748b", `<rect x="5" y="11.5" width="2" height="3.5" rx=".6" fill="#fff"/><rect x="8.2" y="9" width="2" height="6" rx=".6" fill="#fff"/><rect x="11.4" y="6.5" width="2" height="8.5" rx=".6" fill="#fff" opacity=".45"/><rect x="14.6" y="4.5" width="2" height="10.5" rx=".6" fill="#fff" opacity=".45"/>`],
    WhatsApp: ["#25d366", W("M6.2 14.2l.7-2.2A4.9 4.9 0 1 1 8.8 14z")],
    News: ["#fa2d48", `<path d="M6.2 14.6V5.4l7.6 9.2V5.4" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round"/>`],
    "Mission Control": ["#1e293b", `<path d="M10 3.8c2 1.4 2.8 3.6 2.6 6.6l1.6 2v1.8l-2.4-.9H8.2l-2.4.9v-1.8l1.6-2C7.2 7.4 8 5.2 10 3.8z" fill="#fff"/><circle cx="10" cy="8.4" r="1.1" fill="#1e293b"/><path d="M9 14.6l1 1.8 1-1.8" fill="#ff9f0a"/>`],
    ChatGPT: ["#10a37f", [0, 60, 120].map((a) => `<ellipse cx="10" cy="10" rx="5.6" ry="2.6" fill="none" stroke="#fff" stroke-width="1.4" transform="rotate(${a} 10 10)"/>`).join("")],
    LinkedIn: ["#0a66c2", `<rect x="5.4" y="8.4" width="2.1" height="6.2" rx=".4" fill="#fff"/><circle cx="6.45" cy="6.2" r="1.25" fill="#fff"/><path d="M9.2 8.4h2v1c.4-.7 1.2-1.2 2.2-1.2 1.6 0 2.1 1.1 2.1 2.5v3.9h-2.1v-3.4c0-.8-.2-1.3-.9-1.3s-1.1.5-1.1 1.3v3.4H9.2z" fill="#fff"/>`],
    "Lime Corp": ["#84cc16", `<circle cx="10" cy="10" r="5.4" fill="#ecfccb"/><circle cx="10" cy="10" r="4.4" fill="#a3e635"/>` + [0, 60, 120, 180, 240, 300].map((a) => `<path d="M10 10V5.8" stroke="#ecfccb" stroke-width=".9" transform="rotate(${a} 10 10)"/>`).join("")],
    Forbes: ["#111", `<path d="M7.4 5.6h5.4M8.8 5.6v8.8M8.8 9.8h3.2M7.4 14.4h3" fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round"/>`],
    "Lake Man 2032": ["#b91c1c", `<path d="M10 4.8l1.5 3.2 3.5.4-2.6 2.4.7 3.5L10 12.6l-3.1 1.7.7-3.5L5 8.4l3.5-.4z" fill="#fff"/>`],
    WITSEC: ["#1e3a8a", `<path d="M10 4.4l4.6 1.8v3.6c0 3-2 4.9-4.6 5.8-2.6-.9-4.6-2.8-4.6-5.8V6.2z" fill="none" stroke="#fff" stroke-width="1.5" stroke-linejoin="round"/><path d="M10 7.6l.8 1.7 1.8.2-1.4 1.2.4 1.8-1.6-.9-1.6.9.4-1.8-1.4-1.2 1.8-.2z" fill="#fff"/>`],
    YouTube: ["#ff0000", `<path d="M8.4 7v6l5-3z" fill="#fff"/>`],
    Strava: ["#fc4c02", `<path d="M8.6 4.8l-3.4 6.6h2.1l1.3-2.6 1.3 2.6h2.1zM12.1 11.4l-1 2-1-2H8.6l2.5 4.8 2.5-4.8z" fill="#fff"/>`],
    Hinge: ["#1a1a1a", `<path d="M7 5.2v9.6M13 5.2v9.6M7 10h6" stroke="#fff" stroke-width="2" stroke-linecap="round"/>`],
    Zola: ["#f4ede4", `<path d="M10 14.8s-4.6-2.8-4.6-5.9c0-1.4 1-2.6 2.4-2.6 1 0 1.8.6 2.2 1.4.4-.8 1.2-1.4 2.2-1.4 1.4 0 2.4 1.2 2.4 2.6 0 3.1-4.6 5.9-4.6 5.9z" fill="#b5867a"/>`],
    Weather: ["#3b82f6", `<circle cx="10" cy="10" r="3" fill="#ffd60a"/>` + [0, 45, 90, 135, 180, 225, 270, 315].map((a) => `<path d="M10 4.2v1.3" stroke="#ffd60a" stroke-width="1.3" stroke-linecap="round" transform="rotate(${a} 10 10)"/>`).join("")],
    Spotify: ["#1db954", `<path d="M5.6 8.2q4.6-1.4 8.8.8M6.3 10.7q3.8-1 7.1.7M7 13q2.9-.7 5.4.5" fill="none" stroke="#000" stroke-width="1.4" stroke-linecap="round"/>`],
  }
  // Apps without an icon get their letter on a stable tint, as the daemon draws them.
  function tint(app) {
    let h = 2166136261
    for (const b of new TextEncoder().encode(app)) h = Math.imul(h ^ b, 16777619) >>> 0
    return `oklch(0.64 0.13 ${h % 360})`
  }
  function icon(app) {
    const i = ICONS[app]
    const bg = i ? i[0] : tint(app)
    const glyph = i ? i[1] : `<text x="10" y="13.9" text-anchor="middle" font-size="10.5" font-weight="700" fill="#fff" font-family="Inter,system-ui">${app[0].toUpperCase()}</text>`
    return `<svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="10" fill="${bg}"/>${glyph}</svg>`
  }

  // ---- story time ----
  const parse = (s) => {
    const [d, t = "00:00"] = s.split("T")
    const [y, m, dd] = d.split("-").map(Number)
    const [hh, mm] = t.split(":").map(Number)
    return new Date(y, m - 1, dd, hh, mm).getTime()
  }
  const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
  const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
  const hm = (d) => `${d.getHours() % 12 || 12}:${String(d.getMinutes()).padStart(2, "0")} ${d.getHours() < 12 ? "AM" : "PM"}`
  const sameDay = (a, b) => a.toDateString() === b.toDateString()
  function label(s, prev) {
    const d = new Date(s.ms)
    if (s.ms - START < 14 * 864e5) return `${DAYS[d.getDay()]} ${hm(d)}`
    if (prev && sameDay(d, new Date(prev.ms))) return hm(d)
    if (d.getFullYear() === 2026) return `${MONTHS[d.getMonth()]} ${d.getDate()}`
    return `${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()}`
  }
  let lastClock = ""
  // Fixed width, so the line never shifts as it ticks: three-letter names, two-digit day and hour.
  const pad = (n) => String(n).padStart(2, "0")
  function setClock(ms) {
    const d = new Date(ms)
    const h = d.getHours() % 12 || 12
    const text = `${DAYS[d.getDay()]} ${MONTHS[d.getMonth()]} ${pad(d.getDate())} ${d.getFullYear()}  ${pad(h)}:${pad(d.getMinutes())} ${d.getHours() < 12 ? "AM" : "PM"}`
    if (text !== lastClock) clock.textContent = lastClock = text
  }

  // ---- cards ----
  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c])
  let cards = [] // newest first
  let geo = { w: 420, x0: 0, top: 40, bottom: 0 }

  // Cards fill the right-hand column (#desk) under its clock line, down to its bottom (Replay shows
  // there only once the story has ended and the column is empty but for Gary). The
  // column has a fixed height, so the stack never moves the page.
  function measureGeo() {
    const vw = body.clientWidth
    const vh = body.clientHeight
    const w = Math.min(420, vw)
    geo = { w, x0: vw - w, top: 34, bottom: vh - 6, vw, vh }
    for (const c of cards) {
      c.el.style.setProperty("--w", w + "px")
      c.h = c.el.offsetHeight
    }
  }

  function make(s) {
    const el = document.createElement("article")
    el.className = "card" + (s.urgency === 2 ? " critical" : "")
    el.setAttribute("role", "status")
    const words = `${s.title} ${s.body || ""}`.split(/\s+/).length
    el.innerHTML =
      `<div class="head"><span class="badge">${icon(s.app)}</span>` +
      `<span class="src">${esc(s.app)}${s.ctx ? " · " + esc(s.ctx) : ""}</span>` +
      `<span class="time">${esc(s.label)}</span>` +
      `<button class="x" aria-label="Dismiss"><svg viewBox="0 0 10 10"><path d="M1 1l8 8M9 1L1 9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg></button></div>` +
      `<div class="row"><div class="content"><div class="title">${esc(s.title)}</div>` +
      (s.body ? `<div class="body">${esc(s.body)}</div>` : "") +
      (s.progress ? `<div class="progress"><i></i></div>` : "") +
      `</div>` +
      (s.avatar ? `<div class="avatar" aria-hidden="true">${esc(s.avatar)}</div>` : "") +
      `</div>` +
      (s.actions ? `<div class="actions">${s.actions.map((a) => `<button>${esc(a)}</button>`).join("")}</div>` : "")
    el.style.setProperty("--w", geo.w + "px")
    stack.appendChild(el)
    const c = {
      s, el, h: el.offsetHeight, bar: el.querySelector(".progress i"),
      x: 48, y: geo.top, sc: 0.97, o: 0, vx: 0, vy: 0, vs: 0, vo: 0,
      tx: 0, ty: geo.top, ts: 1, to: 1,
      age: 0, life: s.final ? Infinity : s.urgency === 2 ? 16 : Math.min(15, 8.5 + words * 0.22),
      leaving: false, fling: false, drag: null,
    }
    if (reduce.matches) { c.x = 0; c.sc = 1; c.o = 1 }
    el.querySelector(".x").addEventListener("click", () => { if (s.choice && choosing) decide("decline"); leave(c, 0, true) })
    el.querySelectorAll(".actions button").forEach((b) =>
      b.addEventListener("click", () => {
        if (s.choice) {
          if (choosing) decide(b.textContent === "Pay" ? "pay" : "decline", b)
          return
        }
        leave(c, 0, true)
      }),
    )
    el.addEventListener("pointerdown", (e) => down(c, e))
    cards.unshift(c)
    layout()
    c.y = c.ty
    return c
  }

  function leave(c, delay = 0, byHand = false) {
    if (c.leaving) return
    if (delay) return void setTimeout(() => leave(c), delay)
    c.leaving = true
    // A short glide out while it fades, as Hokum's cards leave. (Gliding the card's full width
    // used to carry it past the column, where it showed beside the stack as a ghost.)
    c.tx = 24
    c.to = 0
    c.el.style.pointerEvents = "none"
    c.el.style.zIndex = "0"
    if (reduce.matches) remove(c)
    layout()
  }
  function remove(c) {
    c.el.remove()
    cards = cards.filter((k) => k !== c)
    layout()
  }

  // Newest at the top; a card that no longer fits above the column's bottom leaves the way
  // cards leave (a short glide and a fade) rather than piling up.
  function layout() {
    let y = geo.top
    let shown = 0
    for (const c of cards) {
      if (c.leaving) continue
      if (shown > 0 && y + c.h > geo.bottom) {
        c.leaving = true
        c.tx = 24
        c.to = 0
        c.el.style.pointerEvents = "none"
        c.el.style.zIndex = "0"
        if (reduce.matches) { c.el.remove(); cards = cards.filter((k) => k !== c) }
        continue
      }
      c.ty = y
      c.to = 1
      c.ts = 1
      c.el.style.pointerEvents = ""
      c.el.style.zIndex = "2"
      y += c.h + GAP
      shown++
    }
  }

  // ---- drag & fling ----
  const canHover = matchMedia("(hover: hover)")
  function down(c, e) {
    if (e.target.closest("button") || c.leaving) return
    c.el.setPointerCapture(e.pointerId)
    c.el.classList.add("dragging")
    c.drag = { id: e.pointerId, sx: e.clientX - c.x, sy: e.clientY, y0: c.y - c.ty, samples: [[performance.now(), e.clientX, e.clientY]] }
    c.vx = c.vy = 0
    const move = (ev) => {
      if (ev.pointerId !== c.drag?.id) return
      c.x = ev.clientX - c.drag.sx
      const dy = ev.clientY - c.drag.sy
      c.dy = c.drag.y0 + dy * 0.45
      c.drag.samples.push([performance.now(), ev.clientX, ev.clientY])
      if (c.drag.samples.length > 6) c.drag.samples.shift()
    }
    const up = (ev) => {
      if (ev.pointerId !== c.drag?.id) return
      c.el.removeEventListener("pointermove", move)
      c.el.removeEventListener("pointerup", up)
      c.el.removeEventListener("pointercancel", up)
      c.el.classList.remove("dragging")
      const sm = c.drag.samples
      const [t0, x0, y0] = sm[0]
      const [t1, x1, y1] = sm[sm.length - 1]
      const dt = Math.max(16, t1 - t0)
      const vx = ((x1 - x0) / dt) * 1000
      const vy = ((y1 - y0) / dt) * 1000
      c.drag = null
      c.y = c.ty + (c.dy || 0)
      c.dy = 0
      if (Math.abs(c.x) > geo.w * 0.33 || (Math.abs(vx) > 600 && Math.sign(vx) === Math.sign(c.x || vx))) {
        c.leaving = c.fling = true
        c.vx = Math.sign(c.x || vx) * Math.max(Math.abs(vx), 1400)
        c.vy = vy * 0.5
        c.el.style.pointerEvents = "none"
        if (reduce.matches) remove(c)
        layout()
      } else {
        c.vx = vx
        c.vy = vy * 0.45
      }
    }
    c.el.addEventListener("pointermove", move)
    c.el.addEventListener("pointerup", up)
    c.el.addEventListener("pointercancel", up)
  }

  // ---- the loop ----
  let t = 0
  let next = 0
  let quieted = false
  let replayAt = 0
  let last = performance.now()
  let onscreen = true
  new IntersectionObserver(([e]) => (onscreen = e.isIntersecting)).observe(body)
  const spring = (c, p, v, target, dt) => {
    c[v] += (K * (target - c[p]) - C * c[v]) * dt
    c[p] += c[v] * dt
  }

  // Which sound a card makes: only a few standout moments do. Money that bursts coins
  // cha-chings, the launch rumbles, the wedding chimes; routine cards are silent.
  function kind(s) {
    if (s.title === "Payment failed") return "error"
    if (s.app === "Mission Control") return "rumble"
    if (s.app === "Calendar" && /wedding/i.test(s.title) && body.dataset.branch === "love") return "wedding"
    if (/\$48,000,000/.test(s.body || "")) return "money"
    if (s.app === "Venmo" && /sent you/.test(s.title)) return "money"
    return null
  }

  // The opening choice: Gary's request waits for Pay or Decline. Pressing one is the click that
  // lets the story make sound; nobody pressing anything declines on their behalf, quietly.
  let choosing = null
  let choiceCard = null
  function decide(pick, button, pressed = !!button) {
    if (!choosing) return
    clearTimeout(choosing.timer)
    choosing = null
    choiceCard?.el.classList.remove("choosing")
    if (button) button.classList.add("picked")
    if (pressed) sound.unlock()
    choiceCard?.el.querySelectorAll(".actions button").forEach((b) => (b.disabled = true))
    if (pick === "pay") {
      // The payment fails; the trunk carries on with "You declined", same as pressing Decline.
      const at = t
      const fail = window.STORY_PAY.map((c) => ({ ...c, at: at + c.at }))
      STORY = [...STORY.slice(0, next), ...fail, ...STORY.slice(next)]
      stamp()
    } else if (pressed) sound.play("declined")
    // Pressed or not, the declined line arrives on cue.
  }

  function fire(s) {
    if (s.scene) body.dataset.scene = s.scene
    // Cards fired to catch up (?t=, or a tab coming back) arrive without sound or effects.
    const live = t - s.at < 1
    const k = kind(s)
    if (live && k) sound.play(k)
    if (s.final) {
      body.classList.add("end")
      // Replay shows once Gary's last word has sat there for a beat.
      clearTimeout(replayAt)
      replayAt = setTimeout(() => body.classList.add("over"), reduce.matches ? 1200 : 3000)
    }
    const c = make(s)
    if (live) window.hokumFx?.card(s, c.el)
    if (s.choice && t - s.at < 1) {
      choiceCard = c
      c.el.classList.add("choosing")
      choosing = { timer: setTimeout(() => decide(PAY ? "pay" : "decline", PAY ? c.el.querySelector(".actions button") : null, false), PAY ? 1500 : 7000) }
    }
  }

  function frame(now) {
    const dt = Math.min(0.1, (now - last) / 1000)
    last = now
    if (!onscreen) return requestAnimationFrame(frame)
    if (!choosing) t += dt
    if (!quieted && t >= FINAL.at - (window.QUIET ?? 3)) {
      quieted = true
      cards.forEach((c, i) => leave(c, reduce.matches ? 0 : (cards.length - 1 - i) * 90))
    }
    while (next < STORY.length && STORY[next].at <= t) fire(STORY[next++])

    // The clock runs between cards, faster where the story skips ahead.
    const a = STORY[Math.max(0, next - 1)]
    const b = STORY[next]
    if (next === 0) setClock(STORY[0].ms - 6e4 * (STORY[0].at - t))
    else if (!b || a.final) setClock(a.ms)
    else setClock(a.ms + (b.ms - a.ms) * Math.min(1, (t - a.at) / (b.at - a.at)))

    const rm = reduce.matches
    const hovering = canHover.matches && !!stack.querySelector(".card:hover")
    for (const c of cards.slice()) {
      if (!hovering && !c.leaving && !choosing) c.age += dt
      if (!c.leaving && c.age > c.life) leave(c)
      if (c.bar) {
        const [p0, p1] = c.s.progress
        c.bar.style.transform = `scaleX(${p0 + (p1 - p0) * Math.min(1, c.age / 6)})`
      }
      if (c.drag) {
        spring(c, "sc", "vs", 1.02, dt)
        c.y = c.ty + (c.dy || 0)
      } else if (c.fling) {
        c.x += c.vx * dt
        c.y += c.vy * dt
        c.o = Math.max(0, 1 - Math.abs(c.x) / (geo.w * 0.9))
        if (c.o <= 0 || Math.abs(c.x) > geo.vw) { remove(c); continue }
      } else if (rm) {
        c.x = c.tx; c.y = c.ty; c.sc = c.ts; c.o = c.to
      } else {
        for (let i = 0; i < 2; i++) {
          const h = dt / 2
          spring(c, "x", "vx", c.tx, h)
          spring(c, "y", "vy", c.ty, h)
          spring(c, "sc", "vs", c.ts, h)
          spring(c, "o", "vo", c.to, h)
        }
        if (c.leaving && c.o < 0.02) { remove(c); continue }
      }
      const o = Math.max(0, Math.min(1, c.o))
      c.el.style.transform = `translate3d(${(geo.x0 + c.x).toFixed(2)}px, ${c.y.toFixed(2)}px, 0) scale(${c.sc.toFixed(4)})`
      c.el.style.opacity = o.toFixed(3)
      c.el.style.visibility = o < 0.01 ? "hidden" : ""
    }
    requestAnimationFrame(frame)
  }

  function reset() {
    if (choosing) clearTimeout(choosing.timer)
    choosing = null
    choiceCard = null
    load(choose(branch))
    for (const c of cards) c.el.remove()
    cards = []
    t = 0
    next = 0
    quieted = false
    delete body.dataset.scene
    body.classList.remove("end", "over")
    clearTimeout(replayAt)
    layout()
  }
  document.getElementById("replay").addEventListener("click", () => { sound.unlock(); reset() })
  // Any click on the story lets it make sound from then on.
  body.addEventListener("pointerdown", () => sound.unlock(), { capture: true })
  const toggle = document.getElementById("sound")
  toggle.addEventListener("click", () => sound.toggle())
  sound.onChange((muted) => { toggle.textContent = muted ? "sound off" : "sound on"; toggle.setAttribute("aria-pressed", String(!muted)) })
  addEventListener("resize", () => { measureGeo(); layout() })

  window.hokumStoryTime = () => t
  window.hokumStoryBranch = () => branch
  load(choose())
  // Debug/preview: ?t=SECONDS starts the story at that point.
  const start = Number(new URLSearchParams(location.search).get("t"))
  measureGeo()
  if (start > 0) {
    t = start
    while (next < STORY.length && STORY[next].at <= t - 12) next++
    const scene = STORY.slice(0, next).reverse().find((s) => s.scene)
    if (scene) body.dataset.scene = scene.scene
  }
  document.fonts.ready.then(() => { measureGeo(); layout() })
  new ResizeObserver(() => { measureGeo(); layout() }).observe(body)
  requestAnimationFrame((n) => { last = n; requestAnimationFrame(frame) })
})()
