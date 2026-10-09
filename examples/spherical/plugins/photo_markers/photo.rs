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
//! applies the orientation a phone wrote into the file and shrinks anything longer than
//! MAX_PHOTO_EDGE, so that a 24 megapixel photo does not cost 96 MB of video memory.
//!
//! Nothing is letterboxed any more. The quad this used to go on had a fixed shape, so a photo had
//! to be padded onto it; a card has no shape of its own and takes the photo's, so what the decode
//! hands over is the photograph and the bars are gone with the frame.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy::window::FileDragAndDrop;
use image::{DynamicImage, ImageDecoder, ImageReader, imageops::FilterType};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use super::card::{self, MarkerCard};
use super::{MarkerId, PhotoMarkers};

/// The longest edge a photo is kept at, in pixels. A screen a few metres across is never drawn
/// at more than a few hundred pixels, so this is already generous; it is the cap that keeps a
/// phone's full resolution out of video memory, where each photo costs width by height by four
/// bytes with no mip levels to fall back on.
const MAX_PHOTO_EDGE: u32 = 2048;

/// What the selected card is tinted while a file hovers the window, so it is clear which marker
/// a drop would land on. A wash rather than a brightening: an empty card is already cream, and
/// there is nothing above cream to go to.
const HOVER_TINT: Color = Color::srgb(0.62, 0.78, 0.86);

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
    /// The photograph itself, RGBA, sRGB, at its own proportions.
    image: Image,
    /// What the file held, before the shrink, for the log.
    source: UVec2,
}

/// Starts a decode for every image file dropped on the window, and says why it cannot when it
/// cannot: nothing selected to put it on, or a format this build does not read.
pub(super) fn receive_drops(
    mut commands: Commands,
    mut drops: MessageReader<FileDragAndDrop>,
    markers: Res<PhotoMarkers>,
) {
    // A marker holds one photograph, so a drop of several files is one intent however many
    // files it carries. Decoding them all would put as many decodes on one card and let
    // whichever finished last have it, which is to say at random; the last file of the drop
    // wins instead, which is at least the same answer twice.
    let mut wanted: Option<PathBuf> = None;

    for drop in drops.read() {
        let FileDragAndDrop::DroppedFile { path_buf, .. } = drop else {
            continue;
        };

        let Some(_) = markers.selected() else {
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

        wanted = Some(path_buf.clone());
    }

    let (Some(path), Some(marker)) = (wanted, markers.selected()) else {
        return;
    };
    info!("photo markers: reading {}", name_of(&path));
    start_decode(&mut commands, marker.id, path);
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
    mut cards: Query<&mut ImageNode>,
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
                //
                // Only when the file that would not read is the one the marker says it is
                // showing. A photograph dropped on a marker that already has one is a different
                // file, and its failing says nothing about the picture still on the card; the
                // warning above has already said which file it was.
                if let Some(marker) = markers
                    .markers
                    .iter_mut()
                    .find(|marker| marker.id == task.marker)
                    && marker.photo.as_deref() == Some(task.path.as_path())
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

        let size = photo.image.texture_descriptor.size;
        // The card takes the photograph's shape, which is what dropping the frame bought: a
        // portrait photo gets a portrait card rather than bars down either side of a fixed one.
        marker.aspect = size.width as f32 / size.height.max(1) as f32;
        card::show_photo(marker.card, images.add(photo.image), &mut cards);
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

/// Washes the selected marker's card while a file hovers the window, and puts it back when the
/// drag leaves or lands. Only a card with no photo on it: one already showing a photograph would
/// have its colours shifted, and the wash is to say which marker is aimed at, not to preview.
///
/// A card with no photo is cream, so this cannot be a brightening the way it was on the dark
/// screen quad. It is the background that is washed rather than the image, since on an empty
/// card the image is the transparent one the renderer skips.
pub(super) fn tint_on_hover(
    mut drops: MessageReader<FileDragAndDrop>,
    markers: Res<PhotoMarkers>,
    mut cards: Query<&mut BackgroundColor, With<MarkerCard>>,
) {
    for drop in drops.read() {
        let tint = match drop {
            FileDragAndDrop::HoveredFile { .. } => Some(HOVER_TINT),
            FileDragAndDrop::HoveredFileCanceled { .. } | FileDragAndDrop::DroppedFile { .. } => {
                None
            }
        };

        let Some(marker) = markers.selected() else {
            continue;
        };
        if marker.photo.is_some() {
            continue;
        }
        let Some(mut background) = marker.card.and_then(|card| cards.get_mut(card).ok()) else {
            continue;
        };
        background.0 = tint.unwrap_or(card::CREAM);
    }
}

/// Forgets the photo on a marker: the path, the complaint about it, and the shape the card took
/// from it. Taking the picture off the card itself is card::clear_card, kept apart so that this
/// much is a plain edit of the marker and can be tested without a world to hold a node.
pub(super) fn clear_photo(
    marker_photo: &mut Option<PathBuf>,
    missing: &mut bool,
    aspect: &mut f32,
) {
    *marker_photo = None;
    *missing = false;
    *aspect = card::EMPTY_ASPECT;
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

    if source.x.max(source.y) > MAX_PHOTO_EDGE {
        photo = photo.resize(MAX_PHOTO_EDGE, MAX_PHOTO_EDGE, FilterType::CatmullRom);
    }

    Ok(Photo {
        image: texture(&photo.to_rgba8()),
        source,
    })
}

/// The decoded photograph as a texture, at its own size and proportions.
fn texture(photo: &image::RgbaImage) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: photo.width(),
            height: photo.height(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        photo.as_raw().clone(),
        // A photo is sRGB encoded, which the shader has to be told or it comes out washed out.
        TextureFormat::Rgba8UnormSrgb,
        // The main world keeps no copy: the card is the only thing that reads it, and the photo
        // on disk is where it came from if it is ever wanted again.
        RenderAssetUsages::RENDER_WORLD,
    );
    // The default sampler takes the nearest pixel, which at anything but exactly one texel to the
    // pixel makes a photo blocky. Linear is what a photograph wants.
    image.sampler = ImageSampler::linear();

    image
}
