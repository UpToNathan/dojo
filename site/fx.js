// Juice: short moments that react to the story (hero.js calls `hokumFx.card(story, el)` for every
// card that arrives). Everything is drawn in code: one canvas for particles, a few overlays, and
// transforms. Nothing darkens the page. It stays black and still between moments. With reduced motion there are no
// shakes or particles, only the police lights, gently.
;(() => {
  const reduce = matchMedia("(prefers-reduced-motion: reduce)")
  const page = document.querySelector(".first")
  const stack = document.getElementById("stack")

  // ---- overlays ----
  function overlay(cls) {
    const el = document.createElement("div")
    el.className = "fx " + cls
    el.setAttribute("aria-hidden", "true")
    document.body.append(el)
    return el
  }
  const red = overlay("fx-red")
  const blue = overlay("fx-blue")

  const anim = (el, frames, opts) => el.animate(frames, { fill: "none", ...opts })

  function shake(el, px, ms) {
    if (reduce.matches) return
    const f = []
    for (let i = 0; i <= 8; i++) {
      const k = px * (1 - i / 8)
      f.push({ transform: i === 8 ? "none" : `translate(${(Math.random() * 2 - 1) * k}px, ${(Math.random() * 2 - 1) * k}px)` })
    }
    anim(el, f, { duration: ms, easing: "linear" })
  }

  // Police lights: red and blue take turns washing over the black, then fade.
  function police(strength = 1, ms = 1800) {
    const peak = (reduce.matches ? 0.3 : 0.7) * strength
    const flashes = Math.max(2, Math.round(ms / 230))
    const frames = (offset) => {
      const f = []
      for (let i = 0; i <= flashes; i++) {
        const on = (i + offset) % 2 === 0 && i < flashes
        f.push({ opacity: on ? peak * (1 - (i / flashes) * 0.6) : 0, offset: i / flashes })
      }
      return f
    }
    anim(red, frames(0), { duration: ms, easing: "ease-out" })
    anim(blue, frames(1), { duration: ms, easing: "ease-out" })
  }

  // ---- particles ----
  const canvas = document.createElement("canvas")
  canvas.className = "fx fx-canvas"
  canvas.setAttribute("aria-hidden", "true")
  document.body.append(canvas)
  const g = canvas.getContext("2d")
  let parts = []
  let running = false
  let last = 0
  const MAX = 260
  function size() {
    const dpr = Math.min(2, devicePixelRatio || 1)
    // The viewport without its scrollbar, as the fixed canvas covers it.
    canvas.width = document.documentElement.clientWidth * dpr
    canvas.height = innerHeight * dpr
    g.setTransform(dpr, 0, 0, dpr, 0, 0)
  }
  size()
  addEventListener("resize", size)

  const CONFETTI = ["#ff6b6b", "#ffd93d", "#6bcB77", "#4d96ff", "#c77dff", "#ffffff"]
  function emit(n, from, make) {
    if (reduce.matches) return
    n = Math.min(n, MAX - parts.length)
    for (let i = 0; i < n; i++) parts.push({ ...from, ...make(i) })
    if (!running) { running = true; last = performance.now(); requestAnimationFrame(tick) }
  }
  function origin(el) {
    const r = el.getBoundingClientRect()
    return { x: r.left + r.width * (0.3 + Math.random() * 0.4), y: r.top + r.height * 0.5 }
  }
  function coins(el, n) {
    emit(n, {}, () => {
      const o = origin(el)
      return { kind: "coin", x: o.x, y: o.y, vx: (Math.random() * 2 - 1) * 260, vy: -180 - Math.random() * 360, g: 900, r: 4 + Math.random() * 3, rot: Math.random() * 6, vr: (Math.random() * 2 - 1) * 10, life: 1.6 + Math.random() * 0.6, age: 0, gold: Math.random() < 0.7 }
    })
  }
  function confetti(el, n) {
    emit(n, {}, () => {
      const o = origin(el)
      return { kind: "confetti", x: o.x, y: o.y, vx: (Math.random() * 2 - 1) * 320, vy: -260 - Math.random() * 380, g: 700, drag: 1.6, w: 5 + Math.random() * 4, h: 3 + Math.random() * 3, rot: Math.random() * 6, vr: (Math.random() * 2 - 1) * 14, life: 2 + Math.random() * 0.8, age: 0, color: CONFETTI[(Math.random() * CONFETTI.length) | 0] }
    })
  }
  function hearts(el, n) {
    emit(n, {}, (i) => {
      const o = origin(el)
      return { kind: "heart", x: o.x + (Math.random() * 2 - 1) * 60, y: o.y, vx: (Math.random() * 2 - 1) * 30, vy: -60 - Math.random() * 70, g: -10, r: 6 + Math.random() * 6, sway: Math.random() * 6, life: 2.4 + Math.random() * 1.2, age: -i * 0.06, color: Math.random() < 0.5 ? "#ff5d8f" : "#ff8fab" }
    })
  }
  // Liftoff: stars streak down the whole screen for a few seconds.
  function streaks(ms) {
    const until = performance.now() + ms
    const spawn = () => {
      if (performance.now() > until || reduce.matches) return
      emit(6, {}, () => ({ kind: "streak", x: Math.random() * innerWidth, y: -20, vx: 0, vy: 900 + Math.random() * 900, g: 0, len: 30 + Math.random() * 90, life: 2, age: 0 }))
      setTimeout(spawn, 50)
    }
    spawn()
  }

  function heart(x, y, r) {
    g.beginPath()
    g.moveTo(x, y + r * 0.35)
    g.bezierCurveTo(x, y - r * 0.3, x - r, y - r * 0.3, x - r, y + r * 0.2)
    g.bezierCurveTo(x - r, y + r * 0.7, x, y + r, x, y + r * 1.25)
    g.bezierCurveTo(x, y + r, x + r, y + r * 0.7, x + r, y + r * 0.2)
    g.bezierCurveTo(x + r, y - r * 0.3, x, y - r * 0.3, x, y + r * 0.35)
    g.fill()
  }
  function tick(now) {
    const dt = Math.min(0.05, (now - last) / 1000)
    last = now
    g.clearRect(0, 0, innerWidth, innerHeight)
    parts = parts.filter((p) => (p.age += dt) < p.life)
    for (const p of parts) {
      if (p.age < 0) continue
      if (p.drag) { p.vx -= p.vx * p.drag * dt; p.vy -= p.vy * p.drag * 0.4 * dt }
      p.vy += p.g * dt
      p.x += p.vx * dt
      p.y += p.vy * dt
      const fade = Math.min(1, (p.life - p.age) / 0.5)
      g.globalAlpha = fade
      if (p.kind === "coin") {
        p.rot += p.vr * dt
        g.fillStyle = p.gold ? "#f5c542" : "#7ee081"
        g.beginPath()
        g.ellipse(p.x, p.y, p.r * Math.abs(Math.cos(p.rot)) + 0.6, p.r, 0, 0, Math.PI * 2)
        g.fill()
      } else if (p.kind === "confetti") {
        p.rot += p.vr * dt
        g.save()
        g.translate(p.x, p.y)
        g.rotate(p.rot)
        g.fillStyle = p.color
        g.fillRect(-p.w / 2, -p.h / 2 * Math.abs(Math.cos(p.rot * 1.3)), p.w, p.h * Math.abs(Math.cos(p.rot * 1.3)) + 0.5)
        g.restore()
      } else if (p.kind === "heart") {
        g.globalAlpha = fade * Math.min(1, p.age * 3)
        g.fillStyle = p.color
        heart(p.x + Math.sin(p.age * 2 + p.sway) * 10, p.y, p.r)
      } else if (p.kind === "streak") {
        g.globalAlpha = 0.5 * fade
        g.strokeStyle = "#fff"
        g.lineWidth = 1
        g.beginPath()
        g.moveTo(p.x, p.y)
        g.lineTo(p.x, p.y - p.len)
        g.stroke()
      }
    }
    g.globalAlpha = 1
    if (parts.length) requestAnimationFrame(tick)
    else { running = false; g.clearRect(0, 0, innerWidth, innerHeight) }
  }

  // ---- which moment for which card ----
  function card(s, el) {
    const title = s.title || ""
    const body = s.body || ""
    // Every arrival: the stack takes a small punch (a firmer one for critical cards).
    if (!reduce.matches) anim(stack, [{ transform: "translateY(0)" }, { transform: `translateY(${s.urgency === 2 ? 5 : 2}px)` }, { transform: "none" }], { duration: 220, easing: "ease-out" })

    if (s.final) return
    if (s.title === "Payment failed") return void shake(stack, 5, 360)
    if (s.app === "Mission Control") {
      if (!reduce.matches) anim(page, [0, 1, 2, 3, 4, 3, 2, 1, 0].flatMap((k, i, a) => [{ transform: `translate(${(i % 2 ? 1 : -1) * k}px, ${(i % 2 ? -1 : 1) * k * 0.6}px)` }]), { duration: 2400, easing: "linear" })
      return void setTimeout(() => streaks(3600), 900)
    }
    if (s.urgency === 2) {
      shake(page, 6, 420)
      return void police(1, 1800)
    }
    if (s.app === "Waze" && /^Police reported/.test(title)) return void police(0.35, 700)
    if (/\$48,000,000/.test(body)) return void coins(el, 120)
    if (s.app === "Venmo" && /\$|sent you/.test(title + body) && !/declined/i.test(title)) return void coins(el, 18)
    if (s.app === "Fitness" && /Move ring/.test(title)) return void confetti(el, 70)
    if (s.app === "Hinge" || /marry me/.test(body)) return void hearts(el, 16)
    if (s.app === "Calendar" && /wedding/i.test(title) && document.getElementById("desk").dataset.branch === "love") { hearts(el, 14); return void confetti(el, 80) }
    if (s.app === "Zola") return void hearts(el, 8)
  }

  window.hokumFx = { card }
})()
