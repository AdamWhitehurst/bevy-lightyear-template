---
date: 2026-03-20T22:10:33-07:00
researcher: Claude
git_commit: 3ad3872b105e69d59e7cea1f728e1541e4335023
branch: bevy-lightyear-template-2
repository: bevy-lightyear-template-2
topic: "Bevy UI ecosystem preferences, theming, and component reuse patterns"
tags: [research, ui, theming, bevy_ui, ecosystem, widget-patterns]
status: complete
last_updated: 2026-03-20
last_updated_by: Claude
---

# Research: Bevy UI Ecosystem Preferences, Theming, and Component Reuse

**Date**: 2026-03-20T22:10:33-07:00
**Researcher**: Claude
**Git Commit**: 3ad3872b105e69d59e7cea1f728e1541e4335023
**Branch**: bevy-lightyear-template-2
**Repository**: bevy-lightyear-template-2

## Research Question

What do people prefer to use for Bevy UI and how do they handle consistent theming and component reuse?

## Summary

The Bevy UI ecosystem in early 2026 is fragmented with no single dominant game UI solution. **`bevy_egui`** is the most popular crate by downloads (84k/month) but targets debug/tool UI. For game UI built on native `bevy_ui`, the community mostly hand-rolls with helper functions and color constants. **BSN** (Bevy Scene Notation) was expected to unify UI authoring in 0.18 but the PR was closed without merging. **`bevy_feathers`** provides token-based theming but is experimental and editor-focused. The most complete theming solution available today is **`bevy_flair`** (CSS-based styling with hot-reload). For component reuse, the blessed pattern is `fn widget() -> impl Bundle` with `children![]`, supplemented by `#[require(Node)]` for custom widget components.

This project currently uses raw `bevy_ui` with inline color literals and no theme system.

## Detailed Findings

### Current State of This Project

The project has a dedicated `crates/ui/` workspace crate using pure `bevy_ui` (no third-party UI libraries). Key characteristics:

- **State machine**: `ClientState` (MainMenu, Connecting, InGame) + `MapTransitionState` sub-state
- **Screen lifecycle**: `OnEnter` spawns UI, `DespawnOnExit` cleans up
- **Button pattern**: Marker components (`ConnectButton`, `QuitButton`, etc.) + `Query<&Interaction, (Changed<Interaction>, With<Marker>)>`
- **No theme system**: All colors/sizes are inline literals (e.g., `Color::srgb(0.1, 0.1, 0.1)` for backgrounds, `Color::srgb(0.2, 0.2, 0.2)` for buttons)
- **3D health bars**: Billboard mesh-based system in `crates/render/src/health_bar.rs`, separate from bevy_ui
- **Previous research**: Lua-scripted addon UI explored in `doc/research/2026-03-17-lua-scripted-client-ui.md` (not yet implemented)

Relevant files:
- `crates/ui/src/lib.rs` -- UiPlugin, all screen setup + interaction systems
- `crates/ui/src/state.rs` -- ClientState, MapTransitionState
- `crates/ui/src/components.rs` -- Button marker components
- `crates/render/src/health_bar.rs` -- 3D billboard health bars

### UI Library Landscape

#### bevy_ui (Built-in)

Bevy's native retained-mode UI. ECS entities with `Node` components, laid out via Flexbox/CSS-Grid (Taffy).

**Capabilities (0.18):**
- Flexbox and CSS Grid layout
- Headless logical widgets: Button, Slider, Scrollbar, Checkbox, RadioButton, RadioGroup (0.17+)
- Popover and MenuPopup components (0.18)
- Automatic directional navigation for gamepad/keyboard
- Variable font weights, strikethrough/underline, OpenType features
- Picking-based interaction, scroll containers
- `Val` helpers: `px()`, `percent()`, `vw()`, `vh()`
- UI gradients, `TextShadow`, per-side border colors (0.17+)

**Limitations:**
- No high-level game widget library
- No built-in theming system
- No data-driven UI authoring format (BSN hasn't landed)
- Verbose spawning code without helper abstractions
- No text input widget

Sources: [Bevy 0.18](https://bevy.org/news/bevy-0-18/), [Bevy 0.17](https://bevy.org/news/bevy-0-17/), [bevy::ui docs](https://docs.rs/bevy/latest/bevy/ui/index.html)

#### bevy_egui -- Most Popular by Downloads

Integration of [egui](https://www.egui.rs/) immediate-mode GUI with Bevy. **84k downloads/month**, #3 in Game dev on lib.rs, used by 276 crates. v0.39 supports Bevy 0.18.

Best for: debug UIs, dev tools, inspector panels, prototyping. Some use it for game UI but it has a "tool" aesthetic. Immediate-mode doesn't integrate well with ECS patterns.

Sources: [bevy_egui GitHub](https://github.com/vladbat00/bevy_egui), [lib.rs](https://lib.rs/crates/bevy_egui)

#### bevy_lunex -- Retained Layout Engine

Path-based layout with strong aspect-ratio handling. 896 GitHub stars, ~472 downloads/month, v0.6.0 (Jan 2026, Bevy 0.18). Native 2D and 3D world-space UI. Has a showcase project "Bevypunk."

Separate layout model from bevy_ui (not flexbox/grid). Smaller community.

Sources: [bevy_lunex GitHub](https://github.com/bytestring-net/bevy_lunex), [Documentation](https://bytestring-net.github.io/bevy_lunex/)

#### bevy_hui -- HTML Templates

HTML/XML-based UI templates with hot-reload. 188 GitHub stars, v0.6.0 (Feb 2026, Bevy 0.18). Write pseudo-HTML, keep logic in Bevy systems. Event binding: `on_press="start_game"`. Reusable templates.

Philosophy: "No widgets, no themes. Just bevy_ui serialized with all the tools necessary to build anything in a reusable manner."

Sources: [bevy_hui GitHub](https://github.com/Lommix/bevy_hui), [Blog post](https://lommix.com/article/bevy_hui)

#### bevy_flair -- CSS Styling

Full CSS-like styling for bevy_ui. Supports pseudo-classes, selectors, `var()` custom properties, `@media` queries, transitions, `@keyframes`, gradients, hot-reload. 116 GitHub stars, v0.7 (Bevy 0.17).

```css
:root {
  --primary: rgb(60, 120, 200);
  --bg: rgb(15%, 15%, 15%);
}
button {
  background-color: var(--bg);
  border-radius: 10px;
  transition: background-color 0.5s;
  &:hover { background-color: rgb(30%, 30%, 25%); }
}
```

**Probably the most complete theming solution available today** for bevy_ui.

Sources: [bevy_flair GitHub](https://github.com/eckz/bevy_flair)

#### Quill -- Reactive Framework

Reactive UI inspired by React/Solid, built on Bevy ECS. 194 GitHub stars. Companion `bevy_quill_obsidian` provides themed editor widgets. Requires unstable Rust feature `impl_trait_in_assoc_type`. Unclear Bevy 0.18 support.

Sources: [Quill GitHub](https://github.com/viridia/quill)

#### bevy_feathers (Built-in, Experimental)

Opinionated widget toolkit with **token-based theming** built into Bevy behind `experimental_bevy_feathers` feature flag. Created by viridia (same author as Quill). Targets Bevy Editor, not game UI.

Token naming: `BUTTON_BG`, `BUTTON_BG_HOVER`, `TEXT_MAIN`, `TEXT_DIM`, `FOCUS_RING`, etc. Uses OKLch color palette. `InheritableFont` for font propagation.

The Bevy team says: "if you like what you see, consider copying this code into your own project" rather than depending on it directly.

Sources: [bevy::feathers docs](https://docs.rs/bevy/latest/bevy/feathers/index.html), [PR #19730](https://github.com/bevyengine/bevy/pull/19730)

#### BSN (Bevy Scene Notation) -- NOT YET LANDED

Cart's proposed `bsn!` macro and `.bsn` asset format for declarative UI with scene inheritance and templates. The main PR ([#20158](https://github.com/bevyengine/bevy/pull/20158)) was **closed without merging** despite being expected to land in 0.18. This is a significant gap.

Sources: [BSN Discussion #14437](https://github.com/bevyengine/bevy/discussions/14437), [BSN PR #20158](https://github.com/bevyengine/bevy/pull/20158)

#### Other Notable Crates

| Crate | Description | Status |
|-------|-------------|--------|
| [sickle_ui](https://github.com/danec020/sickle_ui) | Widget library with UiBuilder, data-driven skins | "Do not depend" -- reference only, but influential patterns |
| [bevy_hammer_ui](https://github.com/ethereumdegen/bevy_hammer_ui) | Lightweight fork of sickle_ui's UiBuilder | Small, niche |
| [woodpecker_ui](https://github.com/StarArawn/woodpecker_ui) | Vello-rendered, ECS-first | Experimental |
| [bevy_cobweb_ui](https://github.com/UkoeHB/bevy_cobweb_ui) | Custom `.cob` asset format, hot-reloadable | May lag behind current Bevy |
| [Famiq](https://github.com/MuongKimhong/famiq) | JSON-driven styling, widget library, hot-reload | Active |

### Theming Approaches

**There is no established community standard for game UI theming in Bevy.** The [Bevy Vision document](https://hackmd.io/@bevy/HkjcMkJFC) explicitly says theming is "an active area of controversy."

#### Pattern 1: Const/Resource Theme (Most Common)

```rust
// Constants
const NORMAL_BUTTON: Color = Color::srgb(0.15, 0.15, 0.15);
const HOVERED_BUTTON: Color = Color::srgb(0.25, 0.25, 0.25);
const TEXT_COLOR: Color = Color::srgb(0.9, 0.9, 0.9);

// Or as a Resource
#[derive(Resource)]
struct UiTheme {
    button_bg: Color,
    button_hover: Color,
    text_color: Color,
    heading_font: Handle<Font>,
    body_font: Handle<Font>,
}
```

Simple, no cascading/inheritance. Manual propagation. Used by most Bevy games and the official `game_menu.rs` example.

#### Pattern 2: bevy_feathers Design Tokens

Token-based system with `ThemeProps` maps. `BUTTON_BG`, `TEXT_MAIN`, `FOCUS_RING` etc. OKLch color palette. Experimental, editor-targeted.

#### Pattern 3: bevy_flair CSS

Full CSS `var()` custom properties, `@media` queries, pseudo-classes, transitions. Hot-reloadable. Most complete available solution.

#### Pattern 4: bevy_hui / Famiq External Files

Styles defined in HTML/XML or JSON files. Hot-reloadable. Separation of structure and logic.

### Component Reuse Patterns

#### Pattern 1: `fn widget() -> impl Bundle` (Blessed Pattern)

The primary idiomatic approach since Bevy 0.16+:

```rust
fn button(label: &str) -> impl Bundle {
    (
        Button,
        Node {
            width: px(150), height: px(65),
            border: UiRect::all(px(5)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BorderColor::all(Color::WHITE),
        BackgroundColor(Color::BLACK),
        children![(
            Text::new(label),
            TextColor(Color::srgb(0.9, 0.9, 0.9)),
        )],
    )
}

// Usage
commands.spawn(button("Play"));
```

Source: [Tainted Coders Bevy UI](https://taintedcoders.com/bevy/ui), [official game_menu.rs](https://github.com/bevyengine/bevy/blob/main/examples/games/game_menu.rs)

#### Pattern 2: Required Components for Custom Widgets

Since Bevy 0.15, `#[require]` auto-inserts dependencies:

```rust
#[derive(Component)]
#[require(Node, UiImage)]
struct MyButton;
```

Spawning `MyButton` automatically inserts `Node` and `UiImage` with defaults.

Source: [Required Components PR #14791](https://github.com/bevyengine/bevy/pull/14791)

#### Pattern 3: Widget Observer Pattern (0.17+)

```rust
commands.spawn((
    Button,
    observe(|_: On<Activate>| { info!("clicked!"); }),
));
```

#### Pattern 4: Extension Trait (sickle_ui style)

```rust
pub trait UiWidgetExt {
    fn my_button(&mut self, config: ButtonConfig) -> &mut Self;
}
```

Keeps widget spawning encapsulated and composable. Detailed walkthrough: [How do Nice UI in Bevy (deadmoney.gg)](https://deadmoney.gg/news/articles/how-do-nice-ui-in-bevy)

### Community Recommendations Summary

| Use Case | Recommended Approach |
|----------|---------------------|
| Debug/dev tools | `bevy_egui` (clear winner, 84k downloads/month) |
| CSS-like theming with hot-reload | `bevy_flair` |
| HTML-like templates with hot-reload | `bevy_hui` |
| Aspect-ratio-first / world-space UI | `bevy_lunex` |
| Reactive React-like patterns | `quill` (unstable Rust required) |
| Minimal ergonomic helpers on raw bevy_ui | `bevy_hammer_ui` or roll your own |
| "Wait for official" | `bevy_feathers` + headless widgets + BSN (when it lands) |

Community consensus: UI is Bevy's weakest area. The ecosystem is the "wild west." Things will improve when BSN lands, but that timeline is uncertain.

## Code References

- `crates/ui/src/lib.rs` -- Current UI plugin, all screen setup, inline colors
- `crates/ui/src/state.rs` -- ClientState, MapTransitionState
- `crates/ui/src/components.rs` -- Button marker components
- `crates/render/src/health_bar.rs` -- 3D billboard health bars

## Related Research

- `doc/research/2026-03-17-lua-scripted-client-ui.md` -- Lua-scripted addon UI system exploration
- `doc/research/2025-11-28-ui-crate-and-client-state.md` -- Original UI crate implementation plan
- `doc/research/2026-02-16-health-respawn-billboard-ui.md` -- Health/respawn billboard UI

## External Sources

- [Bevy 0.18 Release Notes](https://bevy.org/news/bevy-0-18/)
- [Bevy 0.17 Release Notes](https://bevy.org/news/bevy-0-17/)
- [A Vision for Bevy UI (HackMD)](https://hackmd.io/@bevy/HkjcMkJFC)
- [BSN Discussion #14437](https://github.com/bevyengine/bevy/discussions/14437)
- [BSN PR #20158 (CLOSED)](https://github.com/bevyengine/bevy/pull/20158)
- [bevy_feathers PR #19730](https://github.com/bevyengine/bevy/pull/19730)
- [bevy_feathers docs](https://docs.rs/bevy/latest/bevy/feathers/index.html)
- [bevy_egui GitHub](https://github.com/vladbat00/bevy_egui)
- [bevy_lunex GitHub](https://github.com/bytestring-net/bevy_lunex)
- [bevy_hui GitHub](https://github.com/Lommix/bevy_hui)
- [bevy_flair GitHub](https://github.com/eckz/bevy_flair)
- [sickle_ui GitHub](https://github.com/danec020/sickle_ui)
- [bevy_hammer_ui GitHub](https://github.com/ethereumdegen/bevy_hammer_ui)
- [Quill GitHub](https://github.com/viridia/quill)
- [woodpecker_ui GitHub](https://github.com/StarArawn/woodpecker_ui)
- [bevy_cobweb_ui GitHub](https://github.com/UkoeHB/bevy_cobweb_ui)
- [Famiq GitHub](https://github.com/MuongKimhong/famiq)
- [How do Nice UI in Bevy (deadmoney.gg)](https://deadmoney.gg/news/articles/how-do-nice-ui-in-bevy)
- [Tainted Coders Bevy UI](https://taintedcoders.com/bevy/ui)
- [Bevy Discussion: Widgets and Styling #9652](https://github.com/bevyengine/bevy/discussions/9652)
- [Bevy Discussion: 10 Challenges for UI Frameworks #11100](https://github.com/bevyengine/bevy/discussions/11100)
- [Required Components PR #14791](https://github.com/bevyengine/bevy/pull/14791)
- [Bevy Standard Widgets Example](https://bevy.org/examples/ui-user-interface/standard-widgets/)

## Open Questions

1. **BSN timeline**: When will BSN actually land? The closed PR suggests it may be 0.19+ or later. This affects whether to invest in patterns that BSN will replace.
2. **bevy_flair 0.18 support**: Currently at v0.7 targeting 0.17. Is an 0.18-compatible version available or imminent?
3. **bevy_feathers for games**: Could the token-based theming approach be adapted for game UI, or is it too coupled to editor aesthetics?
4. **Lua UI integration**: How do these findings interact with the Lua-scripted addon UI system explored in `doc/research/2026-03-17-lua-scripted-client-ui.md`?
