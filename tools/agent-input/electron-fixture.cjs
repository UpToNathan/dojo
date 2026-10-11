const {app, BrowserWindow} = require('electron');
app.setPath('userData', process.env.DOJO_ELECTRON_PROFILE);
app.whenReady().then(() => {
  console.log('VERSIONS ' + JSON.stringify(process.versions));
  const win = new BrowserWindow({width: 400, height: 240, title: 'Dojo Electron seat fixture'});
  win.webContents.on('console-message', (_event, ...args) => console.log('PAGE ' + JSON.stringify(args)));
  win.webContents.on('did-finish-load', () => console.log('FIXTURE_READY'));
  win.loadURL('data:text/html,' + encodeURIComponent(`<html><body style="margin:0;background:#18354a;color:white"><button style="width:100vw;height:100vh;font:28px sans-serif" onclick="this.textContent='CLICK RECEIVED';console.log('AGENT_CLICK_RECEIVED')">Click test</button><script>
    document.addEventListener('mousedown', e => console.log('AGENT_MOUSE_DOWN ' + e.button));
    document.addEventListener('mouseup', e => console.log('AGENT_MOUSE_UP ' + e.button));
    document.addEventListener('keydown', e => console.log('AGENT_KEY ' + JSON.stringify({key:e.key,shift:e.shiftKey})));
    document.addEventListener('keyup', e => console.log('AGENT_UP ' + JSON.stringify({key:e.key,shift:e.shiftKey})));
    document.addEventListener('wheel', e => console.log('AGENT_WHEEL ' + JSON.stringify({x:e.deltaX,y:e.deltaY})));
  </script></body></html>`));
});
app.on('window-all-closed', () => app.quit());
