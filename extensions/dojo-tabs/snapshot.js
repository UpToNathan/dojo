// What the extension tells Dojo: every normal window and its tabs, in order. Pure, so it can be
// tested without a browser.

/** @param {{id?: number, focused?: boolean, tabs?: object[]}[]} windows the browser's windows, with tabs */
export function toSnapshot(windows, now = Date.now()) {
  return {
    v: 1,
    at: now,
    windows: windows
      .filter(window => typeof window.id === 'number')
      .map(window => ({
        id: window.id,
        focused: window.focused === true,
        tabs: (window.tabs ?? [])
          .slice()
          .sort((a, b) => (a.index ?? 0) - (b.index ?? 0))
          .map(tab => ({
            id: tab.id ?? -1,
            active: tab.active === true,
            pinned: tab.pinned === true,
            // A tab still loading has only its pending address.
            url: String(tab.url || tab.pendingUrl || ''),
            title: String(tab.title || ''),
          })),
      })),
  };
}
