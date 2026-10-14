# Targeted capture result

The rebuilt target/debug/dojo-agent exposes optional region, window_id and scale
fields on desktop_screenshot. Existing output-only requests remain valid. Captures
return origin and actual-image mapping. Window targets crop visible tile bounds;
other surfaces may overlap them. Unknown, inactive, offscreen or unpositioned
windows error. Regions clip to the selected output; scale is bounded 0.1–1.

Verification:

- devenv shell -- cargo test --bin dojo-agent: 11 passed.
- Rebuilt MCP in private nested Niri, normal output origin: PASS,
  /tmp/dojo-agent-nested.nnJ2dx/events.jsonl.capture-results.json.
- Rebuilt MCP in private nested Niri at (-800, 100): PASS,
  /tmp/dojo-agent-nested.zfv9Pg/events.jsonl.capture-results.json.
  Actual pixels match literal fixture colors, region and scaled PNG dimensions
  match expected values, window-crop coordinate mapping delivers a real click,
  focus/clipboard stay unchanged, and invalid inputs error without a seat.
- Live full-output capture: PASS, /tmp/dojo-mcp-targeted-live.png, with existing
  focus/clipboard checks. Actual PNG dimensions 1646x1029 drive the mapping from
  Niri's rounded logical dimensions 1645x1028.
- cargo fmt --check, Python syntax and git diff --check: PASS.

The first nested attempt used an older debug compositor lacking coordination
requests, so it failed before capture. Verification used the previously installed
4677bd2c0bdd6fea compositor in a private nested session. A subsequent real crop
attempt exposed grim's mutually exclusive -o/-g constraint; command construction
was corrected, rebuilt and all final checks passed. The host compositor was not
restarted or changed.

Current tiled-window positions are absent from Niri IPC. This unit does not add
compositor geometry, arbitrary-resolution supersampling or hidden-window capture.
The grim process and encoded result retain existing timeout/size limits; those
are not a strict cap on compositor-side capture buffers. No measured token or
power saving is claimed. Existing running MCP servers need reload to advertise
and use the rebuilt tool schema. No global agent configuration was changed.

The memory gist was read for discussion only; see memory-discussion.md. No memory
engine, prompt replacement, provider changes or shared configuration were applied.
