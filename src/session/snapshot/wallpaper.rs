//! The wallpaper piece of a snapshot: the pixels already decoded, so the greeter, which sees
//! every user's password, paints them without running an image decoder over a user's file.
//!
//! The snapshot file is `lock-wallpaper.raw`: the magic `DJWP`, the width and the height as
//! little-endian `u32`, then exactly `width * height * 4` bytes of BGRA, the channel order
//! GPUI's `RenderImage` takes. Only the collector, after it has dropped to the user, turns
//! the user's `lock-wallpaper.png` into this ([`Wallpaper::from_png`]).

use std::fmt;
use std::io::Cursor;

use image::{ImageFormat, ImageReader, Limits};

use super::{WALLPAPER_DECODE_ALLOC, WALLPAPER_MAX_HEIGHT, WALLPAPER_MAX_WIDTH};

const MAGIC: &[u8; 4] = b"DJWP";
pub(super) const HEADER_LEN: usize = 12;

/// Decoded pixels of a sane size.
#[derive(Clone, PartialEq, Eq)]
pub struct Wallpaper {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

impl fmt::Debug for Wallpaper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Wallpaper({}x{})", self.width, self.height)
    }
}

impl Wallpaper {
    /// The snapshot file: checks the header and the exact length, nothing else.
    pub fn parse(mut bytes: Vec<u8>) -> anyhow::Result<Wallpaper> {
        use anyhow::Context as _;
        let header = bytes.get(..HEADER_LEN).context("shorter than a wallpaper header")?;
        anyhow::ensure!(&header[..4] == MAGIC, "not a raw wallpaper");
        let field = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
        let (width, height) = (field(4), field(8));
        bytes.drain(..HEADER_LEN);
        Wallpaper::new(width, height, bytes)
    }

    /// The user's `lock-wallpaper.png`. The limits are set before decoding starts, so an
    /// oversized or bomb-shaped file is refused from its header without allocating.
    pub fn from_png(png: &[u8]) -> anyhow::Result<Wallpaper> {
        let mut reader = ImageReader::with_format(Cursor::new(png), ImageFormat::Png);
        let mut limits = Limits::no_limits();
        limits.max_image_width = Some(WALLPAPER_MAX_WIDTH);
        limits.max_image_height = Some(WALLPAPER_MAX_HEIGHT);
        limits.max_alloc = Some(WALLPAPER_DECODE_ALLOC);
        reader.limits(limits);
        let mut pixels = reader.decode()?.into_rgba8();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let (width, height) = pixels.dimensions();
        Wallpaper::new(width, height, pixels.into_raw())
    }

    fn new(width: u32, height: u32, bgra: Vec<u8>) -> anyhow::Result<Wallpaper> {
        anyhow::ensure!(
            (1..=WALLPAPER_MAX_WIDTH).contains(&width) && (1..=WALLPAPER_MAX_HEIGHT).contains(&height),
            "wallpaper is {width}x{height}, outside 1x1 to {WALLPAPER_MAX_WIDTH}x{WALLPAPER_MAX_HEIGHT}"
        );
        let expected = width as usize * height as usize * 4;
        anyhow::ensure!(
            bgra.len() == expected,
            "a {width}x{height} wallpaper has {expected} bytes of pixels, not {}",
            bgra.len()
        );
        Ok(Wallpaper {
            width,
            height,
            bgra,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.bgra.len());
        bytes.extend(MAGIC);
        bytes.extend(self.width.to_le_bytes());
        bytes.extend(self.height.to_le_bytes());
        bytes.extend(&self.bgra);
        bytes
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// `width * height` pixels, four bytes each: blue, green, red, alpha.
    pub fn bgra(&self) -> &[u8] {
        &self.bgra
    }
}

#[cfg(test)]
mod tests {
    use image::{ImageError, Rgba, RgbaImage};

    use super::*;

    fn png(image: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn raw(width: u32, height: u32, pixels: usize) -> Vec<u8> {
        let mut bytes = b"DJWP".to_vec();
        bytes.extend(width.to_le_bytes());
        bytes.extend(height.to_le_bytes());
        bytes.resize(HEADER_LEN + pixels, 7);
        bytes
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
            }
        }
        !crc
    }

    /// A few dozen bytes of PNG that declare `width` x `height` pixels and carry none.
    fn bomb(width: u32, height: u32) -> Vec<u8> {
        let chunk = |kind: &[u8; 4], data: &[u8]| {
            let mut out = (data.len() as u32).to_be_bytes().to_vec();
            let body = [kind.as_slice(), data].concat();
            out.extend(&body);
            out.extend(crc32(&body).to_be_bytes());
            out
        };
        let mut header = width.to_be_bytes().to_vec();
        header.extend(height.to_be_bytes());
        header.extend([8, 6, 0, 0, 0]);
        [
            b"\x89PNG\r\n\x1a\n".to_vec(),
            chunk(b"IHDR", &header),
            chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]),
            chunk(b"IEND", &[]),
        ]
        .concat()
    }

    #[test]
    fn the_raw_file_round_trips() {
        let bytes = [raw(3, 2, 24)].concat();
        let wallpaper = Wallpaper::parse(bytes.clone()).unwrap();
        assert_eq!((wallpaper.width(), wallpaper.height()), (3, 2));
        assert_eq!(wallpaper.bgra(), &[7; 24]);
        assert_eq!(wallpaper.encode(), bytes);
    }

    #[test]
    fn the_raw_file_must_be_exactly_a_header_and_its_pixels() {
        assert!(Wallpaper::parse(raw(WALLPAPER_MAX_WIDTH, WALLPAPER_MAX_HEIGHT, 4 << 20)).is_ok());
        let mut wrong_magic = raw(2, 2, 16);
        wrong_magic[3] = b'Q';
        let mut png_instead = png(&RgbaImage::new(2, 2));
        png_instead.resize(HEADER_LEN + 16, 0);
        for (name, bytes) in [
            ("empty", Vec::new()),
            ("short header", b"DJWP\x02\0\0\0".to_vec()),
            ("magic", wrong_magic),
            ("a png", png_instead),
            ("one byte short", raw(2, 2, 15)),
            ("one byte over", raw(2, 2, 17)),
            ("a pixel over", raw(2, 2, 20)),
            ("no pixels", raw(2, 2, 0)),
            ("zero width", raw(0, 2, 0)),
            ("zero height", raw(2, 0, 0)),
            ("too wide", raw(WALLPAPER_MAX_WIDTH + 1, 1, (WALLPAPER_MAX_WIDTH as usize + 1) * 4)),
            ("too tall", raw(1, WALLPAPER_MAX_HEIGHT + 1, (WALLPAPER_MAX_HEIGHT as usize + 1) * 4)),
            ("huge declared size", raw(u32::MAX, u32::MAX, 16)),
        ] {
            assert!(Wallpaper::parse(bytes).is_err(), "{name}");
        }
    }

    #[test]
    fn a_png_becomes_bgra_pixels() {
        let mut image = RgbaImage::new(2, 1);
        image.put_pixel(0, 0, Rgba([10, 20, 30, 255]));
        image.put_pixel(1, 0, Rgba([200, 100, 50, 128]));
        let wallpaper = Wallpaper::from_png(&png(&image)).unwrap();
        assert_eq!((wallpaper.width(), wallpaper.height()), (2, 1));
        assert_eq!(wallpaper.bgra(), [30, 20, 10, 255, 50, 100, 200, 128]);

        let gray = image::GrayImage::from_pixel(1, 1, image::Luma([90]));
        let mut bytes = Vec::new();
        gray.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png).unwrap();
        assert_eq!(Wallpaper::from_png(&bytes).unwrap().bgra(), [90, 90, 90, 255]);
    }

    #[test]
    fn the_largest_accepted_png_decodes() {
        let image = RgbaImage::from_pixel(WALLPAPER_MAX_WIDTH, WALLPAPER_MAX_HEIGHT, Rgba([1, 2, 3, 4]));
        let wallpaper = Wallpaper::from_png(&png(&image)).unwrap();
        assert_eq!(wallpaper.encode().len() as u64, crate::session::snapshot::WALLPAPER_CAP);
    }

    #[test]
    fn an_oversized_png_is_refused_by_the_limits() {
        for (width, height) in [
            (WALLPAPER_MAX_WIDTH + 1, 8),
            (8, WALLPAPER_MAX_HEIGHT + 1),
        ] {
            let error = Wallpaper::from_png(&png(&RgbaImage::new(width, height))).unwrap_err();
            assert!(
                matches!(error.downcast_ref::<ImageError>(), Some(ImageError::Limits(_))),
                "{width}x{height}: {error:#}"
            );
        }
    }

    #[test]
    fn a_decompression_bomb_is_refused_before_anything_is_allocated() {
        for (width, height) in [(100_000, 100_000), (u32::MAX >> 1, 1), (1, u32::MAX >> 1)] {
            let bytes = bomb(width, height);
            assert!(bytes.len() < 100);
            let error = Wallpaper::from_png(&bytes).unwrap_err();
            assert!(
                matches!(error.downcast_ref::<ImageError>(), Some(ImageError::Limits(_))),
                "{width}x{height}: {error:#}"
            );
        }
        // The same file with sane dimensions is not rejected for its size, only for its data.
        let error = Wallpaper::from_png(&bomb(8, 8)).unwrap_err();
        assert!(!matches!(error.downcast_ref::<ImageError>(), Some(ImageError::Limits(_))));
    }

    #[test]
    fn only_png_is_accepted() {
        let mut jpeg = Vec::new();
        image::RgbImage::new(8, 8)
            .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();
        for bytes in [jpeg, b"GIF89a".to_vec(), b"\x89PNG\r\n\x1a\n".to_vec(), Vec::new()] {
            assert!(Wallpaper::from_png(&bytes).is_err());
        }
    }

    fn sources(path: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        if path.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                sources(&entry.unwrap().path(), found);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path.to_owned());
        }
    }

    #[test]
    fn no_decoder_creeps_into_the_greeters_snapshot_path() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        for path in ["bin/dojo-greeter", "bin/dojo-greeter.rs", "dm/edge.rs"] {
            sources(&root.join(path), &mut files);
        }
        assert!(files.len() >= 3, "{files:?}");
        let decoders = [
            ["load_from", "_memory"].concat(),
            ["image::", "open"].concat(),
            ["image::", "load"].concat(),
            ["Image", "Reader"].concat(),
            ["image::io", "::Reader"].concat(),
        ];
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for decoder in &decoders {
                assert!(!text.contains(decoder.as_str()), "{} uses {decoder}", file.display());
            }
        }
    }
}
