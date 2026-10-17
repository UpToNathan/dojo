# GTK shell version 7 integration

The copied XML and license come from GTK 4.22.4 commit
`7f99ab1a26408b6499a18f353f081e3c0598ea5c`. The protocol identifies the
AT-SPI top-level object using the exact client-owned `wl_surface`.
The compositor does not resolve or invoke D-Bus objects.

GTK's producer is `gtk/gtkapplication-wayland.c::set_a11y_properties` at the
pinned revision. It obtains the realized `GtkAtSpiContext` reference and sets
the GDK top-level properties during GtkApplication window realization. A
GtkApplication using the AT-SPI backend announces its windows automatically.
A bare GTK window outside GtkApplication has no proved automatic producer and
can return no endpoint. `GTK_A11Y=atspi` selects this backend for live fixtures.

| Version | Requests or behavior | Niri policy |
| --- | --- | --- |
| 1 | capabilities | Advertise no global menu, menu bar, or desktop icons. |
| 1 | get_gtk_surface | Associate each wrapper with the exact surface and retain its own annotation. Multiple live wrappers make exact accessibility reads refuse explicitly. |
| 1 | set_dbus_properties | No global menu capability; menu/application metadata does not create accessibility bindings. |
| 1 | set_modal, unset_modal | Advisory hints do not override existing parent placement or input policy. |
| 1 | present | Legacy timestamps cannot grant activation. Clients use xdg-activation. |
| 1 | set_startup_id, system_bell | No legacy startup focus or audible bell policy. These optional notifications have no effect. |
| 1 | configure | Emit corresponding tiling flags before each xdg toplevel configure. |
| 2 | configure_edges | Emit resizable edges before each xdg configure. Fullscreen/maximized windows have none. |
| 3 | notify_launch | Notification only; no compositor launch animation or focus authority. |
| 3 | request_focus | Resolve an existing xdg-activation token and apply existing age, urgency and window activation rules. Unknown tokens have no effect. |
| 4 | release | Remove only the releasing wrapper and its annotation, and increment generation. Repeated destruction callbacks have no effect. |
| 5 | titlebar_gesture | Double click requires the native seat, current exact pointer focus and a matching live pointer grab serial, then uses existing maximize behavior. Right/middle clicks do not request a supported compositor action. Unknown gestures are protocol errors. |
| 6 | surface offsets | Existing handlers/compositor.rs consumes committed SurfaceAttributes.buffer_delta and adjusts floating toplevel positions; cursor hotspots and DnD icons also consume offsets. Tiled layout continues to govern tiled window positions. |
| 7 | set_a11y_properties | Parse a bounded unique AT-SPI bus name and object path. Invalid updates retire the old endpoint and make the target query fail explicitly. Every update/release/unmap changes generation. |

The association is application-authored semantic evidence. The reader must prove
the announced AT-SPI unique owner's credentials match the native Wayland client.
An application that does not announce an endpoint returns explicit absence.
Native semantic actions remain unavailable.

GTK surface wrappers and annotation state live with the native surface.
Each wrapper receives configure events for its negotiated protocol version.
Destruction never clears another wrapper's binding. Unmapping clears every
wrapper's endpoint before a later mapped window identity can reuse that
surface. Destroying a competing wrapper after unmapping cannot restore an old
endpoint. A remaining wrapper must announce a new endpoint after unmapping.
