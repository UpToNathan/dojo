- **Overlays can hand focus to another app.** New: `rune::press_activation_token(app_id)`, an
  XDG activation token from the latest pointer press on a Wayland layer-shell overlay, made with
  that press's serial, seat and surface. A notification server emits it as `ActivationToken`
  before `ActionInvoked`, so the app it opens may raise its window. None off Wayland, before any
  press, or when the compositor lacks `xdg_activation_v1`.
