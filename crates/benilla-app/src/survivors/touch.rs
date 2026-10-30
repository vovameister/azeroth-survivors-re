//! Touch play, for phones and tablets, as a twin-stick: a floating stick on the left of the screen
//! moves the hero, one on the right turns him (and aims the mobility spell), and a tap works the
//! UI as a left click. The HUD's mobility slot becomes the on-screen key for
//! [`super::mobility::KEY`], icon and cooldown sweep and all, and a pause button sits right of the
//! minimap. Nothing here changes until the first touch, so a desktop with a mouse never sees it.
//!
//! The mode's UI roots ([`SafeRoot`]) keep inside the device's safe area ([`SafeArea`]): an
//! invisible border of the inset on every side, which shifts their absolutely placed children too.

use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform};
use bevy::window::PrimaryWindow;

use super::hud::{MobilityButton, MobilityKeyLabel, MobilitySlot, MOBILITY_SIZE};
use super::minimap::MapCluster;
use super::Phase;

/// A stick's reach: a finger this far from where it landed is full push, in logical pixels.
const STICK_RADIUS: f32 = 70.0;
/// Below this a stick rests, so a thumb laid down without moving does not creep the hero.
const STICK_DEAD: f32 = 8.0;
/// The share of the screen's width, from the left, where a touch in a run takes the move stick;
/// a touch right of it, off any button, takes the aim stick.
const STICK_ZONE: f32 = 0.45;
/// The mobility slot's size on a touch screen, as a multiple of the HUD's, and its gap to the
/// safe area's bottom edge (clear of the experience bar), in UI units (the 768-tall canvas, as the
/// HUD). It stays centred, between the thumbs, where neither stick's finger covers it.
const MOBILITY_SCALE: f32 = 2.4;
const MOBILITY_GAP: f32 = 30.0;
/// The pause button, right of the minimap cluster, which steps left to make room. It centres in
/// the strip between the cluster and the screen's right edge, inset and all: at its height, above
/// the middle, nothing of the device sits in the inset (a landscape notch or island is centred).
/// Its top centres it on the minimap's disc (the cluster's `MapBody`, 140 tall from 22).
const PAUSE_SIZE: f32 = 76.0;
const PAUSE_GAP: f32 = 12.0;
const PAUSE_TOP: f32 = 92.0 - PAUSE_SIZE / 2.0;
/// How far the minimap cluster steps left of the safe area's right edge.
const MAP_STEP: f32 = PAUSE_SIZE + PAUSE_GAP * 2.0;
/// The pause button's fill at rest and while pressed.
const PAUSE_FILL: Color = Color::srgba(0.05, 0.07, 0.14, 0.78);
const PAUSE_FILL_PRESSED: Color = Color::srgba(0.42, 0.28, 0.04, 0.92);
const GOLD: Color = Color::srgb(1.0, 0.82, 0.18);
const GOLD_EDGE: Color = Color::srgb(0.45, 0.27, 0.02);
/// The camera's distance on a touch screen: closer than the desktop's 30, as a phone's screen is
/// small and there is no wheel to zoom with.
const CAMERA_DIST: f32 = 22.0;

/// The device's safe area as insets from each window edge, in logical pixels; zero on a desktop.
#[derive(Resource, Default, Clone, Copy, PartialEq, Debug)]
pub(super) struct SafeArea {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) right: f32,
    pub(super) bottom: f32,
}

/// A full-screen UI root that keeps its content inside [`SafeArea`].
#[derive(Component)]
pub(super) struct SafeRoot;

/// A floating stick: the finger holding it, where it landed, and the push, `y` down the screen,
/// at most unit length.
#[derive(Default, Clone, Copy)]
pub(super) struct Stick {
    finger: Option<u64>,
    origin: Vec2,
    pub(super) push: Vec2,
}

impl Stick {
    fn take(&mut self, id: u64, at: Vec2) {
        *self = Stick {
            finger: Some(id),
            origin: at,
            push: Vec2::ZERO,
        };
    }

    /// Follow the finger, or let go when it lifts.
    fn follow(&mut self, touches: &Touches) {
        let Some(id) = self.finger else { return };
        match touches.get_pressed(id) {
            Some(touch) => {
                let pull = touch.position() - self.origin;
                self.push = if pull.length() < STICK_DEAD {
                    Vec2::ZERO
                } else {
                    (pull / STICK_RADIUS).clamp_length_max(1.0)
                };
            }
            None => *self = Stick::default(),
        }
    }
}

/// The two sticks, and where the aim stick last pointed: the hero keeps facing it after the thumb
/// lifts, as a twin-stick shooter does, until the next run. [`super::hero::drive_hero`] adds the
/// move stick to the keys, and [`super::hero::aim_cursor`] aims along `facing`.
#[derive(Resource, Default)]
pub(super) struct TouchSticks {
    pub(super) walk: Stick,
    aim: Stick,
    pub(super) facing: Option<Vec2>,
}

/// Set on the first touch: the sticks and buttons show from then on.
#[derive(Resource, Default)]
pub(super) struct TouchMode(bool);

impl TouchMode {
    pub(super) fn on(&self) -> bool {
        self.0
    }
}

/// The finger working the UI as a mouse: its id, a release held over from the frame it landed
/// in, and whether the cursor still has to be cleared.
#[derive(Default)]
pub(super) struct UiFinger {
    id: Option<u64>,
    release_next: bool,
    clear: bool,
}

/// One of a stick's rings: the base where the finger landed, or the knob under it.
#[derive(Component, Clone, Copy, PartialEq)]
pub(super) struct StickRing {
    aim: bool,
    knob: bool,
}

/// The pause button, for its pressed look.
#[derive(Component)]
pub(super) struct PauseButton;

/// An on-screen button and the key it presses.
#[derive(Component, Clone, Copy)]
pub(super) struct TouchKey(KeyCode);

#[derive(Component)]
pub(super) struct TouchControls;

/// Read the safe area off the native window: on iOS winit reports it as the inner rectangle
/// within the outer (full-screen) one.
pub(super) fn read_safe_area(
    _main_thread: bevy::ecs::system::NonSendMarker,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut safe: ResMut<SafeArea>,
) {
    let Ok((entity, window)) = windows.single() else { return };
    let scale = window.scale_factor() as f64;
    let found = bevy::winit::WINIT_WINDOWS.with_borrow(|ww| {
        let w = ww.get_window(entity)?;
        let (outer, inner) = (w.outer_position().ok()?, w.inner_position().ok()?);
        let (outer_size, inner_size) = (w.outer_size(), w.inner_size());
        let left = (inner.x - outer.x) as f64;
        let top = (inner.y - outer.y) as f64;
        let right = outer_size.width as f64 - inner_size.width as f64 - left;
        let bottom = outer_size.height as f64 - inner_size.height as f64 - top;
        let px = |v: f64| (v.max(0.0) / scale) as f32;
        Some(SafeArea {
            left: px(left),
            top: px(top),
            right: px(right),
            bottom: px(bottom),
        })
    });
    let found = found.unwrap_or_default();
    if *safe != found {
        info!("safe area insets (logical px): {found:?}");
        *safe = found;
    }
}

/// Inset every [`SafeRoot`] by the safe area, in UI units. A root with a fill (the pause and
/// results dimmers) paints its border the same, so the fill still reaches the screen's edges.
#[allow(clippy::type_complexity)]
pub(super) fn inset_roots(
    mut commands: Commands,
    safe: Res<SafeArea>,
    ui_scale: Res<UiScale>,
    mut roots: Query<
        (Entity, &mut Node, Option<&BackgroundColor>, Option<&BorderColor>),
        With<SafeRoot>,
    >,
) {
    let s = ui_scale.0.max(1e-3);
    let want = UiRect {
        left: Val::Px(safe.left / s),
        right: Val::Px(safe.right / s),
        top: Val::Px(safe.top / s),
        bottom: Val::Px(safe.bottom / s),
    };
    for (entity, mut node, fill, edge) in &mut roots {
        if node.border != want {
            node.border = want;
        }
        if let Some(fill) = fill {
            let edge_want = BorderColor::all(fill.0);
            if edge != Some(&edge_want) {
                commands.entity(entity).insert(edge_want);
            }
        }
    }
}

/// Turn touches into the cursor, the left mouse button and the sticks, before the UI reads its
/// interactions.
///
/// In a run a finger landing on the left of the screen takes the move stick, and one landing on
/// the right off any button takes the aim stick; any other finger is the mouse: the cursor follows
/// it and the left button is down while it is. The UI then reads the mouse alone (the touches it
/// would also read are cleared), so a press and release in one frame, which a quick tap can be,
/// plays over two: down on the first, up on the next, and the button under it sees the press come
/// back inside ([`crate::glue::glue_clicks`]). The cursor stays where the finger left for that
/// frame, then clears, so nothing aims at a spot the player only tapped.
///
/// The cursor is set past change detection, as winit's own `CursorMoved` lands: a changed `Window`
/// makes bevy_winit push the position back to the platform cursor, which iOS has none of
/// ("could not set cursor position: … not supported by Winit" on every touch).
#[allow(clippy::too_many_arguments)]
pub(super) fn touch_input(
    mut touches: ResMut<Touches>,
    phase: Res<State<Phase>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut sticks: ResMut<TouchSticks>,
    mut mode: ResMut<TouchMode>,
    mut finger: Local<UiFinger>,
    buttons: Query<(&ComputedNode, &UiGlobalTransform, &InheritedVisibility), With<Interaction>>,
) {
    let Ok(mut window) = windows.single_mut() else { return };
    if finger.release_next {
        finger.release_next = false;
        finger.clear = true;
        mouse.release(MouseButton::Left);
    } else if finger.clear {
        finger.clear = false;
        window.bypass_change_detection().set_cursor_position(None);
    }
    let phase = *phase.get();
    let playing = phase == Phase::Playing;
    if !playing {
        sticks.walk = Stick::default();
        sticks.aim = Stick::default();
    }
    if phase == Phase::Menu {
        sticks.facing = None;
    }

    let scale = window.scale_factor();
    let on_button = |at: Vec2| {
        buttons.iter().any(|(node, tf, vis)| vis.get() && node.contains_point(*tf, at * scale))
    };
    let landed: Vec<(u64, Vec2)> =
        touches.iter_just_pressed().map(|t| (t.id(), t.position())).collect();
    for (id, at) in landed {
        mode.0 = true;
        if playing && sticks.walk.finger.is_none() && at.x < window.width() * STICK_ZONE {
            sticks.walk.take(id, at);
        } else if playing
            && sticks.aim.finger.is_none()
            && at.x >= window.width() * STICK_ZONE
            && !on_button(at)
        {
            sticks.aim.take(id, at);
        } else if finger.id.is_none() && !finger.release_next {
            finger.id = Some(id);
            window.bypass_change_detection().set_cursor_position(Some(at));
            mouse.press(MouseButton::Left);
        }
    }

    sticks.walk.follow(&touches);
    sticks.aim.follow(&touches);
    if sticks.aim.push != Vec2::ZERO {
        sticks.facing = Some(sticks.aim.push.normalize());
    }

    if let Some(id) = finger.id {
        if let Some(touch) = touches.get_pressed(id) {
            window.bypass_change_detection().set_cursor_position(Some(touch.position()));
        } else {
            if let Some(touch) = touches.get_released(id) {
                window.bypass_change_detection().set_cursor_position(Some(touch.position()));
            }
            finger.id = None;
            if mouse.just_pressed(MouseButton::Left) {
                finger.release_next = true;
            } else {
                mouse.release(MouseButton::Left);
                finger.clear = true;
            }
        }
    }

    // The UI reads the mouse above, not the fingers.
    let seen: Vec<u64> = touches
        .iter_just_pressed()
        .chain(touches.iter_just_released())
        .chain(touches.iter_just_canceled())
        .map(|t| t.id())
        .collect();
    for id in seen {
        touches.clear_just_pressed(id);
        touches.clear_just_released(id);
        touches.clear_just_canceled(id);
    }
}

/// Press a [`TouchKey`]'s key for the frame its button is pressed, and let it go on the next.
pub(super) fn touch_keys(
    buttons: Query<(&Interaction, &TouchKey), Changed<Interaction>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut held: Local<Vec<KeyCode>>,
) {
    for key in held.drain(..) {
        keys.release(key);
    }
    for (interaction, key) in &buttons {
        if *interaction == Interaction::Pressed {
            keys.press(key.0);
            held.push(key.0);
        }
    }
}

/// The sticks' rings and the pause button: one safe-area root over the HUD.
pub(super) fn spawn_controls(mut commands: Commands) {
    let ring = |aim: bool, knob: bool| {
        (
            StickRing { aim, knob },
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, if knob { 0.5 } else { 0.25 })),
            BorderColor::all(Color::srgba(1.0, 0.82, 0.0, 0.7)),
            bevy::ui::FocusPolicy::Pass,
            Visibility::Hidden,
        )
    };
    commands.spawn((
        TouchControls,
        SafeRoot,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        // Over the HUD (400), whose full-screen root would take the press, and over the panels
        // (410–440), which never show in a run, when these do.
        GlobalZIndex(450),
        Visibility::Hidden,
        bevy::ui::FocusPolicy::Pass,
        children![
            ring(false, false),
            ring(false, true),
            ring(true, false),
            ring(true, true),
            (
                PauseButton,
                TouchKey(KeyCode::Escape),
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(PAUSE_SIZE),
                    height: Val::Px(PAUSE_SIZE),
                    right: Val::Px(PAUSE_GAP),
                    top: Val::Px(PAUSE_TOP),
                    border: UiRect::all(Val::Px(4.0)),
                    border_radius: BorderRadius::MAX,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(PAUSE_SIZE * 0.12),
                    ..default()
                },
                BackgroundColor(PAUSE_FILL),
                BorderColor::all(GOLD),
                BoxShadow::new(
                    Color::srgba(0.0, 0.0, 0.0, 0.6),
                    Val::Px(0.0),
                    Val::Px(3.0),
                    Val::Px(2.0),
                    Val::Px(6.0),
                ),
                // The pause sign: two gold bars, rimmed dark so they read on any ground.
                children![pause_bar(), pause_bar()],
            ),
        ],
    ));
}

/// One bar of the pause sign.
fn pause_bar() -> impl Bundle {
    (
        Node {
            width: Val::Px(PAUSE_SIZE * 0.15),
            height: Val::Px(PAUSE_SIZE * 0.44),
            border: UiRect::all(Val::Px(1.5)),
            border_radius: BorderRadius::all(Val::Px(3.0)),
            ..default()
        },
        BackgroundColor(GOLD),
        BorderColor::all(GOLD_EDGE),
        bevy::ui::FocusPolicy::Pass,
    )
}

/// The pause button lights up while pressed, so a tap shows it landed.
#[allow(clippy::type_complexity)]
pub(super) fn pause_feedback(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (With<PauseButton>, Changed<Interaction>),
    >,
) {
    for (interaction, mut fill) in &mut buttons {
        fill.0 = if *interaction == Interaction::Pressed {
            PAUSE_FILL_PRESSED
        } else {
            PAUSE_FILL
        };
    }
}

/// Centre the pause button between the minimap cluster and the screen's right edge. Its root keeps
/// inside the safe area, so past the right inset its offset goes negative.
pub(super) fn place_pause(
    safe: Res<SafeArea>,
    ui_scale: Res<UiScale>,
    mut pause: Query<&mut Node, With<PauseButton>>,
) {
    let inset = safe.right / ui_scale.0.max(1e-3);
    // The strip runs from the screen's edge to the cluster's, `inset + MAP_STEP` wide.
    let right = Val::Px((MAP_STEP - inset) / 2.0 - PAUSE_SIZE / 2.0);
    for mut node in &mut pause {
        if node.right != right {
            node.right = right;
        }
    }
}

/// Once touch is in use, rework the HUD for thumbs: the mobility slot, centred at the bottom,
/// grows to thumb size and presses [`super::mobility::KEY`] (it still shows only while the class
/// has its spell, with the icon and cooldown sweep over it), its `F` goes, and the minimap steps
/// left for the pause button. The camera comes in closer.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn touch_layout(
    mut commands: Commands,
    mode: Res<TouchMode>,
    mut rig: ResMut<super::hero::CamRig>,
    mut done: Local<bool>,
    mut slot: Query<
        (Entity, &mut Node),
        (With<MobilitySlot>, Without<MobilityKeyLabel>, Without<MapCluster>),
    >,
    button: Query<Entity, With<MobilityButton>>,
    mut label: Query<&mut Node, (With<MobilityKeyLabel>, Without<MapCluster>)>,
    mut cluster: Query<&mut Node, With<MapCluster>>,
) {
    if *done || !mode.0 {
        return;
    }
    let Ok((slot_entity, mut node)) = slot.single_mut() else { return };
    *done = true;
    info!("touch: thumb layout on, camera at {CAMERA_DIST} (was {})", rig.dist);
    rig.dist = CAMERA_DIST;
    // The transform scales about the slot's centre, so the bottom gap takes the growth too; the
    // HUD's own `left: 50%` and half-width margin keep it centred.
    let grown = MOBILITY_SIZE * (MOBILITY_SCALE - 1.0) / 2.0;
    node.bottom = Val::Px(MOBILITY_GAP + grown);
    commands
        .entity(slot_entity)
        .insert(UiTransform::from_scale(Vec2::splat(MOBILITY_SCALE)));
    if let Ok(e) = button.single() {
        commands.entity(e).insert(TouchKey(super::mobility::KEY));
    }
    if let Ok(mut label) = label.single_mut() {
        label.display = Display::None;
    }
    if let Ok(mut cluster) = cluster.single_mut() {
        cluster.right = Val::Px(MAP_STEP);
    }
}

/// Show the controls in a run once touch is in use, and seat each stick's rings on its finger.
pub(super) fn show_controls(
    mode: Res<TouchMode>,
    phase: Res<State<Phase>>,
    sticks: Res<TouchSticks>,
    safe: Res<SafeArea>,
    ui_scale: Res<UiScale>,
    mut root: Query<&mut Visibility, (With<TouchControls>, Without<StickRing>)>,
    mut rings: Query<(&StickRing, &mut Node, &mut Visibility)>,
) {
    let show = mode.0 && *phase.get() == Phase::Playing;
    if let Ok(mut vis) = root.single_mut() {
        vis.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
    }
    let s = ui_scale.0.max(1e-3);
    // Window to root coordinates: the root starts at the safe area's corner.
    let to_root = |p: Vec2| (p - Vec2::new(safe.left, safe.top)) / s;
    // The rings are sized in screen pixels, as the sticks' reach is, not on the UI canvas.
    for (ring, mut node, mut vis) in &mut rings {
        let stick = if ring.aim { &sticks.aim } else { &sticks.walk };
        let held = show && stick.finger.is_some();
        vis.set_if_neq(if held { Visibility::Inherited } else { Visibility::Hidden });
        if !held {
            continue;
        }
        let origin = to_root(stick.origin);
        let (at, size) = if ring.knob {
            (origin + stick.push * STICK_RADIUS / s, STICK_RADIUS * 0.8 / s)
        } else {
            (origin, STICK_RADIUS * 2.0 / s)
        };
        node.width = Val::Px(size);
        node.height = Val::Px(size);
        node.left = Val::Px(at.x - size / 2.0);
        node.top = Val::Px(at.y - size / 2.0);
    }
}
