//! Attaches to a Rex session (creating it), types a command, and prints the screen.
//! `cargo run --example dump -- LABEL 'echo hi'`
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let label = args.next().unwrap_or_else(|| "dojo-rex-dump".into());
    let input = args.next();
    let attachment = dojo_rex::Attachment::open(
        dojo_rex::Options {
            socket: dojo_rex::default_socket(),
            label,
            command: Vec::new(),
            cwd: None,
            cols: 80,
            rows: 12,
            client_name: "dojo-rex-dump".into(),
        },
        || {},
    )?;
    std::thread::sleep(Duration::from_millis(500));
    if let Some(input) = input {
        attachment.write(format!("{input}\r").as_bytes())?;
        std::thread::sleep(Duration::from_millis(500));
    }
    if let Some(error) = attachment.shared.error.lock().unwrap().as_ref() {
        anyhow::bail!("stream failed: {error}");
    }
    let mut terminal = attachment.shared.terminal.lock().unwrap();
    let screen = terminal.as_mut().expect("snapshot restored").screen(true)?.expect("screen");
    println!("{}x{} cursor {:?}", screen.cols, screen.rows, screen.cursor.map(|c| (c.x, c.y)));
    for row in 0..screen.cells.len() {
        println!("|{}", screen.row_text(row));
    }
    Ok(())
}
