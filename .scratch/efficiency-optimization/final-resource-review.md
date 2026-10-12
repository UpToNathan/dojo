This earlier review is superseded by [final-allocation-review.md](final-allocation-review.md).

# Final resource review

The final evidence closes the three gaps from the earlier review. No concrete correctness defect was found in the implementation or activation evidence.

The 300-frame tray probe is retained at `/tmp/dojo-optimization/tray-gpu-lifecycle.log`: cache redraw and replacement/removal modes each show zero growth; the recreate control grows as expected and is checked separately. The five-cycle live picker check returns to its 414,044 KiB GPU-memory baseline, with at most 4 KiB closed-state growth and a visually verified preview. The final dirty source overlay is reproducible from base HEAD `7fb1e4ffa79843314624783fe8f302c734322caf` using the 81-file manifest and archive; manifest digest: `d3e02257c36c17f1ac659afd230d8e823227c838f36576b0a7b45c2c399f52bb`.

The retained suite logs support 181 passed tests and one existing ignored test: root 101 (41 library, 52 shell binary, 8 agent binary), native speech 43, voice 17, Niri input 19, and renderer 1. The outcome correctly discloses that live preview replacement within one picker and output-scale changes were not exercised, and that Apply/Derive were not used. Focused thumbnail tests cover decoding and scale budgets, but do not close those live checks. The AC observation supports idle residency only; it does not support a battery-life or controlled before/after speedup claim.

## Attention

reviewed by gpt-6-luna

- Keep live picker replacement and output-scale changes marked unverified; the five-cycle check establishes open/close cleanup only.
- The final quiet counter rerun is still being incorporated by the root agent into `outcome.md`; use its updated values when quoting resident CPU/PSS/GPU figures.
