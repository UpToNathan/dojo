# Fullscreen frame visibility

Fullscreen applications must cover the Dojo desktop frame on their output. Leaving fullscreen reveals the same frame and retains its edge reservations. Other outputs retain their frames. Launcher surfaces remain independent.

The frame belongs to the layer-shell Top layer, which Niri places behind fullscreen clients. Its negative exclusive zone keeps it outside the positive edge reservations. Verify real rendered pixels with the nested fullscreen regression; record the failing baseline before changing layer placement.
