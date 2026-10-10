let port;
let reconnect;
let pending = Promise.resolve();

function connect() {
  clearTimeout(reconnect);
  port = browser.runtime.connectNative("org.dojo.theme");
  port.onMessage.addListener((message) => {
    pending = pending.then(async () => {
      if (!message.theme || typeof message.digest !== "string") return;
      await browser.theme.update(message.theme);
      await browser.storage.local.set({ theme: message.theme });
      port.postMessage({ ack: message.digest });
    }).catch((error) => console.error("Dojo theme update failed", error));
  });
  port.onDisconnect.addListener(() => { reconnect = setTimeout(connect, 2000); });
  port.postMessage({ watch: true });
}

browser.storage.local.get("theme").then(({ theme }) => {
  if (theme) return browser.theme.update(theme);
}).catch((error) => console.error("Dojo saved theme could not be restored", error))
  .then(connect);
