//! The gpu memory readout in the top left, with a button to copy it to the clipboard.

use bevy::dev_tools::fps_overlay::FPS_OVERLAY_ZINDEX;
use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems, renderer::RenderDevice};
use bevy::text::FontSize;
use bevy_terrain::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Reports how much gpu memory wgpu's allocator is holding.
///
/// This is what the allocator has handed out and what it has reserved from the driver, not
/// the size of the card: wgpu exposes no portable way to ask for total or free video
/// memory. It is still the number that moves when a terrain streams in or out, since a
/// terrain's atlas textures dwarf everything else in this example.
#[derive(Resource, Clone, Default)]
struct VramUsage {
    allocated: Arc<AtomicU64>,
    reserved: Arc<AtomicU64>,
}

#[derive(Component)]
struct VramText;

#[derive(Component)]
struct CopyButton;

/// The "Copied!" note next to the button, gone again once the timer runs out.
#[derive(Component)]
struct CopiedToast(Timer);

pub struct VramUsagePlugin;

impl Plugin for VramUsagePlugin {
    fn build(&self, app: &mut App) {
        let usage = VramUsage::default();

        app.insert_resource(usage.clone())
            .add_systems(Startup, spawn_vram_text)
            .add_systems(
                Update,
                (
                    update_vram_text,
                    offset_fps_overlay,
                    copy_stats_to_clipboard,
                    highlight_copy_button,
                    expire_copied_toast,
                ),
            );

        // The allocator lives in the render world, so the counters are shared across the
        // two rather than extracted: extraction only runs main to render.
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(usage)
                .add_systems(Render, sample_vram.in_set(RenderSystems::Cleanup));
        }
    }
}

fn sample_vram(device: Res<RenderDevice>, usage: Res<VramUsage>) {
    let Some(report) = device.wgpu_device().generate_allocator_report() else {
        return;
    };

    usage
        .allocated
        .store(report.total_allocated_bytes, Ordering::Relaxed);
    usage
        .reserved
        .store(report.total_reserved_bytes, Ordering::Relaxed);
}

fn spawn_vram_text(mut commands: Commands) {
    commands
        .spawn((
            CopyButton,
            Button,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(6.0),
                left: Val::Px(420.0),
                padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        ))
        .with_child((
            Text::new("copy"),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
        ));

    commands.spawn((
        VramText,
        Text::default(),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            left: Val::Px(6.0),
            ..default()
        },
    ));
}

/// Puts the overlay text on the clipboard, so the numbers can be pasted somewhere.
///
/// On X11 whoever sets the clipboard has to keep serving it until another application
/// claims it, which is what set_clipboard_text waits for - hence the thread, since it
/// blocks.
fn copy_stats_to_clipboard(
    mut commands: Commands,
    button: Query<&Interaction, (Changed<Interaction>, With<CopyButton>)>,
    text: Single<&Text, With<VramText>>,
    mut toast: Query<&mut CopiedToast>,
) {
    for interaction in &button {
        if *interaction != Interaction::Pressed {
            continue;
        }

        // One toast at a time: a second press while it is showing just restarts the clock.
        if let Ok(mut toast) = toast.single_mut() {
            toast.0.reset();
        } else {
            commands.spawn((
                CopiedToast(Timer::from_seconds(3.0, TimerMode::Once)),
                Text::new("Copied!"),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(8.0),
                    // Just right of the button.
                    left: Val::Px(476.0),
                    ..default()
                },
            ));
        }

        let stats = text.0.clone();

        std::thread::spawn(move || {
            match arboard::Clipboard::new() {
                Ok(mut clipboard) => {
                    if let Err(error) = set_clipboard_text(&mut clipboard, stats) {
                        error!("could not set the clipboard: {error}");
                    }
                }
                Err(error) => error!("could not reach the clipboard: {error}"),
            };
        });
    }
}

/// Linux only, as arboard has it: block until another application has taken the
/// clipboard over, since on X11 the setter serves the contents until then. The platforms
/// below have a clipboard that keeps the text itself, and no wait() to call.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
))]
fn set_clipboard_text(
    clipboard: &mut arboard::Clipboard,
    text: String,
) -> Result<(), arboard::Error> {
    use arboard::SetExtLinux;

    clipboard.set().wait().text(text)
}

#[cfg(not(all(
    unix,
    not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
)))]
fn set_clipboard_text(
    clipboard: &mut arboard::Clipboard,
    text: String,
) -> Result<(), arboard::Error> {
    clipboard.set_text(text)
}

fn expire_copied_toast(
    mut commands: Commands,
    time: Res<Time>,
    mut toasts: Query<(Entity, &mut CopiedToast)>,
) {
    for (entity, mut toast) in &mut toasts {
        if toast.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// Gives the copy button a hover and a press state, so it reads as a button.
fn highlight_copy_button(
    mut button: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<CopyButton>),
    >,
) {
    for (interaction, mut background) in &mut button {
        background.0 = match interaction {
            Interaction::None => Color::srgba(0.0, 0.0, 0.0, 0.5),
            Interaction::Hovered => Color::srgba(0.35, 0.35, 0.35, 0.8),
            Interaction::Pressed => Color::srgba(0.6, 0.6, 0.6, 0.9),
        };
    }
}

/// Moves the fps overlay down so the vram line can have the top left corner.
///
/// The overlay spawns at the origin and exposes no position setting, but its root is the
/// node carrying FPS_OVERLAY_ZINDEX, which is public.
fn offset_fps_overlay(mut overlay: Query<(&mut Node, &GlobalZIndex), Added<GlobalZIndex>>) {
    for (mut node, z_index) in &mut overlay {
        if z_index.0 == FPS_OVERLAY_ZINDEX {
            node.top = Val::Px(52.0);
            node.left = Val::Px(6.0);
        }
    }
}

fn update_vram_text(
    usage: Res<VramUsage>,
    atlases: Query<&TileAtlas>,
    mut text: Single<&mut Text, With<VramText>>,
) {
    const GIB: f64 = (1u64 << 30) as f64;

    let allocated = usage.allocated.load(Ordering::Relaxed) as f64 / GIB;
    let reserved = usage.reserved.load(Ordering::Relaxed) as f64 / GIB;

    // Slots run out before memory does, and unlike the figures above they respond to
    // frustum culling: the atlas texture is allocated whole, however little of it is used.
    let (used_slots, total_slots) = atlases
        .iter()
        .map(TileAtlas::slot_usage)
        .fold((0, 0), |(used, total), (u, t)| (used + u, total + t));

    text.0 = format!(
        "VRAM used / alloc {allocated:.2} / {reserved:.2} GiB\n\
         atlas slots {used_slots} / {total_slots} ({} terrains)",
        atlases.iter().len()
    );
}
