# Expose targeted screenshot capture

Status: resolved
Type: task

Implement the spec in ../spec.md. Keep capture validation and planning in the
screenshot module. Verify against the actual MCP subprocess.

## Comments

Implemented output-relative regions, positioned-window crops and scale 0.1–1.
11 dojo-agent tests pass. Actual nested MCP checks pass at normal and negative
output origins, including pixel colors, scale dimensions, click mapping,
clipboard/focus preservation and rejected inputs. The live output-only check
passes. Geometry absent from compositor data remains an explicit error.
