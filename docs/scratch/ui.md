The UI should not “own” gameplay. Hotkeys, buttons, hotbar clicks, radial menus, and controller bindings should all resolve into the same semantic commands, and gameplay systems should process those commands. Immediate-mode UI is then just one way to display state and emit commands.

A typical architecture looks like this:

Physical input -> input actions / keybind layer -> UI focus + routing layer -> commands/events -> ECS gameplay systems -> ECS/world/UI state -> immediate-mode rendering reads state every frame

The important distinction is between “input,” “UI,” and “gameplay intent.” Pressing 1, clicking hotbar slot 1, or using a controller face button should all produce something like:

ActivateHotbarSlot { player, slot: 0 }

not three separate code paths.

For an ECS game, I would usually model it like this:

```rust
struct PlayerInputBindings {
    hotbar_slot_1: ActionId,
    hotbar_slot_2: ActionId,
    inventory: ActionId,
    map: ActionId,
}

struct Hotbar {
    slots: [Option<AbilityOrItemId>; 10],
}

struct ActivateHotbarSlotRequest {
    player: Entity,
    slot: usize,
}

struct TogglePanelRequest {
    player: Entity,
    panel: PanelId,
}

struct UiState {
    open_panels: HashSet<PanelId>,
    focused_panel: Option<PanelId>,
    input_capture: InputCaptureMode,
}
```

Then the flow is:

```
InputSystem:
  "1" pressed -> ActivateHotbarSlotRequest(player, 0)

HotbarUi:
  button clicked on slot 0 -> ActivateHotbarSlotRequest(player, 0)

AbilitySystem:
  reads ActivateHotbarSlotRequest
  looks up Hotbar[player].slots[0]
  validates cooldown / mana / range / silence / targeting
  starts ability or rejects it

```

That is the main “standard” idea: UI and hotkeys both produce intent; gameplay systems decide whether intent is valid.

Immediate-mode UI should generally be treated as a projection of state, not the state itself. Dear ImGui’s own framing is that it outputs renderable vertex buffers and is renderer-agnostic, which fits this style well: your ECS/game state exists elsewhere, and the UI redraws from it each frame. Unity’s own IMGUI documentation also positions immediate-mode GUI primarily as a programmer-oriented, code-driven GUI system, which matches how many teams use IMGUI for tools/debug UI rather than as the full shipping UI architecture.

For hotkeys, use semantic actions, not raw keys sprinkled through systems. This is the pattern used by modern engine input systems. Unity’s Input System binds device controls to named Actions, and those bindings are referenced from code instead of hard-coding the device input. Unreal’s Enhanced Input similarly supports adding and removing mapping contexts at runtime, which is exactly the kind of mechanism you want for gameplay mode versus menu mode versus vehicle mode versus targeting mode.

So for WoW-style hotbars, the cleaner model is:

```
Key "1"       ┐
Mouse click   ├──> ActivateHotbarSlot(0)
Gamepad bind  ┘

HotbarSlot(0) -> AbilityId.Fireball
AbilitySystem -> TryActivateAbility(Fireball)
```

An ECS game would usually split this into several small systems, not one “UI system.” The core idea is:

```text
Input systems create intent.
UI systems create intent.
Gameplay systems consume intent.
Rendering systems display the resulting state.
```

A clean setup might look like this.

### 1. Raw input collection system

This reads keyboard, mouse, controller, etc. It should not know what “cast Fireball” means.

```rust
struct RawInputState {
    keys_down: HashSet<KeyCode>,
    keys_pressed: HashSet<KeyCode>,
    mouse_buttons_pressed: HashSet<MouseButton>,
    mouse_position: Vec2,
    scroll_delta: f32,
}
```

This is low-level platform input.

It answers: “Was key 1 pressed?” It does not answer: “Should hotbar slot 1 activate?”

---

### 2. Input action mapping system

This converts raw input into semantic actions.

```rust
enum InputAction {
    HotbarSlot(usize),
    ToggleInventory,
    ToggleMap,
    OpenCharacterPanel,
    Cancel,
    Confirm,
    Interact,
    MoveLeft,
    MoveRight,
}
```

Example output:

```rust
struct InputActionEvent {
    player: Entity,
    action: InputAction,
}
```

So pressing `1` might produce:

```rust
InputActionEvent {
    player,
    action: InputAction::HotbarSlot(0),
}
```

This is also where rebinding lives. The rest of the game should not care whether hotbar slot 1 is bound to `1`, `Mouse4`, or a controller bumper combo.

---

### 3. UI focus / input routing system

This decides whether an input action goes to gameplay, a panel, a modal, a text field, or the console.

```rust
enum InputCaptureMode {
    Gameplay,
    Ui,
    TextInput,
    Modal,
    Console,
}

struct UiFocusState {
    focused_widget: Option<WidgetId>,
    focused_panel: Option<PanelId>,
    capture_mode: InputCaptureMode,
}
```

Examples:

```text
Escape while gameplay is focused -> OpenPauseMenu
Escape while inventory is focused -> CloseInventory
Escape while modal is focused -> CancelModal
1 while chat box is focused -> type "1"
1 while gameplay is focused -> ActivateHotbarSlot(0)
```

This system is important. Without it, hotkeys and UI panels become messy quickly.

---

### 4. UI panel state system

This manages which panels are open, closed, focused, pinned, modal, etc.

```rust
enum PanelId {
    Inventory,
    Character,
    Skills,
    Map,
    QuestLog,
    Settings,
    DebugInspector,
}

struct UiPanelState {
    open_panels: HashSet<PanelId>,
    focused_panel: Option<PanelId>,
    modal_stack: Vec<PanelId>,
}
```

It consumes requests like:

```rust
struct TogglePanelRequest {
    panel: PanelId,
}

struct OpenPanelRequest {
    panel: PanelId,
}

struct ClosePanelRequest {
    panel: PanelId,
}
```

Both hotkeys and UI buttons should emit these same requests.

For example:

```text
Press I              -> TogglePanelRequest(Inventory)
Click bag icon       -> TogglePanelRequest(Inventory)
Quest reward opens inventory -> OpenPanelRequest(Inventory)
```

All three go through the same panel system.

---

### 5. Immediate-mode UI draw systems

These render the actual UI every frame. They read ECS/world/UI state and emit requests when clicked.

Example systems:

```text
DrawHudSystem
DrawHotbarSystem
DrawInventorySystem
DrawCharacterPanelSystem
DrawTooltipSystem
DrawContextMenuSystem
DrawDebugInspectorSystem
```

A hotbar draw system might do this:

```rust
fn draw_hotbar(
    ui: &mut Ui,
    hotbar: &Hotbar,
    cooldowns: &Cooldowns,
    commands: &mut EventWriter<GameCommand>,
) {
    for slot in 0..hotbar.slots.len() {
        let slot_data = hotbar.slots[slot];

        draw_hotbar_slot(ui, slot_data, cooldowns);

        if ui.slot_clicked(slot) {
            commands.send(GameCommand::ActivateHotbarSlot {
                slot,
            });
        }
    }
}
```

The important part: `DrawHotbarSystem` does not cast the spell. It only emits an activation request.

---

### 6. Hotbar model system

This owns the data behind the hotbar.

```rust
struct Hotbar {
    slots: Vec<HotbarSlot>,
}

enum HotbarSlot {
    Empty,
    Ability(AbilityId),
    Item(ItemId),
    Macro(MacroId),
    Emote(EmoteId),
    Command(GameCommandId),
}
```

This system handles editing hotbars:

```rust
struct AssignHotbarSlotRequest {
    slot: usize,
    binding: HotbarSlot,
}

struct MoveHotbarSlotRequest {
    from: usize,
    to: usize,
}

struct ClearHotbarSlotRequest {
    slot: usize,
}
```

Drag-and-drop should modify this model. It should not be special-cased inside the rendering code.

---

### 7. Hotbar activation system

This converts “activate slot 3” into “try to activate the thing in that slot.”

```rust
struct ActivateHotbarSlotRequest {
    player: Entity,
    slot: usize,
}
```

The system does roughly:

```rust
fn activate_hotbar_slot_system(
    requests: EventReader<ActivateHotbarSlotRequest>,
    hotbars: Query<&Hotbar>,
    mut commands: EventWriter<ActivateAbilityRequest>,
) {
    for request in requests.read() {
        let hotbar = hotbars.get(request.player).unwrap();

        match hotbar.slots[request.slot] {
            HotbarSlot::Ability(ability_id) => {
                commands.send(ActivateAbilityRequest {
                    caster: request.player,
                    ability: ability_id,
                    source: ActivationSource::Hotbar,
                });
            }

            HotbarSlot::Item(item_id) => {
                commands.send(UseItemRequest {
                    user: request.player,
                    item: item_id,
                });
            }

            HotbarSlot::Macro(macro_id) => {
                commands.send(RunMacroRequest {
                    user: request.player,
                    macro_id,
                });
            }

            HotbarSlot::Empty => {}
        }
    }
}
```

This is a useful separation because hotbar slots are not necessarily abilities. They might point to items, macros, stances, pings, mounts, buildings, or commands.

---

### 8. Ability activation system

This is gameplay, not UI.

```rust
struct ActivateAbilityRequest {
    caster: Entity,
    ability: AbilityId,
    source: ActivationSource,
}
```

This system validates:

```text
Does the entity know this ability?
Is the ability on cooldown?
Does the entity have enough mana/stamina/rage?
Is the entity silenced/stunned/rooted?
Is there a valid target?
Is the target in range?
Is line of sight required?
Is the global cooldown active?
Is the ability usable in the current stance/form/state?
```

If valid, it emits or applies:

```rust
struct ActiveAbility {
    ability: AbilityId,
    started_at: GameTime,
    phase: AbilityPhase,
}

struct Cooldown {
    ability: AbilityId,
    remaining: f32,
}

struct SpendResourceRequest {
    entity: Entity,
    resource: ResourceType,
    amount: f32,
}
```

This is the system that actually starts the ability.

---

### 9. Targeting system

Many abilities need target selection. That should also be separate from UI.

```rust
enum TargetingMode {
    None,
    SelectUnit {
        ability: AbilityId,
    },
    SelectGroundPoint {
        ability: AbilityId,
        radius: f32,
    },
    SelectDirection {
        ability: AbilityId,
    },
}
```

Components/resources:

```rust
struct TargetingState {
    mode: TargetingMode,
    preview_position: Option<Vec3>,
    hovered_entity: Option<Entity>,
}
```

Flow:

```text
Player presses hotbar slot 4
Ability requires ground target
Ability system enters TargetingMode::SelectGroundPoint
UI/cursor system shows targeting circle
Player clicks ground
ConfirmTargetRequest emitted
Ability system activates ability at that point
```

This prevents each ability button from needing its own targeting logic.

---

### 10. Tooltip / hover system

Immediate-mode UI usually gives you hover state directly, but it is still useful to centralize tooltip data.

```rust
struct TooltipRequest {
    anchor: UiRect,
    subject: TooltipSubject,
}

enum TooltipSubject {
    Ability(AbilityId),
    Item(ItemId),
    Entity(Entity),
    StatusEffect(StatusEffectId),
    Text(String),
}
```

The hotbar UI can emit:

```rust
TooltipRequest {
    subject: TooltipSubject::Ability(Fireball),
}
```

Then a tooltip system looks up the ability name, cost, cooldown, range, scaling, etc.

---

### 11. Drag-and-drop system

For WoW-style hotbars, inventory, ability books, and equipment screens, drag-and-drop should be its own UI interaction state.

```rust
struct DragState {
    source: DragSource,
    payload: DragPayload,
}

enum DragSource {
    HotbarSlot(usize),
    InventorySlot(usize),
    AbilityBook(AbilityId),
}

enum DragPayload {
    Ability(AbilityId),
    Item(ItemId),
    HotbarSlot(usize),
}
```

Drop requests:

```rust
struct DropPayloadRequest {
    payload: DragPayload,
    target: DropTarget,
}

enum DropTarget {
    HotbarSlot(usize),
    InventorySlot(usize),
    EquipmentSlot(EquipmentSlot),
    WorldPosition(Vec3),
}
```

Then systems decide what the drop means.

Example:

```text
AbilityBook Fireball dragged to HotbarSlot(1)
-> AssignHotbarSlotRequest { slot: 1, binding: Ability(Fireball) }

Inventory potion dragged to HotbarSlot(2)
-> AssignHotbarSlotRequest { slot: 2, binding: Item(Potion) }

Item dragged from inventory to world
-> DropItemRequest
```

Do not bury this logic inside the button code.

---

### 12. Cooldown / resource display system

This is presentation-side state derived from gameplay.

```rust
struct CooldownDisplay {
    ability: AbilityId,
    remaining: f32,
    duration: f32,
}

struct ResourceDisplay {
    entity: Entity,
    resource: ResourceType,
    current: f32,
    max: f32,
}
```

The hotbar UI reads cooldowns and resources to show:

```text
greyed out icon
cooldown spiral
numeric countdown
not enough mana tint
out of range indicator
```

But it should not decide whether the spell can actually be used. That decision belongs to the ability system.

---

### 13. Command queue / event cleanup system

Many of these requests are frame-local:

```rust
ActivateHotbarSlotRequest
TogglePanelRequest
OpenPanelRequest
UseItemRequest
DropPayloadRequest
ConfirmTargetRequest
CancelTargetingRequest
```

Depending on your ECS framework, these might be events, transient components, or command-buffer entries. Either way, you need a convention for lifetime.

Common rule:

```text
Requests/events live for one frame or one fixed tick.
Persistent state lives in components/resources.
```

---

### 14. UI synchronization system

This is optional, but useful if your ECS game has a retained-ish model behind the immediate-mode UI.

Examples:

```text
Selected entity changed -> inspector panel should update
Inventory changed -> inventory panel should refresh sort/filter state
Player learned ability -> ability book becomes dirty
Quest accepted -> quest tracker opens or refreshes
```

This system keeps UI model state aligned with gameplay state.

---

### 15. Save/load settings system

Hotbars, keybindings, panel layout, and UI preferences usually need persistence.

Saveable data:

```rust
struct SavedInputBindings {
    bindings: HashMap<InputAction, Vec<InputBinding>>,
}

struct SavedHotbarLayout {
    slots: Vec<HotbarSlot>,
}

struct SavedUiLayout {
    panel_positions: HashMap<PanelId, Vec2>,
    panel_sizes: HashMap<PanelId, Vec2>,
    locked_hotbars: bool,
    scale: f32,
}
```

This should not be mixed into the draw code.

---

A reasonable system schedule would be:

```text
Begin frame
  RawInputSystem
  InputActionMappingSystem
  UiFocusRoutingSystem

UI/input intent phase
  ImmediateModeHudSystem
  ImmediateModePanelSystems
  DragDropSystem
  HotkeyCommandSystem

Command interpretation phase
  UiPanelSystem
  HotbarActivationSystem
  TargetingSystem
  InventoryCommandSystem
  ItemUseSystem
  AbilityActivationSystem

Gameplay phase
  AbilityExecutionSystem
  CooldownSystem
  ResourceSystem
  StatusEffectSystem
  CombatSystem

Presentation phase
  TooltipSystem
  CooldownDisplaySystem
  FloatingTextSystem
  UiAnimationSystem
  RenderUiSystem

End frame
  ClearTransientInputSystem
  ClearOneFrameRequestsSystem
```

For your specific case — immediate-mode, ECS, in-game UI, hotkeys, abilities, toggled panels, and WoW-style hotbars — I would start with these core systems:

```text
RawInputSystem
InputActionMappingSystem
UiFocusRoutingSystem
UiPanelStateSystem
ImmediateModeHudSystem
ImmediateModePanelDrawSystem
HotbarEditSystem
HotbarActivationSystem
AbilityActivationSystem
TargetingSystem
TooltipSystem
DragDropSystem
ClearUiRequestsSystem
```

That is enough to scale cleanly.

The key split is this:

```text
Hotkey pressed or UI button clicked
  -> request

Request interpreted
  -> gameplay command

Gameplay command validated
  -> state change

State change rendered
  -> UI updates next frame
```

That separation is what keeps ECS UI from turning into a pile of direct calls from buttons into gameplay logic.
