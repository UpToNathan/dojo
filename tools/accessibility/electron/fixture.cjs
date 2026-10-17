const { app, BrowserWindow } = require('electron');
const { mkdirSync } = require('node:fs');
const { isAbsolute, join } = require('node:path');
const readline = require('node:readline');

const profile = process.env.DOJO_ELECTRON_FIXTURE_PROFILE;
if (!profile || !isAbsolute(profile)) throw new Error('An isolated fixture profile is required');
if (process.env.DOJO_TEST_PRIVATE_BUS !== '1') throw new Error('The fixture requires the isolated accessibility test bus');
mkdirSync(profile, { recursive: true, mode: 0o700 });
mkdirSync(join(profile, 'session'), { recursive: true, mode: 0o700 });
app.setPath('userData', profile);
app.setPath('sessionData', join(profile, 'session'));
const windows = new Map();
const bridgeAddon = process.env.DOJO_ELECTRON_FIXTURE_BRIDGE_ADDON;
if (bridgeAddon && (!isAbsolute(bridgeAddon) || !bridgeAddon.startsWith('/nix/store/'))) throw new Error('Use an immutable bridge fixture addon');
const bridge = bridgeAddon ? require(bridgeAddon) : undefined;

function createWindow(name) {
  const window = new BrowserWindow({
    title: 'Dojo Electron exact fixture', width: 540, height: 440,
    webPreferences: { nodeIntegration: false, contextIsolation: true, sandbox: true },
  });
  windows.set(name, window);
  window.on('closed', () => { if (windows.get(name) === window) windows.delete(name); });
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.on('will-navigate', event => event.preventDefault());
  const prefix = `${name} electron`;
  const html = `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'">
    <title>Dojo Electron exact fixture</title></head><body>
    <h1>${name} native accessibility fixture</h1>
    <input id="input" aria-label="${prefix} input" value="initial">
    <button id="action" aria-label="${prefix} action">Run action</button>
    <input type="range" min="0" max="100" value="1" aria-label="${prefix} value">
    <select size="2" aria-label="${prefix} choices">
      <option selected>${prefix} choice one</option><option>${prefix} choice two</option>
    </select>
    <script>document.getElementById('action').addEventListener('click', () => {
      document.getElementById('action').setAttribute('aria-label', '${prefix} action invoked');
      document.getElementById('action').textContent = 'Invoked';
    });</script></body></html>`;
  window.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`).catch(error => { console.error(error); app.exit(1); });
  return window;
}

app.whenReady().then(() => {
  app.setAccessibilitySupportEnabled(true);
  createWindow('Alpha'); createWindow('Beta');
  const lines = readline.createInterface({ input: process.stdin });
  lines.on('line', line => {
    try {
      const request = JSON.parse(line);
      if (!['Alpha', 'Beta'].includes(request.window)) throw new Error('Unknown fixture window');
      const window = windows.get(request.window);
      if (!window) throw new Error('Fixture window is missing');
      switch (request.command) {
        case 'hide': window.hide(); break;
        case 'show': window.show(); break;
        case 'recreate': window.destroy(); createWindow(request.window); break;
        case 'disable-accessibility': app.setAccessibilitySupportEnabled(false); break;
        case 'enable-accessibility': app.setAccessibilitySupportEnabled(true); break;
        case 'cleanup-bridge': if (!bridge) throw new Error('Bridge fixture addon is missing'); bridge.cleanup(); break;
        case 'initialize-bridge': if (!bridge) throw new Error('Bridge fixture addon is missing'); bridge.initialize(); break;
        default: throw new Error('Unknown fixture command');
      }
    } catch (error) { console.error(error); app.exit(1); }
  });
  console.log(JSON.stringify({ fixture_ready: true, versions: process.versions }));
}).catch(error => { console.error(error); app.exit(1); });
app.on('window-all-closed', () => app.quit());
