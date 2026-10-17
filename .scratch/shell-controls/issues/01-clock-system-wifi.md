# Add date/time, system menu and Wi-Fi controls

Status: resolved
Type: task

Implement the [shell controls spec](../spec.md). The requested placement is
top-center date/time, bottom-right system menu, and Wi-Fi left of Bluetooth.

## Comments

The existing frame has independent right-hand controls, explicit click-through
input regions, and native full-output overlays for Bluetooth and display settings.
NetworkManager is active on the dev machine. The similarly named system-menu thread handled
gamepad bindings and has no competing menu source changes.

Implemented the centered clock, native NetworkManager Wi-Fi menu, and compact
bottom-right power menu. The power menu extends from the shell like the other
menus and offers only power/session actions. Removed the display panel's Close
button and matched its reveal to Bluetooth's 150 ms animation, delayed content
fade, stable clipped layout, and reduced-motion handling.

Validation: library and binary suites passed; display UI tests passed after the
animation change. Isolated native UI checks covered menu opening, confirmation
cancellation, dismissal, and reopening. Wi-Fi discovery was checked live; fake
D-Bus tests covered credentials and connection operations without changing the
host connection. Fullscreen regression verified all eight edges and pointer
pass-through. No actual power action was executed.

Follow-up: the display panel's right attachment used a 16 px inset while the
desktop border uses 12 px, leaving a visible step below the curved join. Use
the shared RIGHT_INSET for the panel so both edges meet.

Power menu follow-up: replaced the top-attached outline with a bottom-right
outline that merges into both reserved edges. The free top-left corner is
rounded, and the two attachment corners curve into the frame. Deployed and
visually inspected on the dev machine; power-menu tests and autostart doctor passed.

Wi-Fi follow-up: removed the extra bordered background, aligned its attachment
with RIGHT_INSET, and animated the painted shape along with the clipped content.
The header stays above a separate scrollable network list and a scroll hint.
Deployed and visually inspected on the dev machine; Wi-Fi UI tests and doctor passed.

Icon follow-up: use 18 px centered cells and normalize shell glyphs to a 12 px
maximum visible outline dimension, measured from the bundled VictorMono font.
Tray images use the same 12 px size. Deployed and visually inspected; frame
tests, fullscreen/pointer regression, and autostart doctor passed.
