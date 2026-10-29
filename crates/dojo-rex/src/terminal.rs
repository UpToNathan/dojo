//! A libghostty-vt terminal restored from a Rex stream's snapshot and kept current with its
//! VT bytes. It only mirrors the server's terminal: nothing it would answer is sent anywhere.
use std::ffi::c_void;
use std::ptr;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::ffi::*;
use crate::stream::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    fn from_ghostty(color: GhosttyColorRgb) -> Self {
        Self { r: color.r, g: color.g, b: color.b }
    }
    fn ghostty(self) -> GhosttyColorRgb {
        GhosttyColorRgb { r: self.r, g: self.g, b: self.b }
    }
    /// `RRGGBB` or `#RRGGBB`.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.len() != 6 {
            return None;
        }
        let value = u32::from_str_radix(hex, 16).ok()?;
        Some(Self { r: (value >> 16) as u8, g: (value >> 8) as u8, b: value as u8 })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cell {
    /// The grapheme in this cell; empty for a blank cell or the tail of a wide character.
    pub text: String,
    pub fg: Rgb,
    /// None means the terminal background.
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    HollowBlock,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub style: CursorStyle,
    pub color: Option<Rgb>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    pub foreground: Rgb,
    pub background: Rgb,
    pub cells: Vec<Vec<Cell>>,
    pub cursor: Option<Cursor>,
    /// Kitty graphics placements, in drawing order.
    pub images: Vec<crate::images::Image>,
}

impl Screen {
    pub fn row_text(&self, row: usize) -> String {
        let text: String = self.cells[row]
            .iter()
            .map(|cell| if cell.text.is_empty() { " " } else { cell.text.as_str() })
            .collect();
        text.trim_end().to_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

/// One key, as the front end saw it. `code` is the W3C `KeyboardEvent.code` name.
#[derive(Debug, Clone)]
pub struct KeyInput {
    pub code: String,
    pub text: Option<String>,
    pub unshifted: Option<char>,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
    pub action: KeyAction,
}

pub struct Terminal {
    raw: GhosttyTerminal,
    render: GhosttyRenderState,
    rows: GhosttyRenderStateRowIterator,
    row_cells: GhosttyRenderStateRowCells,
    encoder: GhosttyKeyEncoder,
    event: GhosttyKeyEvent,
    builtin_palette: Option<[GhosttyColorRgb; 256]>,
    /// The shape shown until a program picks one (Ghostty's `cursor-style`); None follows the
    /// terminal, which is a block.
    default_cursor: Option<CursorStyle>,
    cursor_requests: CursorRequests,
    image_iterator: GhosttyRenderStateImageIterator,
    pixels: crate::images::PixelCache,
    /// The render state's placements generation at the last screen read.
    placements: u64,
}

/// Watches the output for DECSCUSR (`CSI Ps SP q`): a program choosing a cursor shape wins over
/// the default until it resets it with `CSI 0 SP q`.
#[derive(Default)]
struct CursorRequests {
    /// The program has set a shape of its own.
    chosen: bool,
    /// Bytes of a possible DECSCUSR cut off at the end of the last write.
    tail: Vec<u8>,
}

impl CursorRequests {
    fn scan(&mut self, bytes: &[u8]) {
        let mut data = std::mem::take(&mut self.tail);
        data.extend_from_slice(bytes);
        let mut at = 0;
        while let Some(start) = data[at..].windows(2).position(|w| w == b"\x1b[").map(|i| at + i) {
            let params: Vec<u8> = data[start + 2..].iter().take_while(|b| b.is_ascii_digit()).copied().collect();
            let end = start + 2 + params.len();
            match data.get(end..end + 2) {
                Some(b" q") => {
                    self.chosen = !matches!(params.as_slice(), b"" | b"0");
                    at = end + 2;
                }
                Some(_) => at = start + 2,
                None if data.len() - start <= 8 => {
                    self.tail = data[start..].to_vec();
                    return;
                }
                None => at = start + 2,
            }
        }
        if data.last() == Some(&0x1b) {
            self.tail = vec![0x1b];
        }
    }
}

// The handles are owned by this value alone and libghostty keeps no thread-local state.
unsafe impl Send for Terminal {}

fn check(result: GhosttyResult, what: &str) -> Result<()> {
    if result == GHOSTTY_SUCCESS {
        Ok(())
    } else {
        bail!("{what} failed ({result})")
    }
}

struct Source<'a> {
    pull: &'a mut dyn FnMut() -> Result<Option<Vec<u8>>>,
    supplied: Vec<u8>,
    position: usize,
    error: Option<anyhow::Error>,
}

unsafe extern "C" fn read_source(userdata: *mut c_void, buffer: *mut u8, capacity: usize, out_read: *mut usize) -> bool {
    let source = unsafe { &mut *(userdata as *mut Source) };
    while source.position == source.supplied.len() {
        match (source.pull)() {
            Ok(Some(chunk)) => source.supplied.extend_from_slice(&chunk),
            Ok(None) => {
                unsafe { *out_read = 0 };
                return true;
            }
            Err(error) => {
                source.error = Some(error);
                return false;
            }
        }
    }
    let count = capacity.min(source.supplied.len() - source.position);
    unsafe { ptr::copy_nonoverlapping(source.supplied.as_ptr().add(source.position), buffer, count) };
    source.position += count;
    unsafe { *out_read = count };
    true
}

impl Terminal {
    /// Restores the snapshot at the start of a stream. `pull` returns stream chunks, or None at
    /// the end. Returns the terminal and the stream bytes after the snapshot.
    pub fn from_snapshot(pull: &mut dyn FnMut() -> Result<Option<Vec<u8>>>) -> Result<(Self, Vec<u8>)> {
        let mut source = Source { pull, supplied: Vec::new(), position: 0, error: None };
        let reader = GhosttyReader { read: Some(read_source), userdata: &mut source as *mut Source as *mut c_void };
        let mut decoder: GhosttySnapshotDecoder = ptr::null_mut();
        check(unsafe { ghostty_snapshot_decoder_new(ptr::null(), &mut decoder, reader) }, "creating the snapshot decoder")?;
        let mut raw: GhosttyTerminal = ptr::null_mut();
        let mut result = unsafe { ghostty_snapshot_decoder_ready(decoder, &mut raw) };
        while result == GHOSTTY_SUCCESS {
            result = unsafe { ghostty_snapshot_decoder_next(decoder) };
        }
        if result == GHOSTTY_NO_VALUE {
            result = GHOSTTY_SUCCESS;
        }
        let mut offset: usize = 0;
        let offset_result = unsafe {
            ghostty_snapshot_decoder_get(
                decoder,
                GHOSTTY_SNAPSHOT_DECODER_DATA_SOURCE_OFFSET,
                &mut offset as *mut usize as *mut c_void,
            )
        };
        unsafe { ghostty_snapshot_decoder_free(decoder) };
        if let Some(error) = source.error.take() {
            if !raw.is_null() {
                unsafe { ghostty_terminal_free(raw) };
            }
            return Err(error.context("reading the snapshot"));
        }
        check(result, "decoding the snapshot")?;
        check(offset_result, "locating the end of the snapshot")?;
        let rest = source.supplied[offset.min(source.supplied.len())..].to_vec();
        Ok((Self::wrap(raw)?, rest))
    }

    /// A blank terminal, for tests and for a stream that starts without a snapshot.
    pub fn new(cols: u16, rows: u16) -> Result<Self> {
        let mut raw: GhosttyTerminal = ptr::null_mut();
        check(unsafe { ghostty_terminal_new(ptr::null(), &mut raw, cols, rows) }, "creating a terminal")?;
        Self::wrap(raw)
    }

    fn wrap(raw: GhosttyTerminal) -> Result<Self> {
        let mut this = Self {
            raw,
            render: ptr::null_mut(),
            rows: ptr::null_mut(),
            row_cells: ptr::null_mut(),
            encoder: ptr::null_mut(),
            event: ptr::null_mut(),
            builtin_palette: None,
            default_cursor: None,
            cursor_requests: CursorRequests::default(),
            image_iterator: ptr::null_mut(),
            pixels: Default::default(),
            placements: u64::MAX,
        };
        crate::images::install_png_decoder();
        crate::images::enable(raw);
        unsafe {
            check(ghostty_render_state_new(ptr::null(), &mut this.render), "creating render state")?;
            check(ghostty_render_state_row_iterator_new(ptr::null(), &mut this.rows), "creating a row iterator")?;
            check(ghostty_render_state_row_cells_new(ptr::null(), &mut this.row_cells), "creating row cells")?;
            check(ghostty_key_encoder_new(ptr::null(), &mut this.encoder), "creating a key encoder")?;
            check(ghostty_key_event_new(ptr::null(), &mut this.event), "creating a key event")?;
            check(ghostty_render_state_image_iterator_new(ptr::null(), &mut this.image_iterator), "creating an image iterator")?;
        }
        Ok(this)
    }

    /// The cursor shape to show while no program has chosen one.
    pub fn set_default_cursor(&mut self, style: Option<CursorStyle>) {
        self.default_cursor = style;
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.cursor_requests.scan(bytes);
        unsafe { ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) -> Result<()> {
        check(
            unsafe { ghostty_terminal_resize(self.raw, cols.max(1), rows.max(1), cell_width_px, cell_height_px) },
            "resizing the terminal",
        )
    }

    /// Applies a Rex message that changes the terminal: `resize` and `theme`.
    pub fn apply(&mut self, message: &Message) -> Result<()> {
        match message {
            Message::Resize { cols, rows, cell_width_px, cell_height_px } => {
                self.resize(*cols, *rows, *cell_width_px, *cell_height_px)
            }
            Message::Theme(theme) => self.apply_theme(theme),
            _ => Ok(()),
        }
    }

    fn set_color(&mut self, option: GhosttyTerminalOption, color: Option<Rgb>) -> Result<()> {
        let value = color.map(Rgb::ghostty);
        let pointer = value.as_ref().map_or(ptr::null(), |color| color as *const GhosttyColorRgb as *const c_void);
        check(unsafe { ghostty_terminal_set(self.raw, option, pointer) }, "setting a colour")
    }

    /// A `theme` message replaces the default colours; anything it leaves out is built in.
    pub fn apply_theme(&mut self, theme: &Value) -> Result<()> {
        let color = |key: &str| theme[key].as_str().and_then(Rgb::parse);
        self.set_color(GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, color("foreground"))?;
        self.set_color(GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, color("background"))?;
        self.set_color(GHOSTTY_TERMINAL_OPT_COLOR_CURSOR, color("cursor"))?;
        let builtin = match self.builtin_palette {
            Some(palette) => palette,
            None => {
                let palette = self.palette()?;
                self.builtin_palette = Some(palette);
                palette
            }
        };
        let mut palette = builtin;
        for entry in theme["palette"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let Some((index, color)) = entry.split_once('=') else { continue };
            if let (Ok(index), Some(color)) = (index.trim().parse::<u8>(), Rgb::parse(color.trim())) {
                palette[index as usize] = color.ghostty();
            }
        }
        check(
            unsafe { ghostty_terminal_set(self.raw, GHOSTTY_TERMINAL_OPT_COLOR_PALETTE, palette.as_ptr() as *const c_void) },
            "setting the palette",
        )
    }

    fn palette(&mut self) -> Result<[GhosttyColorRgb; 256]> {
        check(unsafe { ghostty_render_state_update(self.render, self.raw) }, "updating render state")?;
        let mut palette = [GhosttyColorRgb::default(); 256];
        check(
            unsafe {
                ghostty_render_state_get(self.render, GHOSTTY_RENDER_STATE_DATA_COLOR_PALETTE, palette.as_mut_ptr() as *mut c_void)
            },
            "reading the palette",
        )?;
        Ok(palette)
    }

    fn render_get<T: Default>(&self, key: GhosttyRenderStateData) -> Option<T> {
        let mut value = T::default();
        let result = unsafe { ghostty_render_state_get(self.render, key, &mut value as *mut T as *mut c_void) };
        (result == GHOSTTY_SUCCESS).then_some(value)
    }

    /// The visible screen. `None` when nothing changed since the last call.
    pub fn screen(&mut self, force: bool) -> Result<Option<Screen>> {
        check(unsafe { ghostty_render_state_update(self.render, self.raw) }, "updating render state")?;
        let dirty: GhosttyRenderStateDirty = self.render_get(GHOSTTY_RENDER_STATE_DATA_DIRTY).unwrap_or_default();
        let placements: u64 = self.render_get(GHOSTTY_RENDER_STATE_DATA_IMAGE_PLACEMENTS_GENERATION).unwrap_or(0);
        if !force && dirty == GHOSTTY_RENDER_STATE_DIRTY_FALSE && placements == self.placements {
            return Ok(None);
        }
        self.placements = placements;
        let foreground = Rgb::from_ghostty(self.render_get(GHOSTTY_RENDER_STATE_DATA_COLOR_FOREGROUND).unwrap_or_default());
        let background = Rgb::from_ghostty(self.render_get(GHOSTTY_RENDER_STATE_DATA_COLOR_BACKGROUND).unwrap_or_default());
        let mut screen = Screen {
            cols: self.render_get::<u16>(GHOSTTY_RENDER_STATE_DATA_COLS).unwrap_or(0),
            rows: self.render_get::<u16>(GHOSTTY_RENDER_STATE_DATA_ROWS).unwrap_or(0),
            foreground,
            background,
            cells: Vec::new(),
            cursor: None,
            images: unsafe { crate::images::collect(self.raw, self.render, &mut self.image_iterator, &mut self.pixels) },
        };

        let visible = self.render_get::<bool>(GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE).unwrap_or(false);
        let in_viewport = self.render_get::<bool>(GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE).unwrap_or(false);
        if visible && in_viewport {
            let style: GhosttyRenderStateCursorVisualStyle =
                self.render_get(GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE).unwrap_or_default();
            let color = self
                .render_get::<bool>(GHOSTTY_RENDER_STATE_DATA_COLOR_CURSOR_HAS_VALUE)
                .unwrap_or(false)
                .then(|| self.render_get::<GhosttyColorRgb>(GHOSTTY_RENDER_STATE_DATA_COLOR_CURSOR))
                .flatten()
                .map(Rgb::from_ghostty);
            screen.cursor = Some(Cursor {
                x: self.render_get(GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X).unwrap_or(0),
                y: self.render_get(GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y).unwrap_or(0),
                style: match (style, self.default_cursor) {
                    (_, Some(default)) if !self.cursor_requests.chosen => default,
                    (GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR, _) => CursorStyle::Bar,
                    (GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE, _) => CursorStyle::Underline,
                    (GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW, _) => CursorStyle::HollowBlock,
                    _ => CursorStyle::Block,
                },
                color,
            });
        }

        unsafe {
            check(
                ghostty_render_state_get(self.render, GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR, &mut self.rows as *mut _ as *mut c_void),
                "reading rows",
            )?;
            while ghostty_render_state_row_iterator_next(self.rows) {
                check(
                    ghostty_render_state_row_get(self.rows, GHOSTTY_RENDER_STATE_ROW_DATA_CELLS, &mut self.row_cells as *mut _ as *mut c_void),
                    "reading cells",
                )?;
                let mut row = Vec::with_capacity(screen.cols as usize);
                while ghostty_render_state_row_cells_next(self.row_cells) {
                    row.push(self.cell(foreground, background));
                }
                screen.cells.push(row);
                let clean = false;
                ghostty_render_state_row_set(self.rows, GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY, &clean as *const bool as *const c_void);
            }
            let clean: GhosttyRenderStateDirty = GHOSTTY_RENDER_STATE_DIRTY_FALSE;
            ghostty_render_state_set(self.render, GHOSTTY_RENDER_STATE_OPTION_DIRTY, &clean as *const _ as *const c_void);
        }
        Ok(Some(screen))
    }

    unsafe fn cell(&self, foreground: Rgb, background: Rgb) -> Cell {
        let cells = self.row_cells;
        let get = |key, out: *mut c_void| unsafe { ghostty_render_state_row_cells_get(cells, key, out) };
        let mut cell = Cell { fg: foreground, ..Default::default() };
        let mut len: u32 = 0;
        get(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN, &mut len as *mut u32 as *mut c_void);
        if len > 0 {
            let mut codepoints = vec![0u32; len as usize];
            get(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF, codepoints.as_mut_ptr() as *mut c_void);
            cell.text = codepoints.into_iter().filter_map(char::from_u32).collect();
        }
        let mut style: GhosttyStyle = Default::default();
        style.size = std::mem::size_of::<GhosttyStyle>();
        get(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE, &mut style as *mut GhosttyStyle as *mut c_void);
        let mut rgb = GhosttyColorRgb::default();
        if get(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR, &mut rgb as *mut _ as *mut c_void) == GHOSTTY_SUCCESS {
            cell.fg = Rgb::from_ghostty(rgb);
        }
        if get(GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR, &mut rgb as *mut _ as *mut c_void) == GHOSTTY_SUCCESS {
            cell.bg = Some(Rgb::from_ghostty(rgb));
        }
        if style.inverse {
            let bg = cell.bg.unwrap_or(background);
            cell.bg = Some(cell.fg);
            cell.fg = bg;
        }
        cell.bold = style.bold;
        cell.italic = style.italic;
        cell.faint = style.faint;
        cell.underline = style.underline != 0;
        cell.strikethrough = style.strikethrough;
        if style.invisible {
            cell.text.clear();
        }
        cell
    }

    /// The bytes a key sends to the program, following the terminal's current modes (cursor
    /// keys, keypad, the kitty keyboard protocol). Empty when the key sends nothing.
    pub fn encode_key(&mut self, key: &KeyInput) -> Vec<u8> {
        let Some(physical) = crate::keys::key_from_code(&key.code) else {
            return key.text.clone().unwrap_or_default().into_bytes();
        };
        let mut mods: GhosttyMods = 0;
        for (on, bit) in [(key.shift, GHOSTTY_MODS_SHIFT), (key.ctrl, GHOSTTY_MODS_CTRL), (key.alt, GHOSTTY_MODS_ALT), (key.logo, GHOSTTY_MODS_SUPER)] {
            if on {
                mods |= bit as GhosttyMods;
            }
        }
        let text = key.text.clone().unwrap_or_default();
        let mut out = [0u8; 128];
        let mut written = 0usize;
        unsafe {
            ghostty_key_encoder_setopt_from_terminal(self.encoder, self.raw);
            ghostty_key_event_set_action(
                self.event,
                match key.action {
                    KeyAction::Press => GHOSTTY_KEY_ACTION_PRESS,
                    KeyAction::Repeat => GHOSTTY_KEY_ACTION_REPEAT,
                    KeyAction::Release => GHOSTTY_KEY_ACTION_RELEASE,
                },
            );
            ghostty_key_event_set_key(self.event, physical);
            ghostty_key_event_set_mods(self.event, mods);
            ghostty_key_event_set_consumed_mods(self.event, if key.shift && !text.is_empty() { GHOSTTY_MODS_SHIFT as GhosttyMods } else { 0 });
            ghostty_key_event_set_utf8(self.event, text.as_ptr() as *const std::ffi::c_char, text.len());
            ghostty_key_event_set_unshifted_codepoint(self.event, key.unshifted.map_or(0, u32::from));
            if ghostty_key_encoder_encode(self.encoder, self.event, out.as_mut_ptr() as *mut std::ffi::c_char, out.len(), &mut written)
                != GHOSTTY_SUCCESS
            {
                return Vec::new();
            }
        }
        out[..written].to_vec()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe {
            ghostty_render_state_image_iterator_free(self.image_iterator);
            ghostty_key_event_free(self.event);
            ghostty_key_encoder_free(self.encoder);
            ghostty_render_state_row_cells_free(self.row_cells);
            ghostty_render_state_row_iterator_free(self.rows);
            ghostty_render_state_free(self.render);
            ghostty_terminal_free(self.raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &str, text: Option<&str>) -> KeyInput {
        KeyInput {
            code: code.into(),
            text: text.map(Into::into),
            unshifted: text.and_then(|text| text.chars().next()),
            shift: false,
            ctrl: false,
            alt: false,
            logo: false,
            action: KeyAction::Press,
        }
    }

    #[test]
    fn writes_show_up_on_the_screen() {
        let mut terminal = Terminal::new(20, 3).unwrap();
        terminal.write(b"hello\r\n\x1b[1;31mred\x1b[0m");
        let screen = terminal.screen(true).unwrap().unwrap();
        assert_eq!((screen.cols, screen.rows), (20, 3));
        assert_eq!(screen.row_text(0), "hello");
        assert_eq!(screen.row_text(1), "red");
        assert!(screen.cells[1][0].bold);
        assert_ne!(screen.cells[1][0].fg, screen.foreground);
        assert_eq!(screen.cursor.map(|cursor| (cursor.x, cursor.y)), Some((3, 1)));
        assert!(terminal.screen(false).unwrap().is_none(), "an unchanged screen is clean");
    }

    #[test]
    fn a_snapshot_restores_and_leaves_the_rest_of_the_stream() {
        let mut source = Terminal::new(30, 4).unwrap();
        source.write(b"before the snapshot");
        let mut bytes: *mut u8 = ptr::null_mut();
        let mut len = 0usize;
        check(unsafe { ghostty_snapshot_encode_alloc(source.raw, ptr::null(), &mut bytes, &mut len) }, "encode").unwrap();
        let mut stream = unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec();
        unsafe { ghostty_free(ptr::null(), bytes, len) };
        assert!(stream.starts_with(b"GHOSTSNP"));
        stream.extend_from_slice(b"\r\nafter");
        let mut chunks = stream.chunks(7).map(<[u8]>::to_vec).collect::<Vec<_>>().into_iter();
        let (mut terminal, mut rest) = Terminal::from_snapshot(&mut || Ok(chunks.next())).unwrap();
        // Bytes the decoder read past the snapshot come back; the rest stay in the source.
        rest.extend(chunks.flatten());
        assert_eq!(rest, b"\r\nafter");
        terminal.write(&rest);
        let screen = terminal.screen(true).unwrap().unwrap();
        assert_eq!(screen.row_text(0), "before the snapshot");
        assert_eq!(screen.row_text(1), "after");
    }

    #[test]
    fn keys_follow_cursor_key_mode() {
        let mut terminal = Terminal::new(10, 2).unwrap();
        assert_eq!(terminal.encode_key(&key("KeyA", Some("a"))), b"a");
        assert_eq!(terminal.encode_key(&key("ArrowUp", None)), b"\x1b[A");
        terminal.write(b"\x1b[?1h");
        assert_eq!(terminal.encode_key(&key("ArrowUp", None)), b"\x1bOA");
        let mut ctrl_c = key("KeyC", Some("c"));
        ctrl_c.ctrl = true;
        assert_eq!(terminal.encode_key(&ctrl_c), b"\x03");
        assert_eq!(terminal.encode_key(&key("Enter", None)), b"\r");
    }

    #[test]
    fn the_default_cursor_holds_until_a_program_chooses_one() {
        let mut terminal = Terminal::new(10, 2).unwrap();
        terminal.set_default_cursor(Some(CursorStyle::Bar));
        let style = |terminal: &mut Terminal| terminal.screen(true).unwrap().unwrap().cursor.unwrap().style;
        assert_eq!(style(&mut terminal), CursorStyle::Bar);
        terminal.write(b"\x1b[2");
        terminal.write(b" q");
        assert_eq!(style(&mut terminal), CursorStyle::Block, "a steady block chosen across two writes");
        terminal.write(b"\x1b[0 q");
        assert_eq!(style(&mut terminal), CursorStyle::Bar, "a reset returns to the default");
        terminal.write(b"\x1b[4 q");
        assert_eq!(style(&mut terminal), CursorStyle::Underline);
    }

    #[test]
    fn a_theme_sets_default_colours() {
        let mut terminal = Terminal::new(10, 2).unwrap();
        terminal
            .apply_theme(&serde_json::json!({"type": "theme", "foreground": "#102030", "background": "405060", "palette": ["1=ff0000"]}))
            .unwrap();
        terminal.write(b"\x1b[31mx");
        let screen = terminal.screen(true).unwrap().unwrap();
        assert_eq!(screen.foreground, Rgb { r: 0x10, g: 0x20, b: 0x30 });
        assert_eq!(screen.background, Rgb { r: 0x40, g: 0x50, b: 0x60 });
        assert_eq!(screen.cells[0][0].fg, Rgb { r: 0xff, g: 0, b: 0 });
    }
}
