//! Touch play, for phones and tablets: a tap works the UI as a left click, a floating stick on the
//! left of the screen moves the hero, and two on-screen buttons stand in for the mobility key
//! ([`super::mobility::KEY`]) and Escape. Nothing here shows until the first touch, so a desktop
//! with a mouse never sees it.
//!
//! The mode's UI roots ([`SafeRoot`]) keep inside the device's safe area ([`SafeArea`]): an
//! invisible border of the inset on every side, which shifts their absolutely placed children too.

use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::Phase;

/// The stick's reach: a finger this far from where it landed is full speed, in logical pixels.
const STICK_RADIUS: f32 = 70.0;
/// Below this the stick rests, so a thumb laid down without moving does not creep the hero.
const STICK_DEAD: f32 = 8.0;
/// The share of the screen's width, from the left, where a touch in a run takes the stick.
const STICK_ZONE: f32 = 0.45;
/// The on-screen buttons' size, in UI units (the 768-tall canvas, as the HUD).
const BUTTON: f32 = 92.0;
const BUTTON_SMALL: f32 = 56.0;
const BUTTON_GAP: f32 = 24.0;
/// Pause sits under the minimap cluster, which holds the top-right corner.
const PAUSE_TOP: f32 = 190.0;

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

/// The floating stick: the finger holding it, where it landed, and the push, `y` down the screen,
/// at most unit length. [`super::hero::drive_hero`] adds it to the keys.
#[derive(Resource, Default)]
pub(super) struct TouchStick {
    finger: Option<u64>,
    origin: Vec2,
    pub(super) push: Vec2,
}

/// Set on the first touch: the stick and buttons show from then on.
#[derive(Resource, Default)]
pub(super) struct TouchMode(bool);

/// The finger working the UI as a mouse: its id, a release held over from the frame it landed
/// in, and whether the cursor still has to be cleared.
#[derive(Default)]
pub(super) struct UiFinger {
    id: Option<u64>,
    release_next: bool,
    clear: bool,
}

/// One of the stick's rings: the base where the finger landed, or the knob under it.
#[derive(Component, Clone, Copy, PartialEq)]
pub(super) enum StickRing {
    Base,
    Knob,
}

/// An on-screen button and the key it presses.
#[derive(Component, Clone, Copy)]
pub(super) struct TouchKey(KeyCode);

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

/// Turn touches into the cursor, the left mouse button and the stick, before the UI reads its
/// interactions.
///
/// A finger landing on the left of the screen during a run takes the stick; any other finger is
/// the mouse: the cursor follows it and the left button is down while it is. The UI then reads the
/// mouse alone (the touches it would also read are cleared), so a press and release in one frame,
/// which a quick tap can be, plays over two: down on the first, up on the next, and the button
/// under it sees the press come back inside ([`crate::glue::glue_clicks`]). The cursor stays where
/// the finger left for that frame, then clears so the hero's aim falls back to where he runs.
pub(super) fn touch_input(
    mut touches: ResMut<Touches>,
    phase: Res<State<Phase>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut stick: ResMut<TouchStick>,
    mut mode: ResMut<TouchMode>,
    mut finger: Local<UiFinger>,
) {
    let Ok(mut window) = windows.single_mut() else { return };
    if finger.release_next {
        finger.release_next = false;
        finger.clear = true;
        mouse.release(MouseButton::Left);
    } else if finger.clear {
        finger.clear = false;
        window.set_cursor_position(None);
    }
    let playing = *phase.get() == Phase::Playing;
    if !playing && stick.finger.is_some() {
        *stick = TouchStick::default();
    }

    let landed: Vec<(u64, Vec2)> =
        touches.iter_just_pressed().map(|t| (t.id(), t.position())).collect();
    for (id, at) in landed {
        mode.0 = true;
        if playing && stick.finger.is_none() && at.x < window.width() * STICK_ZONE {
            stick.finger = Some(id);
            stick.origin = at;
            stick.push = Vec2::ZERO;
        } else if finger.id.is_none() && !finger.release_next {
            finger.id = Some(id);
            window.set_cursor_position(Some(at));
            mouse.press(MouseButton::Left);
        }
    }

    if let Some(id) = stick.finger {
        match touches.get_pressed(id) {
            Some(touch) => {
                let pull = touch.position() - stick.origin;
                stick.push = if pull.length() < STICK_DEAD {
                    Vec2::ZERO
                } else {
                    (pull / STICK_RADIUS).clamp_length_max(1.0)
                };
            }
            None => *stick = TouchStick::default(),
        }
    }

    if let Some(id) = finger.id {
        if let Some(touch) = touches.get_pressed(id) {
            window.set_cursor_position(Some(touch.position()));
        } else {
            if let Some(touch) = touches.get_released(id) {
                window.set_cursor_position(Some(touch.position()));
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

/// The stick's two rings and the buttons: one safe-area root over the HUD.
pub(super) fn spawn_controls(mut commands: Commands) {
    let ring = |size: f32, alpha: f32| {
        (
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(size),
                height: Val::Px(size),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, alpha)),
            BorderColor::all(Color::srgba(1.0, 0.82, 0.0, 0.7)),
            bevy::ui::FocusPolicy::Pass,
        )
    };
    let button = |key: KeyCode, label: &str, size: f32, top: Val, bottom: Val| {
        (
            TouchKey(key),
            Button,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(size),
                height: Val::Px(size),
                right: Val::Px(BUTTON_GAP),
                top,
                bottom,
                border: UiRect::all(Val::Px(3.0)),
                border_radius: BorderRadius::MAX,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderColor::all(Color::srgba(1.0, 0.82, 0.0, 0.85)),
            children![(
                Text::new(label),
                TextFont::from_font_size(size * 0.3),
                TextColor(Color::srgb(1.0, 0.82, 0.0)),
            )],
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
            (StickRing::Base, ring(STICK_RADIUS * 2.0, 0.25), Visibility::Hidden),
            (StickRing::Knob, ring(STICK_RADIUS * 0.8, 0.5), Visibility::Hidden),
            button(super::mobility::KEY, "F", BUTTON, Val::Auto, Val::Px(BUTTON_GAP * 2.0)),
            button(KeyCode::Escape, "II", BUTTON_SMALL, Val::Px(PAUSE_TOP), Val::Auto),
        ],
    ));
}

#[derive(Component)]
pub(super) struct TouchControls;

/// Show the controls in a run once touch is in use, and seat the stick's rings on the finger.
pub(super) fn show_controls(
    mode: Res<TouchMode>,
    phase: Res<State<Phase>>,
    stick: Res<TouchStick>,
    safe: Res<SafeArea>,
    ui_scale: Res<UiScale>,
    mut root: Query<&mut Visibility, (With<TouchControls>, Without<StickRing>)>,
    mut rings: Query<(&StickRing, &mut Node, &mut Visibility)>,
) {
    let show = mode.0 && *phase.get() == Phase::Playing;
    if let Ok(mut vis) = root.single_mut() {
        vis.set_if_neq(if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    let held = show && stick.finger.is_some();
    let s = ui_scale.0.max(1e-3);
    // Window to root coordinates: the root starts at the safe area's corner.
    let to_root = |p: Vec2| (p - Vec2::new(safe.left, safe.top)) / s;
    let origin = to_root(stick.origin);
    let knob_at = origin + stick.push * STICK_RADIUS / s;
    // The rings are sized in screen pixels, as the stick's reach is, not on the UI canvas.
    for (ring, mut node, mut vis) in &mut rings {
        vis.set_if_neq(if held {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !held {
            continue;
        }
        let (at, size) = match ring {
            StickRing::Base => (origin, STICK_RADIUS * 2.0 / s),
            StickRing::Knob => (knob_at, STICK_RADIUS * 0.8 / s),
        };
        node.width = Val::Px(size);
        node.height = Val::Px(size);
        node.left = Val::Px(at.x - size / 2.0);
        node.top = Val::Px(at.y - size / 2.0);
    }
}
