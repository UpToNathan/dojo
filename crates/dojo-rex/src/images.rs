//! Kitty graphics: the images a terminal shows, as placements with their pixels. Programs send
//! images in the VT stream (or name a file the mirror can read, on this machine), and a Rex
//! snapshot carries the ones already shown.
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::{Arc, Once};

use crate::ffi::*;

/// Images and placements kept per terminal: Ghostty's own default.
const STORAGE_LIMIT: u64 = 320 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImageLayer {
    BelowBackground,
    BelowText,
    AboveText,
}

/// One placement of one image, in device pixels and viewport cells.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// Changes when the pixels do: the image, its generation and the part shown.
    pub key: String,
    /// Straight-alpha RGBA of just the part shown.
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// The top-left cell, which may be above or left of the viewport.
    pub col: i32,
    pub row: i32,
    /// Where in that cell the image starts.
    pub offset_x: u32,
    pub offset_y: u32,
    /// The size to draw at.
    pub draw_width: u32,
    pub draw_height: u32,
    pub layer: ImageLayer,
    pub z: i32,
}

/// Lets the mirror decode PNG images; libghostty asks its embedder for that. Process-wide.
pub fn install_png_decoder() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let decode: GhosttySysDecodePngFn = Some(decode_png);
        ghostty_sys_set(GHOSTTY_SYS_OPT_DECODE_PNG, decode.map_or(ptr::null(), |f| f as *const c_void));
    });
}

unsafe extern "C" fn decode_png(
    _userdata: *mut c_void,
    allocator: *const GhosttyAllocator,
    data: *const u8,
    data_len: usize,
    out: *mut GhosttySysImage,
) -> bool {
    let bytes = unsafe { std::slice::from_raw_parts(data, data_len) };
    let Some((width, height, rgba)) = png_rgba(bytes) else { return false };
    let buffer = unsafe { ghostty_alloc(allocator, rgba.len()) };
    if buffer.is_null() {
        return false;
    }
    unsafe {
        ptr::copy_nonoverlapping(rgba.as_ptr(), buffer, rgba.len());
        *out = GhosttySysImage { width, height, data: buffer, data_len: rgba.len() };
    }
    true
}

fn png_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..frame.buffer_size()];
    let rgba = match frame.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((frame.width, frame.height, rgba))
}

/// Turns Kitty graphics on for a mirror. File, temporary-file and shared-memory transmissions
/// name things on this machine, which the mirror can read as the server's terminal does.
pub fn enable(terminal: GhosttyTerminal) {
    let limit = STORAGE_LIMIT;
    let on = true;
    unsafe {
        ghostty_terminal_set(terminal, GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT, &limit as *const u64 as *const c_void);
        for option in [GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_FILE, GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_SHARED_MEM] {
            ghostty_terminal_set(terminal, option, &on as *const bool as *const c_void);
        }
        // Temporary files are read only from the temporary directory.
        let directory = std::env::temp_dir();
        let directory = directory.as_os_str().as_encoded_bytes();
        let directory = GhosttyString { ptr: directory.as_ptr(), len: directory.len() };
        ghostty_terminal_set(terminal, GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_TEMP_FILE, &directory as *const GhosttyString as *const c_void);
    }
}

/// Converted pixels by image, generation and part, so a frame doesn't copy them again.
#[derive(Default)]
pub struct PixelCache(HashMap<String, Arc<Vec<u8>>>);

/// The placements the render state holds, bottom layer first and by z-index within a layer.
///
/// # Safety
/// `render` must have been updated from `terminal`, and `iterator` must be a live iterator.
pub unsafe fn collect(
    terminal: GhosttyTerminal,
    render: GhosttyRenderState,
    iterator: &mut GhosttyRenderStateImageIterator,
    cache: &mut PixelCache,
) -> Vec<Image> {
    let mut graphics: GhosttyKittyGraphics = ptr::null_mut();
    unsafe {
        if ghostty_terminal_get(terminal, GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS, &mut graphics as *mut _ as *mut c_void) != GHOSTTY_SUCCESS
            || ghostty_render_state_get(render, GHOSTTY_RENDER_STATE_DATA_IMAGE_ITERATOR, iterator as *mut _ as *mut c_void) != GHOSTTY_SUCCESS
        {
            return Vec::new();
        }
    }
    let mut seen = HashMap::new();
    let mut images = Vec::new();
    while unsafe { ghostty_render_state_image_iterator_next(*iterator) } {
        let get = |key, out: *mut c_void| unsafe { ghostty_render_state_image_get(*iterator, key, out) == GHOSTTY_SUCCESS };
        let mut protocol: GhosttyImageProtocol = Default::default();
        if !get(GHOSTTY_RENDER_STATE_IMAGE_DATA_PROTOCOL, &mut protocol as *mut _ as *mut c_void) || protocol != GHOSTTY_IMAGE_PROTOCOL_KITTY {
            continue;
        }
        macro_rules! read {
            ($key:ident, $ty:ty) => {{
                let mut value: $ty = Default::default();
                get($key, &mut value as *mut $ty as *mut c_void);
                value
            }};
        }
        let id = read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_ID, u32);
        let generation = read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_GENERATION, u64);
        let source = [
            read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_X, u32),
            read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_Y, u32),
            read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_WIDTH, u32),
            read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_HEIGHT, u32),
        ];
        let key = format!("{id}:{generation}:{}:{}:{}:{}", source[0], source[1], source[2], source[3]);
        let rgba = match cache.0.get(&key) {
            Some(rgba) => rgba.clone(),
            None => {
                let Some(rgba) = (unsafe { image_rgba(graphics, id, source) }) else { continue };
                let rgba = Arc::new(rgba);
                cache.0.insert(key.clone(), rgba.clone());
                rgba
            }
        };
        seen.insert(key.clone(), ());
        let layer: GhosttyRenderStateImageLayer = read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_LAYER, GhosttyRenderStateImageLayer);
        images.push(Image {
            key,
            rgba,
            width: source[2],
            height: source[3],
            col: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_COL, i32),
            row: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_ROW, i32),
            offset_x: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_X, u32),
            offset_y: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_Y, u32),
            draw_width: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_WIDTH, u32),
            draw_height: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_HEIGHT, u32),
            layer: match layer {
                GHOSTTY_RENDER_STATE_IMAGE_LAYER_BELOW_BACKGROUND => ImageLayer::BelowBackground,
                GHOSTTY_RENDER_STATE_IMAGE_LAYER_BELOW_TEXT => ImageLayer::BelowText,
                _ => ImageLayer::AboveText,
            },
            z: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_Z, i32),
        });
    }
    // Pixels of images no longer shown are let go.
    cache.0.retain(|key, _| seen.contains_key(key));
    images.sort_by_key(|image| (image.layer, image.z));
    images
}

/// The shown part of an image as straight-alpha RGBA.
unsafe fn image_rgba(graphics: GhosttyKittyGraphics, id: u32, [sx, sy, sw, sh]: [u32; 4]) -> Option<Vec<u8>> {
    let image = unsafe { ghostty_kitty_graphics_image(graphics, id) };
    if image.is_null() {
        return None;
    }
    let get = |key, out: *mut c_void| unsafe { ghostty_kitty_graphics_image_get(image, key, out) == GHOSTTY_SUCCESS };
    let (mut width, mut height, mut len) = (0u32, 0u32, 0usize);
    let mut format: GhosttyKittyImageFormat = Default::default();
    let mut data: *const u8 = ptr::null();
    if !(get(GHOSTTY_KITTY_IMAGE_DATA_WIDTH, &mut width as *mut _ as *mut c_void)
        && get(GHOSTTY_KITTY_IMAGE_DATA_HEIGHT, &mut height as *mut _ as *mut c_void)
        && get(GHOSTTY_KITTY_IMAGE_DATA_FORMAT, &mut format as *mut _ as *mut c_void)
        && get(GHOSTTY_KITTY_IMAGE_DATA_DATA_PTR, &mut data as *mut _ as *mut c_void)
        && get(GHOSTTY_KITTY_IMAGE_DATA_DATA_LEN, &mut len as *mut _ as *mut c_void))
        || data.is_null()
    {
        return None;
    }
    let pixels = unsafe { std::slice::from_raw_parts(data, len) };
    let channels = match format {
        GHOSTTY_KITTY_IMAGE_FORMAT_RGBA => 4,
        GHOSTTY_KITTY_IMAGE_FORMAT_RGB => 3,
        GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA => 2,
        GHOSTTY_KITTY_IMAGE_FORMAT_GRAY => 1,
        _ => return None,
    };
    crop_rgba(pixels, width, height, channels, [sx, sy, sw, sh])
}

/// The `[x, y, w, h]` part of an image with `channels` bytes a pixel, as RGBA.
fn crop_rgba(pixels: &[u8], width: u32, height: u32, channels: usize, [x, y, w, h]: [u32; 4]) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || x + w > width || y + h > height || pixels.len() < (width * height) as usize * channels {
        return None;
    }
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = (row * width + x) as usize * channels;
        for p in pixels[start..start + w as usize * channels].chunks_exact(channels) {
            out.extend_from_slice(&match channels {
                4 => [p[0], p[1], p[2], p[3]],
                3 => [p[0], p[1], p[2], 255],
                2 => [p[0], p[0], p[0], p[1]],
                _ => [p[0], p[0], p[0], 255],
            });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_of_an_rgb_image_becomes_rgba() {
        // 3x2 RGB: pixel n is [n, n, n].
        let pixels: Vec<u8> = (0..6u8).flat_map(|n| [n, n, n]).collect();
        let part = crop_rgba(&pixels, 3, 2, 3, [1, 0, 2, 2]).unwrap();
        assert_eq!(part, [1, 1, 1, 255, 2, 2, 2, 255, 4, 4, 4, 255, 5, 5, 5, 255]);
        assert!(crop_rgba(&pixels, 3, 2, 3, [2, 0, 2, 1]).is_none(), "past the edge");
    }

    #[test]
    fn png_decodes_to_rgba() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        assert_eq!(png_rgba(&bytes), Some((2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255])));
    }
}
