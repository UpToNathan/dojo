// The page around the hero: the platform guess and copy buttons.

// Which install to show first. Browsers rarely say the distribution; when they do, use it.
// `?platform=arch|debian|fedora|binary` overrides.
const ua = navigator.userAgent
const forced = new URLSearchParams(location.search).get("platform")
const guess = forced || (/Ubuntu|Debian|Mint|Pop!_OS/i.test(ua) ? "debian" : /Fedora|SUSE|Red Hat|CentOS/i.test(ua) ? "fedora" : "arch")
const radio = document.getElementById("p-" + guess)
if (radio) radio.checked = true
// ARM machines get the arm64 builds (the browser says aarch64 on Linux, or `?cpu=arm64`).
const arm = document.getElementById("arm")
if (arm && (/aarch64|arm64/i.test(ua) || new URLSearchParams(location.search).get("cpu") === "arm64")) arm.checked = true

// Copy buttons.
for (const b of document.querySelectorAll("button[data-copy]"))
  b.addEventListener("click", () => {
    navigator.clipboard.writeText(b.dataset.copy.trimEnd()).then(() => {
      b.classList.add("done")
      setTimeout(() => b.classList.remove("done"), 1500)
    })
  })
