# USB DualSense control

Use the DualSense touchpad as normal system mouse input, the sticks to focus
windows, the D-pad to launch configured applications, and the face buttons for
launcher and focus navigation. Install a graphical-session user service and
preserve existing desktop configuration.

## Design decision

The kernel's joystick axis/button maps hide device indexing; a small Python
service owns controller transitions and delegates window focus to Niri. The
touchpad remains system mouse input. Unassigned buttons have no action.

## Verification

USB discovery and kernel maps checked. The current mapping is documented in
`docs/gamepad.md`; launcher, application shortcuts, and Niri focus navigation
are covered by the adapter tests.
