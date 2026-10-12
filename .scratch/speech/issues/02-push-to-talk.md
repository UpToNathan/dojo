# Push-to-talk activation

Type: task
Status: claimed

Holding Copilot or Super+Shift+P records. Releasing the bound key closes the
microphone, transcribes locally and sends immediately. Key repeat must not
toggle recording. A release before the worker finishes starting must not leave
a microphone open. Modifier release order must not affect completion.

Niri needs a spawn-on-release binding property; track release by physical key
code from the original press. Keep current press-only bindings unchanged.
