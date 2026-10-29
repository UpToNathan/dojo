//! Drives a whole session: split, a new tab, the listing, closing panes.
//! `cargo run --example session_check`
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let wait = || std::thread::sleep(Duration::from_millis(400));
    let session = dojo_rex::Session::open(
        dojo_rex::SessionOptions {
            socket: dojo_rex::default_socket(),
            label: "dojo-session-check".into(),
            command: vec![],
            pane_command: vec![],
            cwd: Some("/tmp".into()),
            cols: 80,
            rows: 24,
            client_name: "session-check".into(),
        },
        || {},
    )?;
    wait();
    let show = |what: &str| {
        let view = session.view();
        let panes = view.panes([0.0, 0.0, 800.0, 600.0], 2.0);
        let streaming = panes.iter().filter(|(block, _)| session.pane(block).is_some_and(|p| p.terminal.lock().unwrap().is_some())).count();
        println!("{what}: {} tab(s), {} pane(s), {} streaming, focused {:?}", view.windows.len(), panes.len(), streaming, view.focused_block().map(|b| b[b.len() - 6..].to_owned()));
    };
    show("opened");
    session.split(true)?;
    wait();
    show("split right");
    session.split(false)?;
    wait();
    show("split down");
    session.new_tab()?;
    wait();
    show("new tab");
    session.next_tab(false)?;
    wait();
    show("previous tab");
    session.close_pane()?;
    wait();
    show("closed a pane");
    let control = dojo_rex::Control::connect(&dojo_rex::default_socket(), "session-check-list")?;
    for summary in dojo_rex::list_sessions(&control)? {
        println!("listed: {} tabs={} dir={:?} title={:?} status={:?}", summary.label, summary.tabs, summary.directory, summary.title, summary.status);
    }
    control.call("session.attach", serde_json::json!({"session_id": session.id()}))?;
    control.call("session.destroy", serde_json::json!({"session_id": session.id()}))?;
    wait();
    println!("ended after destroy: {}", session.ended());
    Ok(())
}
