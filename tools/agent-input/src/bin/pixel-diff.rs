use std::{fs::File, io::BufReader};
fn read(path: &str) -> Result<(u32, u32, usize, Vec<u8>), Box<dyn std::error::Error>> {
    let decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    let mut reader = decoder.read_info()?;
    let mut data = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let info = reader.next_frame(&mut data)?;
    data.truncate(info.buffer_size());
    if info.bit_depth != png::BitDepth::Eight {
        return Err("Expected 8-bit PNG".into());
    }
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => return Err("Expected RGB or RGBA screenshot".into()),
    };
    Ok((info.width, info.height, channels, data))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 6 {
        return Err("Usage: pixel-diff BEFORE AFTER X Y RADIUS".into());
    }
    let a = read(&args[1])?;
    let b = read(&args[2])?;
    if (a.0, a.1, a.2) != (b.0, b.1, b.2) {
        return Err("Screenshot formats differ".into());
    }
    let x: i32 = args[3].parse()?;
    let y: i32 = args[4].parse()?;
    let radius: i32 = args[5].parse()?;
    let mut changed = 0;
    for row in (y - radius).max(0)..(y + radius).min(a.1 as i32) {
        for column in (x - radius).max(0)..(x + radius).min(a.0 as i32) {
            let offset = (row as usize * a.0 as usize + column as usize) * a.2;
            if a.3[offset..offset + a.2] != b.3[offset..offset + a.2] {
                changed += 1;
            }
        }
    }
    println!("{{\"changed_pixels\":{changed}}}");
    Ok(())
}
