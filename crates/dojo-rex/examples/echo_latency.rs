//! Times a key's round trip through Rex: `terminal.write` on the control connection, the pty's
//! echo, and its arrival on the data stream. Excludes drawing. `cargo run --release --example echo_latency`
use std::time::{Duration, Instant};

use dojo_rex::data::{self, Chunk, DataStream};
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let socket = dojo_rex::default_socket();
    let control = dojo_rex::Control::connect(&socket, "echo-latency")?;
    let created = control.call("session.create", json!({"label": "dojo-echo-latency", "initial_windows": [{"layout": {"block": {
        "flavor": "com.superlogical.terminal.shell", "options": {"shell": "none", "command": ["cat"]}}}}]}))?;
    let session = created["session_id"].as_str().unwrap().to_owned();
    let block = created["initial_windows"][0]["block_ids"][0].as_str().unwrap().to_owned();
    control.call("session.attach", json!({"session_id": session}))?;
    let ticket = control.call("data.connect", json!({"session_id": session, "block_id": block, "data_ref_id": "terminal", "subprotocol": data::SUBPROTOCOL}))?;
    let mut stream = DataStream::open(&socket, &data::decode_ticket(ticket["ticket"].as_str().unwrap())?)?;
    // The snapshot and anything else already queued.
    let (_terminal, _) = dojo_rex::Terminal::from_snapshot(&mut || match stream.next()? { Chunk::Bytes(b) => Ok(Some(b)), Chunk::End => Ok(None) })?;
    std::thread::sleep(Duration::from_millis(300));
    let mut samples = Vec::new();
    for i in 0..300 {
        let key = [b'a' + (i % 26) as u8];
        let start = Instant::now();
        control.block_call("com.superlogical.terminal.write", &session, &block, json!({"data": String::from_utf8_lossy(&key)}))?;
        loop {
            match stream.next()? {
                Chunk::Bytes(bytes) if bytes.contains(&key[0]) => break,
                Chunk::Bytes(_) => {}
                Chunk::End => anyhow::bail!("stream ended"),
            }
        }
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        std::thread::sleep(Duration::from_millis(5));
    }
    control.call("session.destroy", json!({"session_id": session}))?;
    report("rex", &mut samples);
    Ok(())
}

fn report(name: &str, samples: &mut [f64]) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pick = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    println!("{name}: n={} p50={:.3}ms p90={:.3}ms p99={:.3}ms max={:.3}ms", samples.len(), pick(0.5), pick(0.9), pick(0.99), pick(1.0));
}
