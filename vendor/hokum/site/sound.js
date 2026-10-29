// The story's sounds, synthesized with the Web Audio API: no files. Nothing plays until the
// visitor has clicked something (browsers require it, and we'd rather not anyway); after that,
// each card gets a short, quiet sound by kind. `window.hokumSound`: unlock(), play(kind),
// toggle(), muted. The choice is remembered in localStorage.
;(() => {
  const KEY = "hokum-sound"
  let ctx = null
  let out = null
  let muted = localStorage.getItem(KEY) === "off"
  let lastAt = 0
  const listeners = new Set()

  function unlock() {
    if (!ctx) {
      const AC = window.AudioContext || window.webkitAudioContext
      if (!AC) return false
      ctx = new AC()
      out = ctx.createGain()
      out.gain.value = 0.32
      // A gentle limiter, so bursts never clip.
      const comp = ctx.createDynamicsCompressor()
      comp.threshold.value = -18
      comp.ratio.value = 6
      out.connect(comp).connect(ctx.destination)
    }
    if (ctx.state === "suspended") ctx.resume()
    return true
  }

  // One soft partial: a sine (or other shape) with a quick attack and an exponential tail.
  function tone(at, freq, { dur = 0.35, gain = 0.25, type = "sine", glide = 0 } = {}) {
    const o = ctx.createOscillator()
    const g = ctx.createGain()
    o.type = type
    o.frequency.setValueAtTime(freq, at)
    if (glide) o.frequency.exponentialRampToValueAtTime(freq * glide, at + dur)
    g.gain.setValueAtTime(0.0001, at)
    g.gain.exponentialRampToValueAtTime(gain, at + 0.012)
    g.gain.exponentialRampToValueAtTime(0.0001, at + dur)
    o.connect(g).connect(out)
    o.start(at)
    o.stop(at + dur + 0.05)
  }
  function noise(at, { dur = 0.3, gain = 0.08, from = 2400, to = 600, q = 0.8 } = {}) {
    const len = Math.ceil(ctx.sampleRate * dur)
    const buf = ctx.createBuffer(1, len, ctx.sampleRate)
    const d = buf.getChannelData(0)
    for (let i = 0; i < len; i++) d[i] = Math.random() * 2 - 1
    const src = ctx.createBufferSource()
    src.buffer = buf
    const f = ctx.createBiquadFilter()
    f.type = "bandpass"
    f.Q.value = q
    f.frequency.setValueAtTime(from, at)
    f.frequency.exponentialRampToValueAtTime(to, at + dur)
    const g = ctx.createGain()
    g.gain.setValueAtTime(0.0001, at)
    g.gain.exponentialRampToValueAtTime(gain, at + dur * 0.3)
    g.gain.exponentialRampToValueAtTime(0.0001, at + dur)
    src.connect(f).connect(g).connect(out)
    src.start(at)
    src.stop(at + dur + 0.05)
  }

  const SOUNDS = {
    // Money: a little cha-ching.
    money: (t, v) => { noise(t, { dur: 0.08, gain: 0.05 * v, from: 5000, to: 3000, q: 3 }); tone(t + 0.06, 1568, { dur: 0.3, gain: 0.12 * v, type: "triangle" }); tone(t + 0.14, 2093, { dur: 0.45, gain: 0.1 * v, type: "triangle" }) },
    // Payment failed: a low double buzz.
    error: (t, v) => { tone(t, 196, { dur: 0.14, gain: 0.14 * v, type: "square" }); tone(t + 0.17, 196, { dur: 0.22, gain: 0.12 * v, type: "square" }) },
    // Declined: a short falling pair.
    declined: (t, v) => { tone(t, 523, { dur: 0.18, gain: 0.16 * v, type: "triangle" }); tone(t + 0.11, 392, { dur: 0.3, gain: 0.14 * v, type: "triangle" }) },
    // Liftoff: a low rumble.
    rumble: (t, v) => { noise(t, { dur: 2.2, gain: 0.12 * v, from: 180, to: 60, q: 0.5 }); tone(t, 55, { dur: 2, gain: 0.12 * v, type: "sawtooth", glide: 0.7 }) },
    // The wedding: a little arpeggio.
    wedding: (t, v) => [523, 659, 784, 1047].forEach((f, i) => tone(t + i * 0.12, f, { dur: 0.7, gain: 0.11 * v })),
  }

  function play(kind) {
    if (muted || !ctx || ctx.state !== "running" || !SOUNDS[kind]) return
    const now = ctx.currentTime
    // Don't stack: skip what lands on top of the last sound, and play near ones quieter.
    const since = now - lastAt
    if (since < 0.09) return
    const v = since < 0.5 ? 0.55 : 1
    lastAt = now
    SOUNDS[kind](now + 0.01, v)
  }

  function toggle() {
    muted = !muted
    localStorage.setItem(KEY, muted ? "off" : "on")
    if (!muted) unlock()
    for (const f of listeners) f(muted)
  }

  window.hokumSound = {
    unlock,
    play,
    toggle,
    get muted() { return muted },
    get live() { return !!ctx && ctx.state === "running" && !muted },
    onChange(f) { listeners.add(f); f(muted) },
  }
})()
