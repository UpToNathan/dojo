# GPUI Wayland AT-SPI surface annotation

The producer adaptation in `crates/gpui_linux` is based on Zed source revision
`cc94985f7042574f569e0b6c7b6bdd27975a4579` (the checked-out `vendor/zed` HEAD
at implementation time), not the Dojo repository revision.

It uses the locally vendored `accesskit_unix` 0.22.1 from AccessKit revision
`c88605b96d04431f9c3c792464a0f2f253480e94`, path `platforms/unix`, and the
pinned draft protocol XML from wayland-protocols MR !493 commit
`2c4da1cf132e7084e0de8a4a7db4d3af7644b91d`. The XML content hash is recorded
beside the compositor copy at `vendor/niri/resources/xdg-dbus-annotation-v1.README`.

The AccessKit callback reports its authenticated unique bus name and its own
current adapter-root object path. GPUI only forwards that tuple to the owning
Wayland window's event thread and attaches it to that window's `wl_surface`.
