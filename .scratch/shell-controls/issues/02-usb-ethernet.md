# Show the active USB Ethernet connection

Status: resolved
Type: task

Extend the shell controls spec so the network menu and top-bar icon follow
phone USB tethering and return to Wi-Fi when NetworkManager changes the primary
connection. Include the host requirements for iPhone pairing and USB Ethernet.

## Comments

Implemented primary-connection observation, Ethernet and USB Ethernet menu
titles, the Ethernet icon, and NetworkManager signal refresh. A private D-Bus
test verifies Wi-Fi to USB to Wi-Fi, Wi-Fi remaining associated while USB is
primary, USB without a Wi-Fi adapter, and disconnection. Existing Wi-Fi tests pass.

The attached iPhone appears on USB, but the host currently uses its Wi-Fi hotspot.
The usbmuxd package is missing. scripts/setup-usb-tethering.sh installs the pairing
dependencies and loads ipheth. Running setup requires the operator's sudo
password and may require Trust on the phone.

The operator installed the packages and reconnected the phone. Pairing validated;
NetworkManager activated eth0 through ipheth with route metric 100, ahead of
Wi-Fi's 600. The real controller reported Ethernet with usb=true. The rendered
menu showed USB Ethernet, Wired connection 2, and eth0, and the top-bar Ethernet
icon appeared. Internet packets sent through eth0 succeeded.

A transient refresh failure during hotplug exposed a stale background error.
Observation errors now clear on successful readback independently of action
errors. The final live menu has no stale error. All 123 binary tests passed before
that fix; the 14 affected Wi-Fi/controller/UI tests passed after it. Release build,
installation, and autostart doctor passed. The setup script passes bash syntax
checking and replays new USB rules for an already-attached iPhone on first install.
