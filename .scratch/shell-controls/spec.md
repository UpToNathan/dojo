# Shell clock and host controls

Show local date and time at the top-center of every output. The clock stays
centered independently of the right-hand controls and updates across minute,
date, timezone, and suspend/resume changes.

Add a native system menu trigger in the bottom-right corner. It offers session
and power actions only, without settings shortcuts. The trigger's render bounds
also define its input region. Power/session failures remain visible. Destructive
session actions require an in-menu confirmation. The menu contains Suspend,
Hibernate, Log out, Restart and Shut down. Hibernate requires its own confirmation.
Power actions delegate to the session/system services.

Add a Wi-Fi control immediately left of Bluetooth. Its native panel shows
NetworkManager's radio state, current connection, nearby networks and signal,
and provides scan, enable/disable, connect and disconnect. Saved connections use
NetworkManager profiles. New WPA2/WPA3 personal connections use a masked native
password editor; credentials never enter process arguments or logs. Unsupported
enterprise networks show a clear explanation rather than a misleading prompt.

Use a shared, GPUI-independent network controller with bounded operations and
authoritative readback. Background observation never disables controls or clears
an action error. Panel dismissal clears credentials without disconnecting an
established connection. All panels use the existing theme, font, shape, keyboard
and per-display lifecycle conventions.

Verify parser/domain behavior, input geometry, real host discovery without
disrupting the connection, and native panel workflows in an isolated compositor.
Do not execute logout, shutdown, or restart on the human's live session as a test.

The network menu and its top-bar icon follow NetworkManager's primary connection.
Show Ethernet for wired connections and USB Ethernet for USB network devices,
including phone tethering. Keep Wi-Fi controls available while using Ethernet.
Plugging in or removing a phone updates the menu and icon through NetworkManager
signals, with periodic reconciliation as a fallback. Do not choose the displayed
connection merely because Wi-Fi remains associated or an Ethernet device exists.
