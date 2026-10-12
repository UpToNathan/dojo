# Compositor and desktop service efficiency audit

Status: ISSUES. Read-only inspection of production source. No builds, benchmarks, input injection, service changes, or private configuration reads.

Snapshot: HEAD `7fb1e4ffa79843314624783fe8f302c734322caf`, plus a dirty worktree. Findings describe the checked-out source, including uncommitted voice/frame/config work. They do not establish which binaries or generated Home Manager files the current session uses. The parent audit owns live measurements. No speedup, memory savings, or battery savings have been measured here.

Read AGENTS.md, GLOSSARY.md, README.md, system-config/README.md, docs/agents/domain.md, the agent input/worker/cursor motion specs, speech spec, Niri session spec, autostart spec, and relevant source/callers. Upstream renderer code is separated below from Dojo additions.

## Ranked owned findings

### 1. The resident desktop shell and speech worker use development binaries

Verified source: `home-manager.nix:27` autostarts `target/debug/dojo`; `home-manager.nix:536` invokes that same build for launcher control. `tools/speech/voice.mjs:238` spawns `target/debug/dojo-speech` through a second `devenv shell`. `Cargo.toml:30` optimizes only naga, wgpu-core, and gpui_wgpu in the dev profile. Most Rust shell/dependency work retains development optimization levels. The compositor itself already has a separate release installation, `scripts/install-niri-fork.sh:5` and `vendor/niri/Cargo.toml:152`.

Affected scenario: daily shell rendering, native service logic, launcher opening, local transcription. The unoptimized binary selection is proven; its contribution to CPU time, latency, and watts is unmeasured. Debug symbols alone do not prove runtime overhead or resident memory inflation.

Smallest direction: use a maintained release installation/wrapper for the resident Dojo binary and speech binary, matching the existing Niri release ownership pattern. Keep development launch commands available. Preserve full capability and runtime graphics/audio libraries. Replace voice's nested development environment startup with an installed wrapper only after proving its library environment.

Verification: identify `/proc/<pid>/exe` and effective service commands; compare the same shell interactions and transcription workload between dev and release, plus quiet-session CPU/GPU activity and PSS. Validate autostart/frame, speech capture and T3 handoff, launcher/control IPC, and application lifetimes. Do not substitute stripped file size for RSS/PSS or claim release necessarily reduces idle watts.

### 2. Voice wakes once per second even when there is no conversation

Verified source: `tools/speech/voice.mjs:383` defaults idle shutdown to zero; `tools/speech/voice.mjs:384` always starts a 1000 ms interval. The callback returns after activity checks and, with no active conversation and default settings, still wakes every second. `voice-home-manager.nix:6` enables the service at activation; `tools/speech/dojo-voice.service:14` attaches it to graphical-session startup.

Affected scenario: the entire quiet graphical session, including battery operation. This is a definite recurring timer, not a measured large CPU consumer. The interval also checks the 30-second conversation settlement deadline and optional worker shutdown.

Smallest direction: schedule/rearm one deadline timer when capture/run/submission activity transitions establish a settlement deadline. Schedule the optional shutdown deadline only when enabled. Cancel/rearm on follow-ups and active runs, preserving the 30-second conversation contract.

Verification: exercise first speech, follow-ups before/after the 30-second deadline, recording/transcription crossing deadlines, reconnect, active agent runs, cancel, and shutdown. Confirm no voice timer fires while no conversation or shutdown deadline exists. Compare quiet-session scheduler wakeups, CPU, and C-state residency using the parent's measurement workflow.

Prewarming is intentional: `.scratch/speech/spec.md:30` requires model preload at login and retention for the session. `tools/speech/voice.mjs:51` starts the worker immediately. Model idle eviction is an optional battery-mode policy change with first-use latency, not a defect or a default capability-preserving fix. The microphone should remain closed outside recordings per the spec; this inspection did not audit Rust audio worker behavior.

### 3. One moving agent cursor keeps every output animating

Verified owned fork source: `vendor/niri/src/agent_input.rs:695` returns a global `any(motion.is_some())`; `vendor/niri/src/niri.rs:4788` computes that flag for each output and `:4799` includes it in that output's unfinished animation state. `vendor/niri/src/backend/tty.rs:1806` and `:1844` queue another frame while that state is true. Agent commands unconditionally queue every output at `vendor/niri/src/agent_input.rs:444`, including keys, Wait, and unchanged-focus operations. Rendering the cursor itself correctly filters to its current output at `:712` onward.

Affected scenario: multi-monitor sessions during cursor travel; input bursts can schedule unrelated outputs too. This proves broader scheduling than the visible cursor needs. Damage tracking may avoid scanout/GPU work on unchanged outputs, but it does not remove compositor traversal/estimated-vblank scheduling. No magnitude was measured. A one-output laptop has no unrelated output savings.

Smallest direction: make moving-cursor activity and cursor damage output-specific. Track both old and new cursor outputs when crossing boundaries or hiding/resetting; retain the existing one-shot arrival fallback. Remove unconditional all-output redraw for commands that do not change cursor/render state only after confirming focus/activation callers independently damage affected windows.

Verification: two nested outputs; move on one and observe scheduling on the other, move across boundaries, redirect mid-flight, wait/button arrival, disconnect/reset/lock, and hot-unplug. Confirm unrelated output animations stop while independent seats, input coordination, focus activation, and correct old-cursor erasure remain intact.

### 4. Every event-loop refresh inspects stationary agent state

Verified owned fork source: `vendor/niri/src/niri.rs:835` calls `refresh_agents` each refresh. `vendor/niri/src/agent_input.rs:484` allocates the peer list, removes/reinserts each seat through `with_agent`, checks target mapping, output membership, pointer focus roots, and surface contents for every visible stationary cursor. Seats are bounded to 16 at `:129`, and lifecycle is intentionally connection-owned.

Affected scenario: active human/app rendering while an agent seat and stationary cursor remain alive. This is bounded work and lower priority than scheduling/rendering findings. No seat means an empty vector with no heap allocation for its contents. The root cost is unmeasured.

Smallest direction: consider only after profiling attributes meaningful time here. Preserve lock/unmap/output-change cleanup. Event-specific dirty checks or separating active motions from stationary seats may help, but a broad seat architecture rewrite has no current evidence.

Verification: profile compositor refresh with zero, one stationary, one moving, and several seats under the same rendering workload; retain lock/unmap/disconnect and contents-under-pointer behavior. Avoid claiming bounded small scans are automatically a material battery cost.

## Upstream renderer candidates requiring runtime proof

### 5. Small speech blur regions may still capture and blur a large surface

Verified source chain: `src/frame.rs:1278` sends only combined speech bubble regions, and `:1240` clears blur regions when no bubbles exist. `home-manager.nix:615` deliberately selects non-xray backdrop blur to sample windows underneath. Upstream `vendor/niri/src/render_helpers/background_effect.rs:237` keeps the surface geometry as effect geometry while carrying the requested rectangles separately as a subregion. `vendor/niri/src/render_helpers/framebuffer_effect.rs:206` derives framebuffer size from source geometry/cropping, `:279` blits that area, and `:309` blurs it. Subregion filtering happens later during draw at `:355`, with an empty filtered draw short-circuit at `:378`.

Hypothesis: for the full-output Dojo frame with a small bubble, capture/blur work may cover the full surface rather than the requested bubble area. Capture calls in Dojo's upstream render helper use element source/geometry at `vendor/niri/src/render_helpers/mod.rs:454`. Smithay's actual DRM damage/capture cropping and upstream GPU behavior were not inspected or measured, so full-screen recurring blur cost is not asserted as established. This renderer code came from the upstream subtree, `git log` identifies `eabc0b1ddb` for framebuffer_effect.rs; it is not a Dojo-specific rendering regression by itself.

Affected scenario: visible speech bubbles over video, scrolling, or animation, especially large/high-scale outputs. With no bubbles, the explicit empty blur region prevents the effect, `background_effect.rs:224`.

Smallest direction if confirmed: crop capture/blur to the requested region bounds plus blur-kernel padding, or give bubble rendering a bounded layer while preserving the desktop frame. Preserve blur of actual windows and region coordinates. Switching to xray wallpaper blur changes the visual behavior and conflicts with the intended speech appearance.

Verification: inspect actual capture source dimensions and GPU trace for an empty desktop, one stationary bubble, bubble over video, and bubble entry/exit. Verify edge padding, separated bubbles, rounded clipping, scale changes, and capture output. Compare GPU busy time and battery discharge only with controlled identical content.

### 6. Niri has a permanent fallback frame-callback timer

Verified upstream source: `vendor/niri/src/niri.rs:2595` installs a one-second repeating timer. `:5302` creates a dummy Output and traverses window/layer/lock/cursor trees, with `FRAME_CALLBACK_THROTTLE` at `:201`. It exists to deliver fallback frame callbacks and does not itself call queue_redraw_all.

Direction: leave unchanged until a profile demonstrates material cost. Removing or suppressing it can stall clients not reached by normal output frame callbacks. Event-driven arming only for surfaces needing fallback delivery is an upstream design candidate, not an owned-fork quick fix. Verify invisible/minimized clients, lock screens, headless/off outputs, and client progress before changing it.

## Battery and activation gaps

- `home-manager.nix:227` enables wluma for screen-content brightness adaptation, with no ALS hardware at `:236`. It preserves adaptive brightness. Its installed capture cadence, output-size strategy, continuous GPU readback behavior, and whether it is currently running require live/package evidence. Do not delete it as a redundant watcher or mistake its historical D-Bus leak for a new capture issue. The maintained no-reply patch at `:230` already addresses the documented broker exhaustion.
- No explicit output mode/refresh rate or adaptive-sync policy appears in the maintained Niri configuration at `home-manager.nix:522` onward. An included generated theme file is not the complete live config. Query supported/effective modes and VRR, EDID range, actual panel scanout rate, and GPU low-power residency before recommending a battery profile. Lower refresh rates and shorter animations are optional smoothness tradeoffs, not unconditional improvements. VRR support alone does not prove low idle refresh or battery savings.
- `home-manager.nix:258` enables stasis with inhibition/login integration, but this maintained extraConfig declares no explicit screen-off/lock/suspend deadline. The imported module's defaults and runtime config may supply one. Verify effective idle policy, lid behavior, idle inhibitors, and display-off state. Add sensible explicit policy only if a gap is proven; preserve media/session inhibition and lock-before-suspend.
- No explicit CPU energy-performance preference, platform power profile, or GPU power-management policy appears in the inspected maintained system configuration. These may be supplied by the host outside this repository. Read actual driver/profile ownership before adding a competing power daemon. Observe utilization, C-states, GPU clocks/residency, and battery discharge together.
- Voice's outer service starts through devenv and its speech worker starts through another devenv. This proves nested environment setup, not resident overhead after exec. Inspect process trees and activation times before replacing it with installed wrappers; use equivalent library paths.

## Existing efficient choices

- Niri normally redraws queued outputs and retains idle/vblank states rather than continuously rendering every output, `vendor/niri/src/niri.rs:3848` and `backend/tty.rs:1806`. Agent motion ends by clearing motion and uses a one-shot arrival fallback, `agent_input.rs:225` and `:514`.
- Image-copy capture checks pending frames/output identity before rendering, caches render elements for cursor/no-cursor variants, and waits for damage, `vendor/niri/src/niri.rs:5600`, `:5605`, and `:5646`. Calling capture handlers in redraw does not prove continuous screen capture without consumers.
- EffectBuffer lazily allocates reusable GPU buffers, keeps blurred results until damage invalidates them, and reuses element storage, `vendor/niri/src/render_helpers/effect_buffer.rs:230`, `:251`, `:268`, and `:311`. Blur textures are reused across renders, `blur.rs:140`.
- Production Niri keeps D-Bus/systemd/screencast default features and builds release with thin LTO, `vendor/niri/Cargo.toml:132`, `:152`. Turning those off would remove required session/capture support. Tracy features are optional at `:140`, so instrumentation names in source do not prove an active profiler.
- Clipboard history and clipboard persistence have distinct capabilities, `home-manager.nix:347`, `:349`, `:363`. They are graphical-session-owned and should not be collapsed solely because both observe clipboard changes.
- Speech Dispatcher is socket-activated, `home-manager.nix:313`, with no service WantedBy declaration. wlsunset has no automatic WantedBy and is toggled on demand, `:328`. Vigiland is explicitly user-toggled, `:122`. Installed packages alone do not imply active daemons.
- T3 standalone server autostart checks and enforces one backend owner, `t3-home-manager.nix:25` and `:94`. Preserve remote clients and voice integration before considering on-demand activation.
- GNOME Keyring starts only its secrets component, `home-manager.nix:276`; GCR prompt services remain D-Bus-activated. Keep required secret/polkit support.

## Result

Proceed first with release deployment evidence, the voice deadline timer, and output-specific agent animation scheduling. Profile bubble blur and wluma capture before renderer changes. Battery policy decisions need effective host/output measurements. Source inspection establishes avoidable scheduling and development-binary defaults; it establishes no numeric savings.
