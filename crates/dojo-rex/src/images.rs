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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// Where the image starts from that cell's top-left corner; past the cell, or before it,
    /// when pieces were merged.
    pub offset_x: i32,
    pub offset_y: i32,
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

/// Advances playing animations to the frame due at `now_ms` (a clock that started with the
/// terminal), and says when the next frame is due on that clock; None when nothing plays.
pub fn tick(terminal: GhosttyTerminal, now_ms: u64) -> Option<u64> {
    let mut graphics: GhosttyKittyGraphics = ptr::null_mut();
    let mut next = 0u64;
    unsafe {
        (ghostty_terminal_get(terminal, GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS, &mut graphics as *mut _ as *mut c_void) == GHOSTTY_SUCCESS
            && ghostty_kitty_graphics_animation_tick(graphics, now_ms, &mut next) == GHOSTTY_SUCCESS)
            .then_some(next)
    }
}

/// The image storage's generation: it changes whenever a program sends, places or deletes an
/// image. Placeholder cells can be drawn before their image arrives, and the image's arrival
/// then changes this without changing the render state's placements generation.
pub fn storage_generation(terminal: GhosttyTerminal) -> u64 {
    let mut graphics: GhosttyKittyGraphics = ptr::null_mut();
    let mut generation = 0u64;
    unsafe {
        if ghostty_terminal_get(terminal, GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS, &mut graphics as *mut _ as *mut c_void) == GHOSTTY_SUCCESS {
            ghostty_kitty_graphics_get(graphics, GHOSTTY_KITTY_GRAPHICS_DATA_GENERATION, &mut generation as *mut u64 as *mut c_void);
        }
    }
    generation
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
    cell_px: [u32; 2],
) -> Vec<Image> {
    let mut graphics: GhosttyKittyGraphics = ptr::null_mut();
    unsafe {
        if ghostty_terminal_get(terminal, GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS, &mut graphics as *mut _ as *mut c_void) != GHOSTTY_SUCCESS
            || ghostty_render_state_get(render, GHOSTTY_RENDER_STATE_DATA_IMAGE_ITERATOR, iterator as *mut _ as *mut c_void) != GHOSTTY_SUCCESS
        {
            return Vec::new();
        }
    }
    let mut pieces = Vec::new();
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
        let layer: GhosttyRenderStateImageLayer = read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_LAYER, GhosttyRenderStateImageLayer);
        let kind: GhosttyRenderStateKittyPlacementKind = read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_KITTY_KIND, GhosttyRenderStateKittyPlacementKind);
        pieces.push(Piece {
            id: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_ID, u32),
            generation: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_GENERATION, u64),
            placement: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_KITTY_PLACEMENT_ID, u32),
            virtual_: kind == GHOSTTY_RENDER_STATE_KITTY_PLACEMENT_KIND_VIRTUAL,
            source: [
                read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_X, u32),
                read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_Y, u32),
                read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_WIDTH, u32),
                read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_HEIGHT, u32),
            ],
            col: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_COL, i32),
            row: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_ROW, i32),
            offset: [
                read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_X, u32) as i32,
                read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_Y, u32) as i32,
            ],
            draw: [read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_WIDTH, u32), read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_HEIGHT, u32)],
            layer: match layer {
                GHOSTTY_RENDER_STATE_IMAGE_LAYER_BELOW_BACKGROUND => ImageLayer::BelowBackground,
                GHOSTTY_RENDER_STATE_IMAGE_LAYER_BELOW_TEXT => ImageLayer::BelowText,
                _ => ImageLayer::AboveText,
            },
            z: read!(GHOSTTY_RENDER_STATE_IMAGE_DATA_Z, i32),
        });
    }
    let mut seen = HashMap::new();
    let mut images = Vec::new();
    for piece in merge_tiles(pieces, cell_px) {
        let source = piece.source;
        let key = format!("{}:{}:{}:{}:{}:{}", piece.id, piece.generation, source[0], source[1], source[2], source[3]);
        let rgba = match cache.0.get(&key) {
            Some(rgba) => rgba.clone(),
            None => {
                let Some(rgba) = (unsafe { image_rgba(graphics, piece.id, source) }) else { continue };
                let rgba = Arc::new(rgba);
                cache.0.insert(key.clone(), rgba.clone());
                rgba
            }
        };
        seen.insert(key.clone(), ());
        images.push(Image {
            key,
            rgba,
            width: source[2],
            height: source[3],
            col: piece.col,
            row: piece.row,
            offset_x: piece.offset[0],
            offset_y: piece.offset[1],
            draw_width: piece.draw[0],
            draw_height: piece.draw[1],
            layer: piece.layer,
            z: piece.z,
        });
    }
    // Pixels of images no longer shown are let go.
    cache.0.retain(|key, _| seen.contains_key(key));
    images.sort_by_key(|image| (image.layer, image.z));
    images
}

/// One placement as the render state lists it, before its pixels are read.
#[derive(Debug, Clone, PartialEq)]
struct Piece {
    id: u32,
    generation: u64,
    placement: u32,
    virtual_: bool,
    source: [u32; 4],
    col: i32,
    row: i32,
    offset: [i32; 2],
    draw: [u32; 2],
    layer: ImageLayer,
    z: i32,
}

/// Programs often show one picture as many placements, one per cell (yazi) or one per run of
/// placeholder cells. Drawn separately, scaled pieces show seams where their edges filter.
/// Placements of the same image at the same scale whose pieces put the whole image at the same
/// place on screen are one picture, and become one image covering all of them. `cell_px` is
/// the terminal's cell size in device pixels; without it nothing is merged.
fn merge_tiles(pieces: Vec<Piece>, [cell_w, cell_h]: [u32; 2]) -> Vec<Piece> {
    if cell_w == 0 || cell_h == 0 {
        return pieces;
    }
    let (cw, ch) = (cell_w as f64, cell_h as f64);
    let mut out: Vec<Piece> = Vec::new();
    // Columns of a picture share an exact horizontal mapping; rows may round their heights
    // differently, so vertically a picture is the pieces whose origins are within a cell.
    let mut columns: HashMap<(u32, u64, ImageLayer, i32, i64, i64), Vec<(f64, Piece)>> = HashMap::new();
    for piece in pieces {
        if piece.source[2] == 0 || piece.source[3] == 0 {
            out.push(piece);
            continue;
        }
        let sx = piece.draw[0] as f64 / piece.source[2] as f64;
        let sy = piece.draw[1] as f64 / piece.source[3] as f64;
        let origin_x = piece.col as f64 * cw + piece.offset[0] as f64 - piece.source[0] as f64 * sx;
        let origin_y = piece.row as f64 * ch + piece.offset[1] as f64 - piece.source[1] as f64 * sy;
        let key = (piece.id, piece.generation, piece.layer, piece.z, origin_x.round() as i64, (sx * 1000.0).round() as i64);
        columns.entry(key).or_default().push((origin_y, piece));
    }
    for (_, mut group) in columns {
        group.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut picture: Vec<Piece> = Vec::new();
        let mut last = f64::NEG_INFINITY;
        for (origin_y, piece) in group {
            if !picture.is_empty() && origin_y - last > ch {
                out.push(merged(std::mem::take(&mut picture), cw, ch));
            }
            last = origin_y;
            picture.push(piece);
        }
        if !picture.is_empty() {
            out.push(merged(picture, cw, ch));
        }
    }
    out
}

/// One image covering `pieces`: their source rectangles' union, drawn over their destinations'.
fn merged(pieces: Vec<Piece>, cw: f64, ch: f64) -> Piece {
    if pieces.len() == 1 {
        return pieces.into_iter().next().unwrap();
    }
    let first = pieces.iter().min_by_key(|p| (p.row, p.col)).unwrap().clone();
    let dest = |p: &Piece| {
        let x = p.col as f64 * cw + p.offset[0] as f64;
        let y = p.row as f64 * ch + p.offset[1] as f64;
        [x, y, x + p.draw[0] as f64, y + p.draw[1] as f64]
    };
    let fold = |pick: fn(&[f64; 4]) -> f64, min: bool| {
        let values = pieces.iter().map(|p| pick(&dest(p)));
        if min { values.fold(f64::INFINITY, f64::min) } else { values.fold(f64::NEG_INFINITY, f64::max) }
    };
    let (x0, y0, x1, y1) = (fold(|d| d[0], true), fold(|d| d[1], true), fold(|d| d[2], false), fold(|d| d[3], false));
    let left = pieces.iter().map(|p| p.source[0]).min().unwrap();
    let top = pieces.iter().map(|p| p.source[1]).min().unwrap();
    let right = pieces.iter().map(|p| p.source[0] + p.source[2]).max().unwrap();
    let bottom = pieces.iter().map(|p| p.source[1] + p.source[3]).max().unwrap();
    Piece {
        source: [left, top, right - left, bottom - top],
        offset: [(x0 - first.col as f64 * cw).round() as i32, (y0 - first.row as f64 * ch).round() as i32],
        draw: [(x1 - x0).round() as u32, (y1 - y0).round() as u32],
        ..first
    }
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

    fn piece(col: i32, row: i32, sx: u32, sy: u32) -> Piece {
        Piece {
            id: 7,
            generation: 1,
            placement: 3 + (col * 10 + row) as u32,
            virtual_: false,
            source: [sx, sy, 8, 16],
            col,
            row,
            offset: [0, 0],
            draw: [10, 20],
            layer: ImageLayer::BelowText,
            z: 0,
        }
    }

    #[test]
    fn pieces_of_one_picture_become_one_image() {
        // A 3x2 cell picture, one placement per cell, its top-left cell scrolled away.
        let pieces = vec![piece(11, 5, 8, 0), piece(12, 5, 16, 0), piece(10, 6, 0, 16), piece(11, 6, 8, 16), piece(12, 6, 16, 16)];
        let merged = merge_tiles(pieces, [10, 20]);
        assert_eq!(merged.len(), 1);
        let image = &merged[0];
        assert_eq!(image.source, [0, 0, 24, 32]);
        assert_eq!(image.draw, [30, 40]);
        assert_eq!((image.col, image.row), (11, 5));
        assert_eq!(image.offset, [-10, 0], "the image starts a cell to the left of its first visible piece");
    }

    #[test]
    fn rows_that_round_their_heights_differently_still_merge() {
        let mut short = piece(10, 6, 0, 16);
        short.source[3] = 15;
        let merged = merge_tiles(vec![piece(10, 5, 0, 0), short], [10, 20]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, [0, 0, 8, 31]);
        assert_eq!(merged[0].draw, [10, 40]);
    }

    #[test]
    fn the_same_image_shown_twice_stays_two_images() {
        let elsewhere = piece(40, 5, 0, 0);
        assert_eq!(merge_tiles(vec![piece(10, 5, 0, 0), elsewhere], [10, 20]).len(), 2);
        assert_eq!(merge_tiles(vec![piece(10, 5, 0, 0), piece(11, 5, 8, 0)], [0, 0]).len(), 2, "no cell size: no merging");
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
