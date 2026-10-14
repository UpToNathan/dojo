use dojo_cua_worker::worker::{ChannelRequest, ChannelResponse, WorkerInitialization};
use serde_json::json;
use std::io::{BufRead, Write};

fn main() {
    let generation = std::env::args().nth(3).unwrap();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let request: ChannelRequest = serde_json::from_str(&line.unwrap()).unwrap();
        let result = match request.operation.as_str() {
            "initialize" => {
                let initialization: WorkerInitialization =
                    serde_json::from_value(request.arguments.unwrap()).unwrap();
                json!({"ready":true,"pid":std::process::id(),"host_bundle_id":initialization.host_bundle_id})
            }
            "metadata" => json!({"driver_version":"fixture","contract_version":"fixture",
                "tools_list_schema_version":"fixture","capability_version":"fixture","mcp_protocol_version":"fixture",
                "pid":std::process::id(),"embedded":true,"host_bundle_id":"fixture"}),
            "list" => json!({"tools":[]}),
            "bind_session" => json!({"session_handle":"fixture-session"}),
            "close_session" => json!({"closed":true}),
            "shutdown" => json!({"shutdown":true}),
            "call" if request.name.as_deref() == Some("block") => {
                std::fs::write(
                    request.arguments.unwrap()["ready_file"].as_str().unwrap(),
                    b"started",
                )
                .unwrap();
                loop {
                    std::thread::park();
                }
            }
            "call" => {
                json!({"content":[],"isError":false,"structuredContent":{"echo":request.arguments}})
            }
            _ => json!({}),
        };
        let response = ChannelResponse::ok(request.request_id, &generation, result);
        writeln!(stdout, "{}", serde_json::to_string(&response).unwrap()).unwrap();
        stdout.flush().unwrap();
        if request.operation == "shutdown" {
            break;
        }
    }
}
