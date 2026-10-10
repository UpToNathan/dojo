# Dojo theming system

Status: design draft

## Intent

Dojo has two theme modes: static presets and algorithmically generated wallpaper-derived themes. Both remain first-class. A shared design system defines what each color means and how components use it. Each mode supplies that contract through its own policy.

Static presets preserve deliberate authored colors. Wallpaper themes derive a coherent palette using a reviewed algorithm. Improving wallpaper generation does not turn it into a preset with an accent override, and does not add a third theme mode.

The [interactive study](../../docs/design/theme-study.html) explores explicit interaction states and a controlled generated palette. Its colors are illustrative. The surface tint toggle uses RGB mixing for the study, not the proposed perceptual algorithm. It does not apply anything to the desktop.

## Current implementation and gaps

Extraction produces 16 ANSI slots. `SemanticPalette::derive` in `src/theme.rs` uses slot 4 for primary, slot 5 for secondary, and other ANSI slots for status roles. It creates surfaces with RGB mixing and corrects colors toward black or white. Appearance labels do not independently control this semantic recipe.

Eight Catppuccin and Tokyo Night variants have explicit UI/syntax maps in `src/theme/authored-presets.txt`. The other 28 presets use terminal-palette adaptation. Those entries remain available, with their provenance made visible until each receives authored mappings.

`src/theme/export.rs` publishes only six roles to `dojo.json`. `src/config.rs` consumes that reduced palette, so the shell loses authored button and selection foregrounds, surfaces, and status roles. The picker chooses black or white button labels locally.

Several exporters collapse distinct meanings. T3 hover, active, and selection map to one role; its warning surface uses `diff_added_background`. Syntax falls back to status colors when explicit tokens are absent. These are concrete migration targets, not evidence that all exporters need replacement.

Generation storage, captured wallpaper bytes, ownership checks, rollback, per-target reports, and system appearance coordination already exist. Preserve their behavior.

## Typography

Victor Mono Nerd Font is a foundational part of Dojo's current design language. Use the bundled `VictorMono Nerd Font Mono` family for shell text and icons, including the desktop frame, launcher, menus, and theme picker. Enable OpenType `ss01` for every Victor Mono use, including text measurement and previews. Both theme modes preserve this typography.

The current bundled font is `assets/fonts/VictorMonoNerdFontMono-Regular.ttf`. Its runtime family and feature policy live with the existing font registration in `src/power.rs`. Components use the shared feature policy rather than declaring it independently. Static colors and wallpaper-derived colors do not select a different font.

Terminals currently configure the unpatched `Victor Mono` family with `ss01` enabled. Keep that existing configuration while deciding whether external applications also need the Nerd Font variant. Type sizes, weight, line height, and spacing still need live evaluation; font family and `ss01` are settled.

The interactive study embeds a subset of the bundled font with `ss01` retained. Its typography must reflect this rule rather than the hosting app's default sans-serif font.

## Shared visual rules

The desktop frame surrounds the workspace without competing with its contents. Panels, overlays, and controls express their function through consistent color and geometry.

- Define canvas, panel, and overlay roles. Generated themes use a controlled tonal relationship. Static presets retain their authored surface hierarchy, including darker panels.
- Distinguish idle, hover, selection, and keyboard focus. Selection has a background/foreground pair; focus remains visible while a row is selected.
- Keyboard focus uses a lighter wash of the hover color, initially 45% of its fill strength, plus a subtle outline and a visible focus marker. A focused selected row retains its selection fill. Evaluate the wash in both appearances rather than treating focus as a darker hover state.
- Distinguish accent text, accent fill, and text on accent fill. One accent hex need not satisfy every usage.
- Give error, warning, success, and information their own text/background pairs. Accent changes never silently swap their meaning.
- Separate readable muted text from disabled content. Use text or symbols as well as color to communicate status.
- Keep syntax and terminal palettes independent from UI feedback. A string can be green without depending on the UI success role.
- Render decorative borders and separators at 20% opacity. This is alpha applied to the border color, not a 20% lighter hex or reduced opacity for the whole component. Use surface color and spacing for hierarchy rather than dense outlines. Native constants live in `src/design.rs`.
- Use pill-shaped rounding for action buttons, mode controls, compact toggles, and short selection controls. Panels and content containers have generous rounded corners; their shapes still suit multiline content.
- Keep the overall aesthetic fluid and calming: quiet surfaces, generous spacing, restrained accents, and soft changes between interaction states. Avoid sharp boxes, competing outlines, and abrupt ornamental motion. Motion respects reduced-motion preferences.
- Keep spacing, typography, radii, and motion consistent across both modes. Start with a shared spacing scale of 4, 8, 12, 16, and 24 pixels. Pill radii follow control height; panel corner radii remain provisional until live evaluation. These are Dojo component rules, not wallpaper parameters or requirements imposed on external apps. Choose type sizes and motion from live shell specimens before centralizing them.

Generated output checks readable text against every permitted surface, and foregrounds against their own fills. The initial normal/muted text target is 4.5:1. Essential focus indicators have a separate 3:1 target against adjacent surfaces. These are design targets, not a claim that existing themes or the HTML study satisfy every accessibility requirement.

Authored preset validation reports weak pairs without recoloring them. Preserve the existing authored acceptance floor during migration. Do not call adapted presets faithful editor ports.

## Light appearance

Light mode needs its own surface hierarchy. The initial pale-surface studies passed text contrast checks but made interaction states too similar, and thin small text remained uncomfortable to read. Softness cannot depend on washing out every role.

Use a lower-glare canvas with visibly brighter panels and overlays. Darken normal and muted text, and evaluate Victor Mono at a larger size and medium weight. Hover, selection, and keyboard focus need visibly different fills. The revised light study tries 15-pixel body text, medium weight, and stronger surface steps. These sizes and colors remain proposals until live evaluation.

Keep accents restrained, status text dark, and status fills distinct. Decorative boundaries still use 20% opacity; the font family, `ss01`, and pill controls remain shared with dark mode. Text contrast checks alone do not establish visual clarity. `scripts/audit-theme-study.py` reports the actual specimen colors, luminance, and adjacent-state contrast so each revision can be compared.

Static previews read the real authored preset roles. Do not invent a paper palette to demonstrate static light mode. Generated light appearance can change its recipe; static light appearance preserves each preset's authored colors and hierarchy. Typography, geometry, border opacity, and interaction treatment are the shared design rules.

## User experience

Keep the two theme modes visible in the picker: Presets and Derive. Wallpaper browsing and wallpaper-only Apply remain independent operations, not a third theme mode.

Open the picker on Wallpaper. Order the tabs Wallpaper, Presets, then Derive. Populate the wallpaper directory immediately, and compute derivation previews only when Derive is opened. Keep derivation an optional attempt; the number of modes is not a headline benefit. A valid wallpaper can be previewed and applied even when it cannot produce a useful palette.

Both mode previews show actual role usage: panel, overlay, hover, selected and focused rows, an accent button, status notices, a code sample, and terminal colors. The preview uses the same resolved palette as export and Apply.

Static presets identify family, variant, appearance, and authored/adapted provenance. They never fabricate a missing opposite variant. Generated themes show paired dark/light output. Existing 23 extraction modes remain available while a replacement recipe is evaluated; naming or removal waits for a concrete comparison.

The wallpaper algorithm chooses candidates from captured image colors rather than assuming ANSI blue is the intended UI accent. A user may choose among those candidates or keep the algorithm's default. Any adjustment is part of Derive. Surface tint is a bounded option; it cannot alter status meaning or the chosen syntax profile. Exact controls remain provisional.

Preview changes are read-only. Apply publishes through the existing store and reports each target's result. Publication, requested reload, and verified visual change remain distinct outcomes.

## Architecture sketch

Callers consume a resolved palette. They never inspect wallpaper pixels or ANSI slots to choose UI colors.

```rust
// Proposed API shapes, not implemented declarations.
let palette = theme.palette(appearance);
let selection = palette.ui.selection;
row.background(selection.background).foreground(selection.foreground);
let button = palette.ui.accent.fill;
button_view.background(button.background).foreground(button.foreground);
let token = palette.syntax.keyword;
```

```rust
struct ColorPair { background: Rgb, foreground: Rgb }
struct UiRoles {
    canvas: Rgb,
    panel: Rgb,
    overlay: Rgb,
    hover: ColorPair,
    selection: ColorPair,
    focus: Rgb,
    text: Rgb,
    text_muted: Rgb,
    text_disabled: Rgb,
    border: Rgb,
    accent: AccentRoles,
    feedback: FeedbackRoles,
    diff_added: ColorPair,
    diff_removed: ColorPair,
}
struct AccentRoles { text: Rgb, fill: ColorPair, subtle: ColorPair }
struct FeedbackRoles {
    error: ColorPair, warning: ColorPair, success: ColorPair, info: ColorPair,
}
struct ResolvedPalette {
    appearance: Appearance,
    ui: UiRoles,
    syntax: SyntaxRoles,
    terminal: [Rgb; 16],
    editor: EditorRoles,
}
```

`SyntaxRoles` and `EditorRoles` must cover existing exporter reads before this sketch becomes an implementation contract. Inventory optional keys such as line numbers and float borders; avoid deleting authored specificity to fit a smaller type.

The theme domain owns resolution and complete roles. Preset parsing and wallpaper generation are separate producers. Exporters own explicit destination mappings; GPUI conversion lives at the shell boundary. Parse old generations at the storage boundary and preserve their stored colors without regeneration. Keep one internal role API and migrate its callers together.

A new perceptual wallpaper recipe needs standard color math private to theme generation. `src/extraction/color.rs` deliberately uses Aether-specific coefficients, and its fixtures pin upstream behavior. Changing those transforms to improve Dojo's UI recipe would break a different contract.

Each generated theme records both extraction and semantic-recipe revisions. An algorithm update creates new generations; loading or rolling back an old generation keeps its exact colors. Manual fallback config remains user-owned and needs an explicit fallback resolution policy instead of synthetic ANSI slots.

## Alternatives and synthesis

Independent candidates compared complete authored palettes with a controlled generated recipe. The shared contract and preservation rules from the authored design provide the base. Typed color pairs, controlled generated surfaces, and component previews come from the generator design.

The initial review favored an authored-first product direction. The user's clarification supersedes that priority: both modes remain peers. Reject an accent-only extension as a replacement for wallpaper-derived theming, and reject a new Compose tab.

Also reject refining contrast formulas alone. That leaves ambiguous role meaning and local exporter decisions intact. A complete token editor is outside this work; it asks users to maintain every relationship themselves.

## Delivery

1. Restore T3 nightly selection through its supported bundled CLI and verify the installed app recognizes the published theme.
2. Carry existing role pairs into the shell and remove local button-text guesses without changing preset colors.
3. Inventory and migrate the shared role contract, including semantic exporter mappings and old-generation reads.
4. Compare the existing wallpaper algorithm with a controlled recipe using the same wallpapers and component specimens.
5. Complete static preset fidelity in family-sized batches while retaining explicit adapted labels.
6. Centralize shell component geometry and verify the native picker, launcher, tray menus, and frame in Niri.

## Open design decisions

The exact generated surface tint, accent ranking, syntax policy, and light-mode treatment need comparisons on several wallpapers, including grayscale and low-chroma images. The study makes those decisions visible but does not settle them. Type sizes and motion need live shell evaluation. The two-mode structure, Victor Mono Nerd Font, and universal `ss01` usage are settled.
