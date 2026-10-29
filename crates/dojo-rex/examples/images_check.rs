//! Shows an image with `kitty +kitten icat` in each transfer mode and counts the placements the
//! mirror sees. `cargo run --example images_check -- IMAGE.png`
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let image = std::env::args().nth(1).expect("an image path");
    let session = dojo_rex::Session::open(
        dojo_rex::SessionOptions {
            socket: dojo_rex::default_socket(),
            target: dojo_rex::Target::Label("dojo-images-check".into()),
            command: vec![],
            pane_command: vec![],
            cwd: None,
            cols: 100,
            rows: 40,
            client_name: "images-check".into(),
        },
        || {},
    )?;
    std::thread::sleep(Duration::from_millis(600));
    let block = session.view().focused_block().unwrap();
    session.resize(&block, 100, 40, 10, 20)?;
    std::thread::sleep(Duration::from_millis(300));
    for mode in ["stream", "file", "memory"] {
        session.write(&block, format!("clear; kitty +kitten icat --clear; kitty +kitten icat --transfer-mode={mode} --place 20x10@0x2 {image}\r").as_bytes())?;
        std::thread::sleep(Duration::from_millis(2500));
        let shared = session.pane(&block).unwrap();
        let mut guard = shared.terminal.lock().unwrap();
        let screen = guard.as_mut().unwrap().screen(true)?.unwrap();
        let summary: Vec<String> = screen.images.iter().map(|i| format!("{}x{} at {},{} drawn {}x{}", i.width, i.height, i.col, i.row, i.draw_width, i.draw_height)).collect();
        println!("{mode}: {} placement(s) {:?}", screen.images.len(), summary);
    }
    let control = dojo_rex::Control::connect(&dojo_rex::default_socket(), "images-check-done")?;
    control.call("session.attach", serde_json::json!({"session_id": session.id()}))?;
    control.call("session.destroy", serde_json::json!({"session_id": session.id()}))?;
    Ok(())
}
