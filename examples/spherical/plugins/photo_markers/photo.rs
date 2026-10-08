//! The photos dropped onto the markers: reading the file, decoding it off the main thread, and
//! putting the result on the selected marker's screen.
//!
//! A drop cannot be aimed. The event the window system hands Bevy carries the file's path and
//! nothing else, no pointer position, and on X11 the pointer is not reported while a drag is in
//! flight, so there is no way to tell which screen a file was let go over. The photo goes to the
//! selected marker instead, and while a file hovers the window that marker's screen brightens,
//! so where it will land can be seen before it is let go.
//!
//! Decoding is a job for another thread: a twelve megapixel jpeg takes hundreds of milliseconds,
//! which on the main one would be a visible hitch. The task is polled each frame, the shape the
//! live feed's fetch uses. What it returns is already the texture's pixels: the image crate
//! applies the orientation a phone wrote into the file, shrinks anything longer than
//! MAX_PHOTO_EDGE so that a 24 megapixel photo does not cost 96 MB of video memory, and letterboxes
//! the result onto a canvas the shape of the screen quad, so that the photo keeps its proportions
//! and the material needs no transform of its own.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy::window::FileDragAndDrop;
use image::{DynamicImage, ImageDecoder, ImageReader, imageops::FilterType};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use super::model::SCREEN_ASPECT;
use super::{BLANK_SCREEN, MarkerId, PhotoMarkers};

/// The longest edge a photo is kept at, in pixels. A screen a few metres across is never drawn
/// at more than a few hundred pixels, so this is already generous; it is the cap that keeps a
/// phone's full resolution out of video memory, where each photo costs width by height by four
/// bytes with no mip levels to fall back on.
const MAX_PHOTO_EDGE: u32 = 2048;

/// The bars a letterboxed photo is padded with: the blank screen's colour, so a portrait photo
/// looks like a picture on a screen rather than one floating on a grey card.
const LETTERBOX: [u8; 4] = [20, 23, 26, 255];

/// How much brighter the selected screen goes while a file hovers the window, so it is clear
/// which marker a drop would land on.
const HOVER_TINT: Color = Color::srgb(0.45, 0.5, 0.55);

/// The formats this build can decode. Bevy turns on png by default and the example adds jpeg for
/// the carriage's textures; webp, heic, tiff and the rest are not compiled in, so a drop of one
/// is refused by name rather than failing somewhere deeper with less to say.
const READABLE: [&str; 3] = ["png", "jpg", "jpeg"];

/// A decode in flight, on a throwaway entity so that several can run at once and one finishing
/// does not disturb the others.
#[derive(Component)]
pub(super) struct PhotoTask {
    marker: MarkerId,
    path: PathBuf,
    task: Task<Result<Photo, String>>,
}

/// A decoded photo, ready to become a texture.
pub(super) struct Photo {
    /// The canvas: the photo letterboxed onto the screen's shape, RGBA, sRGB.
    image: Image,
    /// What the file held, before the shrink and the letterbox, for the log.
    source: UVec2,
}

/// Starts a decode for every image file dropped on the window, and says why it cannot when it
/// cannot: nothing selected to put it on, or a format this build does not read.
pub(super) fn receive_drops(
    mut commands: Commands,
    mut drops: MessageReader<FileDragAndDrop>,
    markers: Res<PhotoMarkers>,
) {
    for drop in drops.read() {
        let FileDragAndDrop::DroppedFile { path_buf, .. } = drop else {
            continue;
        };

        let Some(marker) = markers.selected() else {
            warn!(
                "photo markers: {} dropped with no marker selected; Ctrl+click the terrain to \
                 place one, or click one to select it",
                name_of(path_buf)
            );
            continue;
        };

        if let Err(message) = readable(path_buf) {
            warn!("photo markers: {message}");
            continue;
        }

        start_decode(&mut commands, marker.id, path_buf.clone());
        info!("photo markers: reading {}", name_of(path_buf));
    }
}

/// Starts a decode for one marker, on the task pool, and leaves it on a throwaway entity for
/// poll_photos to pick up. Both a drop and a marker coming back from the file go through here.
pub(super) fn start_decode(commands: &mut Commands, marker: MarkerId, path: PathBuf) {
    let decoding = path.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move { decode_photo(&decoding) });

    commands.spawn(PhotoTask { marker, path, task });
}

/// Puts a finished photo on its marker's screen, and drops the task either way. The marker is
/// found by id rather than by index, since the list can have been edited while the task ran, and
/// a photo for a marker that has gone is simply let go of.
pub(super) fn poll_photos(
    mut commands: Commands,
    mut markers: ResMut<PhotoMarkers>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tasks: Query<(Entity, &mut PhotoTask)>,
) {
    for (entity, mut task) in &mut tasks {
        let Some(result) = block_on(poll_once(&mut task.task)) else {
            continue;
        };
        commands.entity(entity).despawn();

        let photo = match result {
            Ok(photo) => photo,
            Err(message) => {
                warn!("photo markers: {message}");
                // Said in the panel too, since a marker whose photo has moved comes back blank
                // and the console scrolls away. The path is kept, so putting the file back and
                // starting again is all it takes.
                if let Some(marker) = markers
                    .markers
                    .iter_mut()
                    .find(|marker| marker.id == task.marker)
                {
                    marker.missing = true;
                }
                continue;
            }
        };

        let Some(marker) = markers
            .markers
            .iter_mut()
            .find(|marker| marker.id == task.marker)
        else {
            continue;
        };
        let Some(handle) = marker.screen_material.clone() else {
            continue;
        };
        let Some(mut material) = materials.get_mut(&handle) else {
            continue;
        };

        let size = photo.image.texture_descriptor.size;
        material.base_color_texture = Some(images.add(photo.image));
        // White, so the photo shows at its own colours rather than tinted by the blank screen.
        material.base_color = Color::WHITE;
        marker.photo = Some(task.path.clone());
        marker.missing = false;
        markers.dirty = true;

        info!(
            "photo markers: {} on screen, {} by {} from {} by {}",
            name_of(&task.path),
            size.width,
            size.height,
            photo.source.x,
            photo.source.y,
        );
    }
}

/// Brightens the selected marker's screen while a file hovers the window, and puts it back when
/// the drag leaves or lands. Only a screen with no photo on it is tinted: one already showing a
/// photo would have its picture discoloured, and the brightening is to say which marker is aimed
/// at, not to preview anything.
pub(super) fn tint_on_hover(
    mut drops: MessageReader<FileDragAndDrop>,
    markers: Res<PhotoMarkers>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for drop in drops.read() {
        let tint = match drop {
            FileDragAndDrop::HoveredFile { .. } => HOVER_TINT,
            FileDragAndDrop::HoveredFileCanceled { .. } | FileDragAndDrop::DroppedFile { .. } => {
                BLANK_SCREEN
            }
        };

        let Some(marker) = markers.selected() else {
            continue;
        };
        if marker.photo.is_some() {
            continue;
        }
        let Some(handle) = &marker.screen_material else {
            continue;
        };
        let Some(mut material) = materials.get_mut(handle) else {
            continue;
        };
        material.base_color = tint;
    }
}

/// Takes the photo off a marker's screen and leaves it blank again.
pub(super) fn clear_photo(
    marker_photo: &mut Option<PathBuf>,
    missing: &mut bool,
    handle: Option<&Handle<StandardMaterial>>,
    materials: &mut Assets<StandardMaterial>,
) {
    *marker_photo = None;
    *missing = false;

    let Some(mut material) = handle.and_then(|handle| materials.get_mut(handle)) else {
        return;
    };
    material.base_color_texture = None;
    material.base_color = BLANK_SCREEN;
}

/// The file's name, for a message. Dropped paths come from outside, so only the name is shown and
/// the whole path is kept out of the log.
fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Whether this build can decode the file, by its extension, and what to say when it cannot.
pub(super) fn readable(path: &Path) -> Result<&'static str, String> {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    READABLE
        .iter()
        .find(|readable| **readable == extension)
        .copied()
        .ok_or_else(|| match extension.is_empty() {
            true => format!("{} has no extension; png and jpeg are read", name_of(path)),
            false => format!(
                "{} is a {extension}, which this build does not read; png and jpeg are",
                name_of(path)
            ),
        })
}

/// Reads and decodes a photo, ready to go on a screen. Off the main thread, so it may take its
/// time; everything that can go wrong comes back as a sentence for the log.
///
/// The asset server is not used, and could not be: a dropped path is absolute and outside the
/// asset root, which it resolves everything against.
pub(super) fn decode_photo(path: &Path) -> Result<Photo, String> {
    readable(path)?;

    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", name_of(path)))?;

    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("{}: {error}", name_of(path)))?
        .into_decoder()
        .map_err(|error| format!("{}: {error}", name_of(path)))?;

    // What the camera wrote into the file about which way up it was held. Without this a photo
    // taken in portrait arrives on its side.
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);

    let mut photo = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("{}: {error}", name_of(path)))?;
    photo.apply_orientation(orientation);

    let source = UVec2::new(photo.width(), photo.height());

    // Shrunk before the letterbox, so the canvas is built from no more pixels than it needs.
    if source.x.max(source.y) > MAX_PHOTO_EDGE {
        photo = photo.resize(MAX_PHOTO_EDGE, MAX_PHOTO_EDGE, FilterType::CatmullRom);
    }

    Ok(Photo {
        image: canvas(&photo.to_rgba8()),
        source,
    })
}

/// The photo centred on a canvas of the screen's shape, padded with bars where it does not reach.
/// Nothing is cropped and nothing is stretched: a wide photo gets bars above and below, a tall
/// one bars to either side, and one already the screen's shape gets none.
fn canvas(photo: &image::RgbaImage) -> Image {
    let (width, height) = (photo.width(), photo.height());
    let (canvas_width, canvas_height) = canvas_size(UVec2::new(width, height));

    let mut pixels = LETTERBOX.repeat((canvas_width * canvas_height) as usize);
    let left = (canvas_width - width) / 2;
    let top = (canvas_height - height) / 2;

    for y in 0..height {
        let row = ((top + y) * canvas_width + left) as usize * 4;
        let source = (y * width) as usize * 4;
        let length = width as usize * 4;
        pixels[row..row + length].copy_from_slice(&photo.as_raw()[source..source + length]);
    }

    let mut image = Image::new(
        Extent3d {
            width: canvas_width,
            height: canvas_height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        // A photo is sRGB encoded, which the shader has to be told or it comes out washed out.
        TextureFormat::Rgba8UnormSrgb,
        // The main world keeps no copy: the screen is the only thing that reads it, and the
        // photo on disk is where it came from if it is ever wanted again.
        RenderAssetUsages::RENDER_WORLD,
    );
    // The default sampler takes the nearest pixel, which at anything but exactly one texel to the
    // pixel makes a photo blocky. Linear is what a photograph wants.
    image.sampler = ImageSampler::linear();

    image
}

/// The canvas a photo of this size is centred on: the screen's shape, no smaller than the photo
/// in either direction, so that nothing is lost. Pure, and the one piece worth a test.
pub(super) fn canvas_size(photo: UVec2) -> (u32, u32) {
    let photo = photo.max(UVec2::ONE);
    let wanted = (photo.x as f32 / photo.y as f32) / SCREEN_ASPECT;

    if wanted >= 1.0 {
        // Wider than the screen: keep the width and grow the height, bars above and below.
        let height = (photo.x as f32 / SCREEN_ASPECT).round() as u32;
        (photo.x, height.max(photo.y))
    } else {
        // Taller: keep the height and grow the width, bars to either side.
        let width = (photo.y as f32 * SCREEN_ASPECT).round() as u32;
        (width.max(photo.x), photo.y)
    }
}
