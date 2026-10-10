# Dojo launcher: Fuzzel responsiveness, Raycast-style extensibility, GPUI throughout

## Findings

- **Keep all launcher UI in GPUI.** This is a user requirement, not a toolkit decision. Extensions should supply results, action descriptions, and optional host-defined detail/form data; Dojo renders them. React/WebView UI is neither required nor recommended.
- **Start with a warm launcher surface owned by the existing Dojo shell, native application search, and out-of-process extensions.** Keep application metadata and search state resident; do not discover applications, load an interpreter, or wait for a remote provider on every invocation. This is an architectural recommendation, not a measured speed advantage.
- **Separate visibility lifetime, index lifetime, and extension lifetime.** A separate GPUI process can also be warm. Conversely, a resident shell can still incur expensive window creation. Benchmark those independently before choosing a permanent process boundary.
- **The pinned GPUI already implements layer-shell, text-input-v3, and an AccessKit Unix adapter.** That makes the approach plausible, but does not establish correct launcher focus, IME composition, screen-reader semantics, or a supported hide/remap API. Those are early verification gates, not finishing touches. [G1–G3]
- **Use Raycast's interaction model without promising its ecosystem.** Native lists, stable selection, primary/secondary actions, contextual action menus, loading states, and explicit command entry are high-value ideas. Full Raycast API compatibility is a much larger commitment than running JavaScript. [R1–R4, V1]
- **Begin extensibility with trusted scripts; add persistent workers only where needed.** Make query cancellation, stale-result rejection, result limits, and host-controlled actions part of the conceptual contract. WASM becomes compelling if running untrusted extensions with enforceable capabilities is an actual requirement. [R2, P1, W1–W3]

Research date: 2026-10-09. **No benchmarks were run.** “Fast,” “lightweight,” and “high-performance” in upstream descriptions are product claims, not verified comparative measurements. Numerical defaults below are documented settings, not latency measurements. All recommendations and proposed experiments are explicitly design judgments.

## Local context and fixed constraints

Dojo's glossary describes a Niri desktop shell that hosts shell surfaces. Its desktop frame and reserved edges are existing concepts; the launcher should be a separate shell surface, not a change to frame state. `src/main.rs` starts the shell without a control argument, while `src/shell.rs` maintains shell/display services and `src/frame.rs` opens GPUI layer-shell windows. `Cargo.toml` pins GPUI and its Wayland platform to Zed revision `9ab0715969e9854f2efe61a9d782e7698e5fe6d2`. Local checkout HEAD was `01f8a82fbed8907a90a2dd0daff35f1ca666b48e`; these are local implementation observations, not upstream performance claims. [D1]

This report treats GPUI as mandatory in either the existing shell process or a separate launcher process. It proposes no configuration schema, implementation, package installation, session changes, or restart.

The user also requires asynchronous work where it is advantageous, not everywhere by default. As a design recommendation, use cancellable asynchronous requests for I/O and extension providers, background execution for expensive indexing or matching, and direct execution for cheap local operations within a measured UI-thread budget. An async function alone does not move CPU-heavy work off the UI thread. All GPUI rendering and entity updates remain on the foreground thread; stale provider replies must be rejected independently of whether cancellation succeeded.

## What Fuzzel actually does

### Discovery and index lifetime

Fuzzel recursively scans `applications/` under the XDG data locations, constructs desktop-file IDs from relative paths, parses desktop entries and localized strings, and builds searchable application records. Its parser handles `TryExec`, `Hidden`, `NoDisplay`, and optionally desktop-environment visibility filtering; desktop actions and executables from `PATH` are optional additions. Default search fields are filename, name, and generic name; additional fields include executable, keywords, categories, and comment. [F1, F2]

In `main.c`, `populate_apps` runs on a population thread, invokes `xdg_find_programs`, reads popularity data, and sends an application-loaded event **before** loading the icon theme and resolving icon paths. The main event handler refreshes matches/UI from application-loading events; dmenu input can produce partial-loading events. This is useful separation of text readiness from icon readiness, not evidence that every startup maps before discovery completes. [F3]

The persistent cache stores application launch counts, not a serialized complete application/file index. Normal invocation performs application population again. The inspected entry point initializes rendering/Wayland, starts population, runs the interaction loop, writes history, and cleans up; it is not a long-lived launcher daemon in this code path. A runtime-directory lock prevents concurrent instances. Do not describe Fuzzel's popularity cache as a warm index or assume repeated invocation avoids discovery. [F1, F3]

### Matching and ranking

Fuzzel documents exact, fzf-style, and Levenshtein-based matching, with fzf-style as the default. `match.c` implements optional workers, waits for their completion, and sorts matching records with `qsort`; worker threads therefore do not by themselves prove nonblocking per-keystroke UI execution. The comparator considers match score, word boundaries, match fragments, and launch counts among its ordering criteria. Popularity is not simply allowed to override all textual relevance. [F2, F4]

The documented large-list behavior defaults to a 300 ms filtering delay above a 20,000-entry/match threshold; exact trigger conditions are in `matches_update` and related loading paths. Sorting can be disabled, ignoring ranking comparisons while still updating the cache. These are explicit throughput/interaction tradeoffs, not proof of uniformly instant filtering on huge indexes. [F2, F4]

### Rendering and interaction lifetime

Fuzzel uses fcft text/font handling and Pixman image compositing in `render.c`, and Wayland shared-memory buffers in its surface rendering path. It exposes separate matching/render-worker counts, both documented as defaulting to available logical CPUs; upstream explicitly cautions that this is not always best. Its implementation is a useful small rendering baseline, not a reason to change Dojo's fixed GPUI renderer. [F2, F5]

Fuzzel normally runs for an invocation, accepts or cancels, and exits. Dmenu reads entries from stdin and returns a selection on stdout; it is a general picker interface, not a persistent provider API. It supports desktop startup notification, IME through text-input-v3, overlay/top layer selection, exclusive/on-demand keyboard focus, and default exit on keyboard-focus loss. `--print-timing-info` exposes internal timings, but those are not automatically compositor presentation timestamps. [F1, F3, F5]

**Transfer to Dojo:** keep the small app-search critical path, metadata-before-icons readiness, useful search aliases/keywords, and straightforward keyboard selection. Do not copy a large-list debounce indiscriminately into ordinary application search, nor conflate a stdin picker with asynchronous live providers.

### Direct follow-up inspection: catalogue aggregation

Rechecked `xdg.c`, `main.c`, `path.c`, and `application.c` against master revision `815d4385afb844f74243a6bc1c7836c4fbebe830` in response to the user's explicit request to inspect Fuzzel's code. This is source inspection, not benchmarking. [F3, F6]

- `xdg_data_dirs()` yields the user data directory first, then the configured system data directories in order (or `/usr/local/share`, `/usr/share` when the variable is absent). `xdg_find_programs()` scans each root's `applications/` recursively using directory descriptors, `fstatat`, and `openat`. Nested components are joined with hyphens to form the desktop-file ID. [F3]
- `scan_dir()` checks previously accumulated application records by a **linear ID scan** before parsing each desktop entry. Entries made invisible by `Hidden`, `NoDisplay`, or failed `TryExec` are still represented, so a higher-priority hidden record can prevent a lower-priority duplicate from appearing. Do not remove hidden entries before resolving precedence. [F3]
- Parsing collects localized names, generic names, comments, keywords, categories, icons, working directory, executable text, terminal flags, and optional desktop actions. Desktop filtering is conditional. Searchable fields and filename basenames are converted to lowercase UTF-32 representations before query matching. It is a custom C parser, not GIO-based discovery. This does not establish complete desktop-entry spec compliance. [F3]
- Discovery accumulates a linked list and publishes an alphabetically sorted application pointer array under the application-list lock. `populate_apps()` runs on a population thread, optionally adds PATH programs and reads popularity history, then emits `EVENT_APPS_ALL_LOADED` **before** icon theme/path lookup. [F3]
- `path_find_programs()` is a distinct optional discovery path. It scans PATH directories, uses regular-file/user-executable-bit checks, deduplicates its own records by title with a linear list scan, and appends them to the desktop-app array. It does not turn arbitrary command-line tools into terminal-aware desktop entries. [F6]
- `application_execute()` ultimately uses `execvp` after preparing arguments, with launch-prefix/activation handling. Discovery and launching are separate responsibilities even in this implementation. Dojo should not equate copying the inventory pipeline with inheriting launch correctness. [F6]

**Proposed improvements for a resident Dojo:** retain a ready catalogue between invocations; use desktop-ID-keyed precedence resolution during construction followed by a compact searchable snapshot; background invalidation/rebuilds rather than rescanning on open; preserve metadata-before-icons readiness. Hash-map construction, compact storage, and persistence are hypotheses to measure, not proven faster than Fuzzel on a normal small application catalogue. SIMD is relevant to downstream scans/matching, not automatically to filesystem traversal or desktop-file parsing.

**F6 — aggregation/launch follow-up:** [path.c](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/path.c), `path_find_programs`; [application.c](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/application.c), `application_execute`.

## What Raycast provides—and what its documentation does not promise

### Runtime and lifecycle

Raycast's security documentation says it runs a single child Node.js process, communicates over standard file handles using a thin RPC protocol, and runs an extension in its own V8 isolate/worker thread with an event loop and limited heap. Its lifecycle docs say commands execute on launch and unload from memory when left or completed; commands exceeding memory limits are terminated. These are documented runtime properties, **not source-verified details of Raycast's proprietary host or quantified startup/memory costs**. No fixed heap size is established here. [R1, R2]

View commands export a React component; no-view commands can export an async function. Root-search command execution, aliases, programmatic launch, fallbacks, deeplinks, and background launches are documented entry paths. This supports an important product distinction: searching for a command at the root need not run every command's domain query. [R1]

Background refresh is a separate opt-in scheduling mechanism for no-view/menu-bar commands. The documented minimum interval is 10 seconds, scheduling is not exact, and runs can be terminated after an interval-dependent timeout. Do not interpret “extensions” as necessarily persistent background processes. [R5]

### Search and action UI

Raycast `List` indexes item titles and keywords for built-in fuzzy filtering. Custom query handlers can turn native filtering off; async search can use throttling, loading indicators, and pagination. Pagination is subject to memory-protection heuristics. `ActionPanel` exposes selected-item actions, semantic groups, shortcuts, and lazy submenus; first/second actions receive primary/secondary keyboard behavior. [R3, R4]

**Recommendation:** adopt that interaction vocabulary in GPUI, with a narrow host-rendered result/action model. Start with a list plus contextual actions and optional details; arbitrary extension widget trees, navigation stacks, and forms should be justified by actual desired commands. Matching Raycast's React API, navigation semantics, utilities, storage, macOS-specific APIs, and packaging is a separate compatibility project—not a prerequisite for “Fuzzel meets Raycast.” [R1–R4, V1]

### Permissions

Raycast explicitly says its extensions are **not further sandboxed** for filesystem I/O, networking, or other Node capabilities. Some access is gated by macOS permissions granted to the parent app. Store review and a restricted host RPC interface do not make arbitrary extension code harmless. These macOS protections cannot simply be assumed to exist for a Linux launcher. [R2]

**Recommendation:** distinguish (1) trusted user scripts, (2) process fault containment/resource limits, and (3) malicious-code security isolation. If third-party/untrusted extensions are desired, define the threat model before choosing a runtime. Host-confirmed actions cannot stop a trusted-mode script from independently calling the filesystem or network.

## Two Linux comparisons worth learning from

- **Vicinae:** its first-party extension documentation says TypeScript/React produces a serialized UI tree rendered by the native C++ core, with no browser, and exposes standard Node APIs. It describes Raycast API compatibility as a long-term goal, not universal compatibility. Its pinned Node runtime source locates/manages Node and verifies downloaded archives by SHA-256. This demonstrates that a JS authoring layer does not require browser UI, but also exposes runtime distribution and API maintenance obligations. In Dojo, any analogous data/tree would still be rendered by GPUI; starting with simple result/action descriptions is a smaller commitment. [V1, V2]
- **Vicinae file search:** first startup performs a full scan of configured locations (home by default), subsequent indexing is incremental, and root file search is asynchronous and explicitly enabled. Its docs warn of substantial initial indexing CPU. That supports keeping file indexing distinct from opening the launcher and from small app search; it does not establish exact latency, index format, or memory usage. [V3]
- **Walker/Elephant:** Walker's pinned README requires Elephant to be running; its source supports service mode, showing a resident window on invocation instead of always exiting. Its query protobuf describes provider-scoped requests, a maximum result count, and structured results. This is a relevant UI/backend split, not evidence that extra IPC is faster or that either process is memory-cheap. Walker's toolkit is only comparative context, never a proposed replacement for GPUI. [L1–L3]

No broad launcher catalogue or cross-project benchmark ranking is implied.

## Process architecture within GPUI

The table is a design analysis, not measurements.

| Architecture | Expected benefit to test | Cost/risk to test |
| --- | --- | --- |
| GPUI surface in resident Dojo shell | Can reuse the initialized platform/renderer and resident app data; straightforward shell integration | UI-thread stalls or host failure can affect the desktop frame; incremental idle memory and retained GPU resources |
| Separate resident GPUI launcher | Independent UI lifecycle/failure boundary; warm requests need not create a new process | Another platform/renderer instance, IPC/control lifecycle, extra resident resources; total memory must include both processes |
| Separate GPUI process per invocation | UI resources can be released on exit; independent app lifecycle | Process/platform/renderer/window/index initialization can fall on the hotkey critical path unless another service retains the index |

**Provisional recommendation:** same-shell GPUI UI, a resident native app snapshot, background native search/index maintenance, and separately supervised extension workers. Keep the launcher module logically independent so it can later move to a resident GPUI process if measured stalls, memory behavior, or crash isolation justify it. Heavy file indexing is a natural candidate for a helper independently of where UI lives.

“Warm” has at least three meanings: warm process, warm data/runtime, and reusable mapped/unmapped surface. **Do not assume GPUI currently exposes a correct hidden-layer-window API.** In the pinned Wayland implementation, layer-shell keyboard interactivity is set during creation; the inspected code exposes visibility reporting, but did not establish a supported generic layer-shell hide/remap or runtime-interactivity setter. Protocol capability is not equivalent to toolkit API support. [G1, G3]

A safe initial fallback worth benchmarking is **resident process/index, create the small launcher surface on show, destroy it on dismiss**. It retains most data/runtime warmth without leaving an invisible exclusive keyboard surface alive. Reusing an unmapped surface might win later, but needs actual platform support and correct configure/commit sequencing. The layer-shell protocol explicitly unmaps on null-buffer attachment, resets state on unmap, and requires a new initial no-buffer commit/configure before remapping. [Y1]

## Wayland/Niri correctness gates

1. **Use a dedicated launcher layer surface.** Do not draw it inside the desktop frame's full-screen surface or inherit reserved-edge geometry. Positive exclusive zone reserves workspace area; keyboard interactivity is an unrelated property. Choose no positive reservation; decide deliberately between zone 0 (avoid other reservations) and -1 (ignore them). [D1, Y1]
2. **Overlay versus top is functional.** Niri's own docs say fullscreen windows cover/prioritize focus over top-layer components; only overlay is above fullscreen. Recommend overlay if the launcher must work over fullscreen, with explicit dismissal and lock-state tests. Neither overlay naming nor Fuzzel's warning is a substitute for validating actual session-lock protection. [N1, F1]
3. **Request exclusive keyboard focus only while visible.** Layer-shell grants exclusive focus to the highest relevant exclusive layer and compositor-selects among contenders; on-demand semantics are implementation-defined. Test repeat-hotkey toggle, Escape, outside click, focus loss, competing panels, workspace/output changes, fullscreen, and return to the previously focused app. Do not promise all compositor global bindings are suppressed. Invisible alpha is not unmapping or focus release. [Y1]
4. **Handle launch activation deliberately.** XDG activation tokens use an initiating surface and input/focus serial; the compositor may reject a token/request. Pass tokens via `XDG_ACTIVATION_TOKEN` for spawned clients or `activation-token` in D-Bus platform data. Acquire the launch context before destroying the source surface. The pinned GPUI URI/reveal helpers obtain tokens through a mouse-focused window and mouse-press serial; that is not proof they correctly cover keyboard-only desktop launching. [Y2, X4, G2]
5. **IME needs a real editable text control.** GPUI's Wayland client implements text-input-v3 and preedit/commit integration; the platform input-handler API supplies editable text/selection/composition behavior. Test Fcitx/IBus, CJK candidates, preedit replacement, compose/dead keys, paste, UTF-8/UTF-16 range conversion, candidate position under scaling, and Escape/Enter while composing. Candidate acceptance must not accidentally launch the selected application. [G2, G3]
6. **Accessibility must be built into host-rendered controls.** The pinned Wayland window has an `accesskit_unix::Adapter` and tree/action update plumbing. That does not automatically give a custom search/list meaningful semantics. Test searchbox labeling, result names/roles, selected state, keyboard focus, result/loading announcements without per-keystroke spam, action names, and operation through Orca/AT-SPI. Keyboard usability alone is not screen-reader accessibility. [G1, G3]
7. **Privacy and session lifetime are separate.** Give the launcher its own namespace; Niri documents namespace rules and block-out-from screen capture behavior. Consider sensitive results separately from public app names. Hide/cancel on session lock and ensure no exclusive keyboard state survives dismiss/resume/output removal; this is recommended behavior, not a verified existing Dojo capability. [N2]

## Desktop entry discovery and launching

Correct app launching is more than splitting `Exec` on whitespace or invoking `sh -c`.

- Use XDG data search precedence and desktop-file IDs, including nested relative paths and user overrides. `Hidden` is a tombstone-like removal override; `NoDisplay` is menu visibility, not the same thing as nonexistent/unlaunchable. Observe `Type`, `TryExec`, `OnlyShowIn`/`NotShowIn`, localization, names/keywords, and icon metadata. [X1–X3]
- Respect desktop-entry quoting/escaping and field expansion: `%f/%F` versus `%u/%U`, `%i` expanding to two arguments, `%c`, `%k`, and `%%`. Multi-file substitutions preserve argument boundaries. This syntax is not a generic shell command language. [X3]
- Account for `Terminal=true`, `Path`, optional startup notification, desktop actions, and `DBusActivatable`. The D-Bus interface distinguishes Activate, Open, and ActivateAction, including activation platform data. Flatpak/exported entries should be launched through their desktop semantics, not by guessing executables from display names. [X2, X4, X5]
- Consider GIO's desktop-app backend as one launch/discovery implementation option: it provides desktop-file-ID lookup, visibility checks, action enumeration/launching, and URI launch APIs. This does **not** require GTK UI; it does add a GLib/GIO dependency and launch-context integration work. Alternatively, audit a Rust implementation against the same conformance fixtures. No particular dependency is selected by this report. [X6]

**Recommendation:** retain stable app/action identities in the search index and route selection through an explicit desktop launcher boundary. Keep “execute arbitrary command text” a separate, intentional action, never an implicit interpretation of an app result or extension label.

## Indexing, ranking, and async providers

The following is proposed behavior, not a description of existing Dojo code.

### Native application path

Build a compact immutable application snapshot off the UI thread; normalize searchable metadata once. Show available names/results first, resolve/decode visible icons lazily, and bound icon/preview caches. Publish replacement snapshots atomically rather than repeatedly blocking readers during scan. Keep filesystem watches plus periodic/recovery rescans conceptually separate: directory changes, watch overflow, package installs, locale/icon-theme changes, and exported app roots all need invalidation policy.

Keep **catalogue discovery**, **matching**, and **rendering** separate budgets. Virtualize visible rows and return bounded top results rather than building/rendering one UI node for every indexed record. A file index should be separately scoped, incrementally maintained, and opt-in; do not recursively scan home on every keypress or make it a prerequisite to the first launcher frame. Vicinae's initial/incremental split and Walker's explicit query limit are relevant precedents. [V3, L3]

### Ranking and history

Prefer strong textual relevance, explicit pins/aliases, then a bounded recency/frequency boost with stable tie-breaking. This is a recommendation, not Fuzzel's exact algorithm or a recovered Raycast root-ranking formula. Fuzzel verifies that launch-count history can be small and useful; Raycast's cited docs do not expose a complete root-search ranking algorithm. [F1, F4, R3]

Use stable identities (desktop ID plus action, provider plus result ID), not translated labels, for history. Persist action history asynchronously and only after the host reports an accepted dispatch; do not call that proof an application became visibly ready. Offer clear/reset/disable behavior and avoid persisting arbitrary queries, secret results, or clipboard content by default. Check that history improves known-item retrieval without letting a popular weak match bury an exact new match.

### Provider scheduling and cancellation

Use an independent monotonically increasing query/session generation. Each provider reply/partial batch belongs to that generation; stale replies must be rejected even when cancellation was ignored. Keep local app results immediate and slow providers independent. Do not wait for all providers before publishing a frame or combine independent tasks into an all-or-nothing deadline.

Cancellation has distinct levels: stop displaying stale data; cooperatively abort a request; terminate/recycle an unresponsive worker when necessary. Dropping a host future alone is not a guarantee external computation has stopped. JSON-RPC itself does not define this whole lifecycle; LSP's cancellation/progress extensions are an appropriate primary-source model to learn from, not a recommendation to implement the full LSP. [P1]

Use per-provider request/result/byte limits, bounded queues, timeouts, backpressure, and limited concurrency. Debounce remote or expensive providers separately from cheap local matching. Batch partial updates rather than repainting for every result. Preserve selection by stable ID; avoid changing the item beneath Enter when a late provider inserts results. Lazy actions/details reduce work before selection. “Cancel” must not roll back a side effect that already executed; query and action lifecycles should be distinct.

## Extension interface choices

Every option below supplies data/actions for **host-rendered GPUI**. Relative performance/memory descriptions are hypotheses to measure; no runtime is declared universally fastest.

| Interface | Good fit | Performance/memory tradeoff | Security/operational tradeoff |
| --- | --- | --- | --- |
| One-shot stdin/stdout script | Personal commands, small static lists, infrequent providers, any installed language | Releases process resources on exit; repeated process/interpreter/import startup can enter query latency; cache static output or run on explicit entry instead of spawning per keypress | Easy process timeout/termination, but ordinary user permissions unless externally confined; separate protocol stdout from logs, bound output, drain pipes, handle child descendants |
| Long-lived JSON-RPC worker over stdio | Interactive remote search, reusable state/clients, costly interpreter startup | Amortizes initialization; persistent heaps/caches and idle processes cost memory; request serialization/batching/backpressure matter | Explicit initialize/query/cancel/action/shutdown lifecycle and version negotiation; process boundary contains many crashes, not arbitrary filesystem/network authority; idle eviction and controlled crash recovery |
| WASM component/module with narrow host imports | Untrusted portable extensions, compute-oriented providers, enforceable host capabilities | Compilation/instantiation, engine footprint, linear memory, and host-call/copy costs all need measurement; compiled-module caching may help | Wasmtime sandboxing is useful only with carefully limited imports/WASI capabilities; CPU fuel/epochs and memory limiters exist, but host calls and host allocations need separate limits [W1–W3] |
| Managed JS/TS worker | Desired JS ecosystem, rich network integrations | Warm runtime avoids repeated initialization; runtime distribution, heap retention, worker scheduling and dependencies add costs | Raycast explicitly leaves Node I/O/network unsandboxed; Node's permission model warns it is not malicious-code isolation. Treat trusted scripts honestly or add an OS sandbox, not “safe because worker/isolate.” [R2, W4] |

**Recommended sequence:** native built-ins → trusted one-shot result/action scripts → optional lazy persistent worker mode for measured expensive providers. Keep one transport-neutral host result/action contract so a language runtime can change without redesigning the GPUI UI. Delay WASM and JS package ecosystems until desired extensions or a real untrusted-code requirement justify their permanent complexity.

Conceptually, extensions need stable result IDs, display metadata, action handles, query/session correlation, readiness/errors, bounded partial results, and cancellation. This is an interface requirements list, **not a prescribed JSON/configuration schema**. Validate incoming values and sizes; do not deserialize extension output into executable shell commands by default. Host actions should mediate desktop launch, URL/file opening, clipboard writes, and destructive-operation confirmation where possible, while clearly stating that trusted-mode workers still have their own OS authority.

## Benchmark methodology before deciding “really fast”

### User requirement: at least Fuzzel-level performance; SIMD investigation

The user explicitly requires a launcher at least as performant as Fuzzel and wants SIMD-aware implementation considered. Treat this as an acceptance target to measure, not a proven property or a commitment to handwritten intrinsics. Compare invocation-to-usable-frame and input-to-presented-results under disclosed equivalent workloads, including tail latency, CPU, and memory. A resident Dojo versus per-invocation Fuzzel comparison is a valid user experience comparison, but must be labeled as different lifecycle costs; isolate matching separately.

**Verified candidate, not selected dependency:** Nucleo's ASCII prefilter uses `memchr`, `memchr2`, `memrchr`, and `memrchr2`; its case-insensitive scan searches lower/upper ASCII bytes. Its matcher separates scoring from highlight-index computation and documents reusable scratch storage (approximately 135 KB per matcher in the inspected source), with no matching allocations except potential index-vector growth. Non-ASCII matching has a separate path. The low-level crate explicitly warns against naive direct UI-loop integration and recommends the high-level engine for interactive larger lists. [S1–S3]

`memchr` documents runtime AVX2 detection with its `std` feature on x86-64, an SSE2 alternative, accelerated aarch64/wasm32 paths, and SWAR fallback. This establishes SIMD-accelerated search primitives, **not** a fully vectorized fuzzy scoring algorithm or a guaranteed win on short application names. Nucleo's published comparisons are explicitly described by upstream as unscientific/incomplete; they compare fzf/skim, not Fuzzel. They cannot establish this acceptance target. [S1, S4]

**Proposed evaluation:** benchmark Nucleo's managed engine, an appropriately scheduled reusable low-level matcher, and Fuzzel against the same candidate corpus and comparable query semantics. Preprocess candidate strings once; keep bytes/IDs compact, reject impossible matches before scoring, reuse scratch/result buffers, compute highlight indices only for displayed results, and evaluate bounded top-k selection against full sorting. Use worker scheduling only when measured useful: extra threads and task dispatch can cost more than a small catalogue scan. Preserve Unicode correctness and ranking quality; prefix/substring speed alone is not an equivalent fuzzy-search benchmark. Implement custom intrinsics only after profiling identifies a bottleneck that maintained libraries do not handle well. No benchmark or implementation was performed for this research update.

Sources inspected on 2026-10-09 (live master links, not pinned dependency selections):
- **S1:** [Nucleo README](https://github.com/helix-editor/nucleo/blob/master/README.md), implementation details and benchmark caveats.
- **S2:** [Nucleo matcher prefilter](https://github.com/helix-editor/nucleo/blob/master/matcher/src/prefilter.rs), ASCII byte scans and non-ASCII path.
- **S3:** [Nucleo matcher implementation/documentation](https://github.com/helix-editor/nucleo/blob/master/matcher/src/lib.rs), scratch allocation, scoring versus indices, and high-level integration warning.
- **S4:** [memchr README](https://github.com/BurntSushi/memchr/blob/master/README.md), SIMD/runtime detection, algorithm selection, and prebuilt versus one-shot search.

No target numbers are prescribed as achieved facts. Agree on desired budgets with the user, then run release builds on the actual Niri machine.

### Define and instrument boundaries

- **Invocation → first presented frame:** record both the hotkey/control-request origin and host receipt, GPUI window creation/map/configure, first render submission, and actual presentation when possible. A frame callback or buffer commit is not itself proof the user saw the frame. The presentation-time protocol reports presented/discarded feedback; if toolkit instrumentation cannot expose it, use a compositor trace or high-speed external camera and disclose its resolution. [Y3]
- **Invocation → usable frame:** separately measure a focused editable searchbox, useful initial application results, and icon completion. A fast blank frame must not count as “launcher ready.” Include missed first keys during focus acquisition.
- **Input → results:** mark key/input receipt, query publication, provider start/finish, merge, and presentation of the corresponding generation. Report first useful local results separately from last-provider completion, and compute p50/p95/p99/worst, not only averages. Measure both committed text and IME composition behavior.
- **Memory/CPU:** baseline resident Dojo without launcher state, hidden warm launcher, visible idle, active search, after repeated hide/show, and after worker eviction. Record per-process and summed PSS/RSS (including children), allocator/heap information where available, GPU allocations separately, idle CPU/wakeups, and peak indexing memory. VM address-space reservation is not resident physical memory; do not compare JS heaps/WASM linear limits to total desktop RSS.

### Controlled matrix

Compare **same-shell GPUI**, **resident separate GPUI**, and **per-invocation separate GPUI**, holding renderer/content/matcher/providers constant where practical. Within resident variants compare surface recreation against verified supported unmap/remap. Record process/index/worker warmth independently.

Distinguish (a) new process with warm filesystem caches, (b) cold application/index/runtime caches, and (c) genuinely cold OS page cache. Do not call the first “fully cold.” Avoid dropping global caches on the user's live desktop; use an isolated disposable session/environment for destructive cache experiments. Alternate/randomize trial order, disclose warmup exclusion, and collect enough repetitions to make tail estimates meaningful (hundreds where feasible).

Record CPU/GPU, power mode, refresh rate, scale, output topology, compositor/toolkit revisions, build profile, provider versions, index size/bytes, and cache state. Repeat under normal background load and during indexing; frame pacing constrains visible latency even if CPU matching is short.

### Workloads and failure cases

1. Real app inventory plus synthetic 1k/10k/100k desktop-entry fixtures. Separate scan/parse/normalization, match-only snapshot tests, and full UI tests. Include duplicates/overrides, Hidden, NoDisplay, missing TryExec, localized names, missing/large icons, terminal apps, actions, unusual quoting, URI arguments, D-Bus activation, and exported sandbox-app entries.
2. File index at 100k/1m records and a larger fixture if relevant. Measure initial scan throughput/time-to-first-searchable-batch, incremental changes, disk/index bytes, resident bytes per record, top-k latency, broad/empty queries, deletions/renames, and update activity during typing. Separate metadata/name search from full-content search.
3. Empty query, common one-letter query, exact/prefix/subsequence matches, Unicode, rapid typing/backspace, and query edits that invalidate incremental narrowing. Measure ranking quality against expected known-item queries as well as latency.
4. Artificial providers delayed by 50 ms/500 ms/5 s, reordered replies, cancellation ignored, huge batches, malformed/oversized output, hangs, worker crashes, and network failure. Confirm app-search latency remains acceptable, no old query replaces new results, no selection/action race, and shutdown/idle eviction leaves no resource leak. These delays are test inputs, not reported product measurements.
5. Hundreds of show/type/dismiss cycles; compare memory before/after settling. Cover fullscreen, multiple monitors, fractional scaling, lock/unlock, suspend/resume, output removal, IME candidates, screen-reader operation, and two competing exclusive surfaces. A timing win with incorrect keyboard focus is a failed experiment.

Fuzzel's `--print-timing-info` can supply an independent sanity-check baseline on the same machine, distinguishing internal timings from end-to-end presentation. Do not compare a warm Dojo toggle against a cold Fuzzel spawn without labeling the differing lifecycle; compare equivalent content/icons/matching settings or disclose differences. Raycast/macOS is interaction inspiration, not a controlled Linux renderer benchmark. [F1]

## Uncertainties and high-leverage questions

1. **What are the first three things beyond launching apps?** Window switching, calculator, personal scripts, file search, clipboard history, and remote APIs imply very different indexing/security requirements.
2. **“Somewhat extensible” means trusted personal scripts or third-party installs?** This determines whether sandboxing/WASM is necessary or a distraction.
3. **Do you want Raycast extensions to run, or Raycast-like interactions?** Compatibility affects runtime, API scope, UI description vocabulary, and long-term maintenance far more than JS versus Rust alone.
4. **Which matters more: minimum idle memory or minimum warm latency?** A hidden resident UI/index/runtime trades idle resources for avoiding repeated initialization. Quantitative budgets are not yet set.
5. **Must root search federate files/network providers, or can users enter an explicit command/provider first?** Explicit entry avoids running everything for every keystroke and simplifies ranking/selection stability.
6. **Must it work above fullscreen, and how should it dismiss/restore focus?** Confirm exclusive versus click-away behavior and the active-output policy.

Technical unknowns to resolve first: supported GPUI layer-surface visibility/interactivity lifecycle at the pinned revision; actual incremental idle/GPU memory; keyboard-triggered activation-token export for desktop launches; IME and AccessKit semantics in the proposed control; installed Niri/session-lock behavior. No source inspection here proves an end-to-end latency, memory footprint, flawless app-spec compliance, or full extension compatibility.

## Primary sources

References below cite exact source paths where feasible. Pinned source revisions were obtained from first-party repository APIs during research; unversioned docs were read on 2026-10-09. The inspected Fuzzel `main.c`, `match.c`, `render.c` and manuals were fetched from master contemporaneously with the recorded revision; `xdg.c`/`wayland.c` were fetched directly by that revision. Niri/Raycast live docs may change.

- **D1 — local Dojo:** `AGENTS.md`, `GLOSSARY.md`, `docs/agents/domain.md`, `Cargo.toml`, `src/main.rs`, `src/shell.rs`, `src/frame.rs`; local HEAD `01f8a82fbed8907a90a2dd0daff35f1ca666b48e`. Research-note format follows `docs/research/1password-linux-autostart-resume.md`.
- **F1 — Fuzzel manual:** [doc/fuzzel.1.scd](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/doc/fuzzel.1.scd), especially SEARCH PATHS, IME, FILES and timing/focus/dmenu options.
- **F2 — Fuzzel options:** [doc/fuzzel.ini.5.scd](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/doc/fuzzel.ini.5.scd).
- **F3 — Fuzzel lifecycle/population/cache:** [main.c](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/main.c), `read_cache`, `write_cache`, `populate_apps`, `process_event`, initialization/cleanup; [xdg.c](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/xdg.c), parser, `scan_dir`, `xdg_find_programs`.
- **F4 — Fuzzel matching:** [match.c](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/match.c), comparator, worker synchronization, sorting, delayed updates.
- **F5 — Fuzzel rendering/Wayland:** [render.c](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/render.c), [wayland.c](https://codeberg.org/dnkl/fuzzel/src/commit/815d4385afb844f74243a6bc1c7836c4fbebe830/wayland.c).
- **G1 — pinned GPUI window implementation:** [crates/gpui_linux/src/linux/wayland/window.rs](https://github.com/zed-industries/zed/blob/9ab0715969e9854f2efe61a9d782e7698e5fe6d2/crates/gpui_linux/src/linux/wayland/window.rs), `WaylandSurfaceState::new`, visibility reporting, AccessKit adapter/tree methods; [layer_shell.rs](https://github.com/zed-industries/zed/blob/9ab0715969e9854f2efe61a9d782e7698e5fe6d2/crates/gpui_linux/src/linux/wayland/layer_shell.rs).
- **G2 — pinned GPUI client:** [crates/gpui_linux/src/linux/wayland/client.rs](https://github.com/zed-industries/zed/blob/9ab0715969e9854f2efe61a9d782e7698e5fe6d2/crates/gpui_linux/src/linux/wayland/client.rs), text-input-v3 binding/events and `open_uri`/`reveal_path` activation handling.
- **G3 — pinned GPUI platform contracts:** [crates/gpui/src/platform.rs](https://github.com/zed-industries/zed/blob/9ab0715969e9854f2efe61a9d782e7698e5fe6d2/crates/gpui/src/platform.rs), input handler, accessibility callbacks, window kind/visibility interfaces.
- **R1 — Raycast command lifecycle:** [information/lifecycle.md](https://developers.raycast.com/information/lifecycle.md).
- **R2 — Raycast runtime/security/permissions:** [information/security.md](https://developers.raycast.com/information/security.md).
- **R3 — Raycast list/filter/pagination API:** [api-reference/user-interface/list.md](https://developers.raycast.com/api-reference/user-interface/list.md).
- **R4 — Raycast actions:** [api-reference/user-interface/action-panel.md](https://developers.raycast.com/api-reference/user-interface/action-panel.md).
- **R5 — Raycast background lifecycle:** [information/lifecycle/background-refresh.md](https://developers.raycast.com/information/lifecycle/background-refresh.md).
- **V1 — Vicinae native-rendered extension model:** [extensions/introduction](https://docs.vicinae.com/extensions/introduction).
- **V2 — Vicinae pinned runtime source:** [src/server/src/extension/node-runtime/node-runtime.cpp](https://github.com/vicinaehq/vicinae/blob/28ac1efa8865b080bf223b337e1e99df63c2d4b2/src/server/src/extension/node-runtime/node-runtime.cpp); [README.md](https://github.com/vicinaehq/vicinae/blob/28ac1efa8865b080bf223b337e1e99df63c2d4b2/README.md).
- **V3 — Vicinae indexing lifecycle:** [file-search](https://docs.vicinae.com/file-search).
- **L1 — Walker backend requirement:** [README.md](https://github.com/abenz1267/walker/blob/a515085fd39fae80acc086c8fbfd9b7771a9a11d/README.md).
- **L2 — Walker resident invocation:** [src/main.rs](https://github.com/abenz1267/walker/blob/a515085fd39fae80acc086c8fbfd9b7771a9a11d/src/main.rs), startup/service/activate handling.
- **L3 — Walker query contract:** [src/protos/query.proto](https://github.com/abenz1267/walker/blob/a515085fd39fae80acc086c8fbfd9b7771a9a11d/src/protos/query.proto).
- **N1 — Niri fullscreen/layer focus:** [docs/wiki/Layer‐Shell-Components.md](https://github.com/YaLTeR/niri/blob/main/docs/wiki/Layer%E2%80%90Shell-Components.md).
- **N2 — Niri namespace/capture behavior:** [docs/wiki/Configuration:-Layer-Rules.md](https://github.com/YaLTeR/niri/blob/main/docs/wiki/Configuration%3A-Layer-Rules.md).
- **Y1 — layer-shell protocol:** [unstable/wlr-layer-shell-unstable-v1.xml](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-layer-shell-unstable-v1.xml), map/unmap/configure, exclusive zone and keyboard interactivity.
- **Y2 — activation protocol:** [staging/xdg-activation/xdg-activation-v1.xml](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/xdg-activation/xdg-activation-v1.xml), protocol text inspected through its repository mirror.
- **Y3 — presentation protocol:** [stable/presentation-time/presentation-time.xml](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/presentation-time/presentation-time.xml), presented/discarded feedback; protocol source retrieved through repository mirror.
- **X1 — desktop identity/precedence:** [Desktop Entry Specification: file naming](https://specifications.freedesktop.org/desktop-entry/latest/file-naming.html); [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/latest/).
- **X2 — desktop keys:** [recognized-keys.html](https://specifications.freedesktop.org/desktop-entry/latest/recognized-keys.html).
- **X3 — desktop execution grammar:** [exec-variables.html](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html).
- **X4 — desktop D-Bus activation:** [dbus.html](https://specifications.freedesktop.org/desktop-entry/latest/dbus.html).
- **X5 — desktop actions:** [extra-actions.html](https://specifications.freedesktop.org/desktop-entry/latest/extra-actions.html). Desktop specification sections also inspected in the [single-page spec](https://specifications.freedesktop.org/desktop-entry/latest-single/).
- **X6 — GIO desktop backend:** [GioUnix.DesktopAppInfo](https://docs.gtk.org/gio-unix/class.DesktopAppInfo.html), links to implementation/API operations.
- **P1 — cancellation/partial results precedent:** [LSP 3.17 base protocol](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#cancelRequest), cancellation and progress sections.
- **W1 — Wasmtime security boundaries:** [security.html](https://docs.wasmtime.dev/security.html).
- **W2 — Wasmtime CPU interruption:** [examples-interrupting-wasm.html](https://docs.wasmtime.dev/examples-interrupting-wasm.html), fuel/epoch traps and async yielding. Upstream's example-specific slowdown claim is deliberately not used as a launcher forecast.
- **W3 — Wasmtime resource limits:** [ResourceLimiter, version 49.0.2](https://docs.rs/wasmtime/49.0.2/wasmtime/trait.ResourceLimiter.html), explicitly excludes some engine/embedder allocations.
- **W4 — Node permission threat model:** [Node.js Permissions](https://nodejs.org/api/permissions.html), read as v26.11.1; explicitly does not protect against malicious code.

## Scope note

Only this report was added to the repository. No implementation/configuration/desktop changes, benchmarks, commits, or restarts were performed. Existing unrelated research notes were not read or modified.
