# Native Bluetooth dock

Add a small Bluetooth icon beside the seven-stroke audio wave in the desktop frame. Click it to open a native magnetic dock beneath the top bar. Keep the frame and existing controls unchanged. Tiles show device type, name and Connected / Saved / Nearby status. Selecting a tile expands details and explicit Connect, Disconnect or Pair buttons; include battery when BlueZ supplies it. Use a subtle entry movement and connected outline rather than drag-to-connect gestures.

Manage all Bluetooth devices, adapter power, explicit bounded discovery, and pairing confirmations / PIN / passkey entry. Facts follow BlueZ events. Pending calls and failures remain visible. Closing the dock stops only Dojo's discovery session. No automatic trust or unsolicited pairing approval. Handle BlueZ absence and restart without preventing shell startup.

Use one keyboard-capable temporary panel across displays, with backend state shared independently of GPUI. Keep the main frame non-keyboard-interactive. Verify real D-Bus behavior through a private fake BlueZ service when the host daemon is missing; install the release and inspect the native shell. The Arch host has Bluetooth hardware, but bluez is absent and sudo requires a human password. Prepare the host setup command without claiming hardware connection tests passed.

## Experience refinement

Use full-width device rows with actions always visible, connected devices first, and battery in the status line. Named discoveries appear first; offer an explicit reveal for unnamed devices without limiting the result count. Keep discovery results visible when scanning stops. Pairing prompts appear above the device list so they remain easy to find.

## Menu material

Menus use solid original background and highlight colors. No menu translucency or compositor blur; speech bubbles retain their blur treatment. Bluetooth closes through outside click, Escape, or toggling its top-bar icon, with no separate close button.

## Shell reveal

Menu silhouettes extend continuously from the desktop frame. Concave shoulders join the top edge to the content; right-side menus flow into the reserved right edge. Opening grows the outline while clipping content at a fixed layout, and dismissal retracts it. The shared reveal uses 100ms for tray menus, 150ms for launcher and Bluetooth, and 250ms for the theme sheet. It supports reversing Bluetooth mid-transition and snaps when reduced motion is requested. Launcher and theme manager reveal from the top edge too.
