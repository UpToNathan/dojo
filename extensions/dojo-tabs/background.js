// Dojo Tabs: reports the browser's windows and tabs to Dojo's native host, which keeps the latest
// report in a private file. Read-only: it never opens, moves, closes or reads the content of a tab.
import { toSnapshot } from './snapshot.js';

const HOST = 'net.dojo.tabs';
let port;
let timer;

async function report() {
  if (!port) return;
  const windows = await chrome.windows.getAll({ populate: true, windowTypes: ['normal'] });
  try { port.postMessage(toSnapshot(windows)); } catch { /* the host went away; onDisconnect reconnects */ }
}

// Many events arrive together when a window opens or closes; one report covers them.
function schedule() {
  clearTimeout(timer);
  timer = setTimeout(report, 150);
}

function connect() {
  port = chrome.runtime.connectNative(HOST);
  port.onDisconnect.addListener(() => {
    port = undefined;
    // Without the host the file is gone, which tells Dojo not to trust an old list. Try again later.
    setTimeout(connect, 5000);
  });
  schedule();
}

for (const event of [
  chrome.tabs.onCreated, chrome.tabs.onUpdated, chrome.tabs.onRemoved, chrome.tabs.onMoved,
  chrome.tabs.onActivated, chrome.tabs.onAttached, chrome.tabs.onDetached, chrome.tabs.onReplaced,
  chrome.windows.onCreated, chrome.windows.onRemoved, chrome.windows.onFocusChanged,
]) event.addListener(schedule);

connect();
