# Launcher

`dojo --launcher` opens the launcher; it is one layer surface that is hidden and mapped again, never
recreated. See the [README](../README.md#application-launcher) for the basics and the
[performance notes](research/dojo-launcher-performance.md) for how fast it opens.

## What you can do

| Type or choose | Result |
|---|---|
| an application name | the application. Applications you launch often, and the searches that led to them, rank first |
| nothing | **Suggestions** (the commands below, most used first), then your applications by use |
| `!yt cats` or `cats !yt` | opens the search in Helium (`xdg-open` when Helium is not installed). A bare `!yt` opens the site. Triggers are in `data/launcher-bangs.toml` |
| text that matches nothing | a web search in the default engine (DuckDuckGo) |
| `? what is on my calendar` | sends the text to the Dojo agent. The shell shows it and the reply as speech bubbles. This works without a microphone |
| `2 + 2 * 10`, `20% of 150`, `5 km in miles`, `72 f to c` | the answer; Enter copies it |
| `#ff8800`, `rgb(255, 136, 0)`, `hsl(…)`, `oklch(…)` | the colour in every format and its contrast against white and black |
| `uuid`, `base64 hi`, `sha256 hi`, `urlencode a b`, `ts 1700000000`, `json {"a":1}`, `pw 24`, `lorem 20` | the result; Enter copies it. `snake`, `camel`, `kebab`, `pascal`, `upper`, `lower`, `title` followed by text convert it, and those rows follow the applications because the words are also ordinary searches |

## Commands

A command opens a view inside the launcher. Type its name in the launcher, pick it from the
Suggestions, or bind a key to open it directly: `dojo --launcher-view <name>`.

| Name | Command | In the view |
|---|---|---|
| `emoji` | Search Emoji | search by name or keyword; Enter copies; Tab cycles a skin tone; emoji you used come first |
| `clipboard` | Clipboard History | search what `clipvault` recorded; Enter copies it again; Ctrl+P pins; Ctrl+D deletes. Images are not listed, and entries that look like secrets are hidden |
| `windows` | Switch Windows | Niri's windows, most recently focused first; Enter focuses one |
| `snippets` | Snippets | saved text; Enter copies it with `{date}`, `{time}`, `{datetime}`, `{clipboard}` and `{uuid}` filled in; type a new name and choose *Save clipboard as…* to add one |
| `color` | Pick Color | choose *Pick a color from the screen*; the launcher closes, you click, and it reopens with the colour in every format |
| `processes` | Processes | running programs by CPU or memory (Tab switches); Enter then Enter again stops one and its helpers; Ctrl+D twice force-kills it |
| `services` | Services | your systemd user units; Enter restarts or starts; Ctrl+D stops (or clears a failed unit); Shift+Enter shows the last log line. The desktop session's own units are refused |

Escape (or Backspace on an empty search) leaves a view for the root list; Escape again closes the
launcher. **Ctrl+H** on a command in the Suggestions hides it from the empty launcher; Ctrl+H on it
again after finding it by typing shows it again. A hidden command is still found by typing.

## Files

| File | Holds |
|---|---|
| `~/.local/state/dojo/launch-history.json` | what you launched and the text you typed (applications, commands, emoji, bangs) |
| `~/.local/state/dojo/launcher-hidden-commands.json` | commands hidden from the Suggestions |
| `~/.local/state/dojo/clipboard-pins.json` | pinned clipboard entries (they last as long as `clipvault` keeps the entry) |
| `~/.local/state/dojo/picked-color.txt` | the last colour picked |
| `~/.config/dojo/bangs.toml` | your bangs and `default_engine`; yours win over the tracked list |
| `~/.config/dojo/snippets.toml` | your snippets |

`snippets.toml`:

```toml
[[snippet]]
name = "Sign-off"
keywords = ["sig", "regards"]
text = """
Kind regards,
Nathan, {date}
"""
```

`bangs.toml`:

```toml
default_engine = "kagi"

[[bang]]
trigger = "gl"
name = "GitLab"
url = "https://gitlab.com/search?search={}"
```

## How it is built

- `src/launcher_core/` holds the logic with no GPUI in it: ranking (`root.rs`), the emoji, bang,
  calculator, colour, snippet, clipboard, window, process and service engines, and `inline.rs`,
  which decides what each query produces.
- `src/launcher/views/` has one file per command. A view loads data off the UI thread, returns
  rows and answers Enter with an `Outcome` (copy, close, notice, error, or run a job after
  closing). Views reach the system through function pointers so their tests need no desktop.
- A command joins the Suggestions when it is listed in `launcher_core::commands::IMPLEMENTED` and
  its view is built in `launcher/views.rs`.
- `devenv shell -- tools/agent-input/run-nested.sh --launcher-views` opens every view in a nested
  Niri and checks the shell survives using them. It needs `wtype`.

## Not done

Automatic paste into the focused window, expansion of snippets as you type in other applications,
time zones in the calculator, an extension or script-command system, and file search.
