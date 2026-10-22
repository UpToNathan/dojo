# Dojo

Dojo is a Niri desktop shell that frames the user's workspace and hosts shell surfaces.

## Language

**Desktop frame**:
The rounded surround that visually frames the usable desktop area while leaving the workspace visible inside.

**Reserved edge**:
An edge inset that keeps tiled windows within the desktop frame.

**Frame state**:
Whether the desktop frame and its reserved edges are enabled or disabled.

**Account**:
One credential source that can carry agent model requests, such as a Claude subscription or an OpenCode key. Accounts are discovered independently and any subset may be present.

**Model picker**:
The shell menu that lists the models an available account can serve and sets the agent's global model selection.

### Locking

**Occasion**:
The reason the desktop frame opens or closes: lock, unlock, login, logout, suspend, hibernate, restart or shutdown.
_Avoid_: Event, mode

**Session lock**:
The state in which the desktop is hidden and input is withheld from every application until the user proves their identity.
_Avoid_: Screen lock, lock state

**Lock screen**:
The full-screen surface Dojo shows on every output while the session is locked.
_Avoid_: Screensaver, greeter

**Login screen**:
The lock screen's form shown on a virtual terminal before any session exists. It is the same surface with a user chip and a session chip, drawn with the look of the user named in the user chip.
_Avoid_: Greeter UI, display manager screen

**Display manager**:
Dojo's own root service, `dojo-dm`, that owns the login virtual terminal, authenticates through PAM, opens the logind session and starts the chosen session.
_Avoid_: Greeter, ly

**Look snapshot**:
The copy of a user's palette, pre-blurred wallpaper, display name and greeting settings that the display manager keeps for the login screen. It is taken when a session starts and ends, so the login screen never reads a home directory.
_Avoid_: Theme cache, profile

**Unlock method**:
A way of proving identity at the lock screen: fingerprint or password. Both are live at the same time and the first to succeed unlocks.
_Avoid_: Credential, factor

**Dial**:
The clock inside a tick ring at the centre of the lock screen. It also shows the fingerprint reader's state.
_Avoid_: Ring, clock widget

**Session transition**:
The movement in which the desktop frame closes in over the workspace, or opens back out, when the session changes occasion.
_Avoid_: Lock animation, shutters, iris

**Greeting**:
The one short line shown as the desktop frame opens, on unlock or login.
_Avoid_: Welcome message, splash

**Farewell**:
The one short line shown as the desktop frame closes in, on lock, logout, suspend, hibernate, restart or shutdown.
_Avoid_: Goodbye message, splash

**Active output**:
The output whose lock screen currently takes keyboard input, which is the one under the pointer. Other outputs show only a dim clock.
_Avoid_: Focused monitor

**Password pause**:
A timed refusal of password attempts after repeated failures.
_Avoid_: Lockout

### Global shortcuts

**Global shortcut**:
A key combination an app asks for through the GlobalShortcuts portal, which acts whichever window has focus. The app learns when it goes down and up.
_Avoid_: Hotkey, keybinding (those are configured Niri binds)

**Shortcut grab**:
A global shortcut's key held by Niri on behalf of an IPC connection. Niri consumes the key, so the focused app never sees it, and releases it when the connection closes.
_Avoid_: Bind, registration

**Binding panel**:
The shell surface where the user picks the keys an app's global shortcuts get.
_Avoid_: Shortcut dialog
