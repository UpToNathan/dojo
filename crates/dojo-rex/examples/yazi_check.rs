//! Runs yazi on a directory in a Rex session and reports what the mirror holds.
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let session = dojo_rex::Session::open(
        dojo_rex::SessionOptions {
            socket: dojo_rex::default_socket(),
            target: dojo_rex::Target::Label("dojo-yazi-check2".into()),
            command: vec!["env".into(), "TERM=xterm-256color".into(), "yazi".into(), dir],
            pane_command: vec![],
            cwd: None,
            cols: 120,
            rows: 40,
            client_name: "yazi-check".into(),
        },
        || {},
    )?;
    std::thread::sleep(Duration::from_millis(800));
    let block = session.view().focused_block().unwrap();
    session.resize(&block, 120, 40, 10, 20)?;
    std::thread::sleep(Duration::from_millis(4000));
    let shared = session.pane(&block).unwrap();
    let mut guard = shared.terminal.lock().unwrap();
    let screen = guard.as_mut().unwrap().screen(true)?.unwrap();
    let placeholders = screen.cells.iter().flatten().filter(|c| c.text.starts_with('\u{10EEEE}')).count();
    println!("images: {}, placeholder cells: {placeholders}", screen.images.len());
    for image in screen.images.iter().take(3) {
        println!("  {}x{} at {},{} drawn {}x{} {:?}", image.width, image.height, image.col, image.row, image.draw_width, image.draw_height, image.layer);
        let cell = &screen.cells[image.row.max(0) as usize][image.col.max(0) as usize];
        println!("    cell text {:?} fg {:?} bg {:?}", cell.text.chars().map(|c| c as u32).collect::<Vec<_>>(), cell.fg, cell.bg);
    }
    for row in 0..6 {
        println!("|{}", screen.row_text(row).chars().take(110).collect::<String>());
    }
    drop(guard);
    let control = dojo_rex::Control::connect(&dojo_rex::default_socket(), "yazi-check-done")?;
    control.call("session.attach", serde_json::json!({"session_id": session.id()}))?;
    control.call("session.destroy", serde_json::json!({"session_id": session.id()}))?;
    Ok(())
}
