# Nerd Font agent cursor

Replace the hand-drawn agent arrow with the actual `nf-fa-mouse_pointer` glyph (U+F245) from Dojo's bundled Victor Mono Nerd Font. Use the active Dojo primary accent, transparent background, and a hotspot at the glyph's top-left tip. Render at 24 logical pixels high with sufficient resolution for fractional and high DPI outputs.

Keep the font-derived cursor asset and its source/license provenance in the Niri subtree so the fork builds independently. Decode the asset only while an agent seat exists; releasing the seat releases its cursor buffer. Preserve input isolation and on-demand worker behavior. Verify the rendered icon and cursor cleanup in a nested compositor, then install the release build for the next login without terminating the active desktop.

The Niri theme export supplies `cursor.agent-color` from the semantic `primary` role. Configuration reload recolors live agent buffers. The embedded glyph supplies alpha coverage only, preserving its shape, size and hotspot.
