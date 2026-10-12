# Resource ownership and limits

| Resource | Ownership and limit |
|---|---|
| Tray commands | One owner thread, FIFO capacity 32; overload returns a logged rejection. Shutdown has an independent wake. |
| Tray changes | Capacity-one notification per live view; immutable shared item snapshots and ARGB buffers. Closed subscribers are removed. |
| Tray bus streams | Five match-specific streams, capacity eight each; owned proxies and cancellation. Nonempty tray has a 30-second compatibility sweep. Empty tray has no periodic deadline. |
| Tray images | At most 16 accepted pixmaps per item; each at most 512 by 512 by four bytes. This bounds accepted pixels at 16 MiB per item, not total bus decode memory or all registered items. |
| Tray menus | Maximum depth 16 and 512 entries per layout. Cached menu refreshes publish only changes. |
| File events | One inotify owner, 16 KiB read batch and capacity-one subscriber notifications. Healthy watches do not poll files. Failed watches retry after 30 seconds. |
| Host status | One shared observer, UPower signals plus a 10-second fallback for battery and externally started vigiland. No pgrep child and no per-output polling. |
| Speech bubbles | Latest eight visible messages. Actual transitions and working pulses animate; static bubbles use one deadline timer. Reduced motion has no pulse frames. |
| Voice clients | At most 32 simultaneous Unix clients; each input buffer capped at 16,384 JavaScript string code units, with a 45-second timeout. The bound is not a UTF-8 byte count. |
| Voice work | At most 128 queued operations. One nearest-deadline timer, one reconnect timer, and coalesced persistence. Singleton advisory lock precedes socket replacement and preload. |
| Voice history | Conversation metadata remains retained; it is not globally memory-bounded. Warm model residency remains intentional. Native in-flight inference cancellation is deferred. |
| Large temporary allocations | Versioned shell bootstrap fixes the glibc mmap threshold at 131,072 bytes unless the caller supplied a threshold. Verified with five interleaved runs per setting and five preview cycles per run. |
| Picker source | At most 50 MiB compressed source bytes. At most 16,384 pixels per axis and 40,000,000 source pixels before decoding. Original bytes remain the Apply/Derive input. |
| Picker preview | Display-sized pixels, with no upscaling. Animated WebP retains at most 32 thumbnail frames and 16 MiB thumbnail pixels; larger animations use the first frame. Images are evicted on replacement and close. |
| Wallpaper | One shared cache for the current wallpaper across outputs. Replacing the path clears the old CPU cache and all window atlas entries. Entity release evicts images. Full wallpaper decode remains necessary. |
| Renderer paths | Scratch textures allocate on the first path scene and survive reuse/resize/recovery. They remain resident after first use; this is lazy allocation, not eviction. |
| Launcher | Resolve at most ten visible-row icons per lookup. Catalogue replacement cancels lookups and clears the index-keyed cache. Previously visited icons remain bounded by that catalogue's application count. |
| Screenshots | Read PNG dimensions without decoding pixels. Preserve captured PNG bytes; maximum 32 MiB compressed, 32,768 per axis, 134,217,728 pixels, and five-second capture timeout. |

These are local bounds. They do not imply a fixed total process budget: the number
of outputs, tray providers, history entries, applications, fonts, and GPU driver
allocations also matters. Process PSS and per-client DRM memory are measured
separately. Shared GPU allocations cannot be summed across clients.

No battery runtime improvement is claimed from AC measurements. Existing display
refresh, power policy, warm speech capability, and suspend behavior remain in place.

The allocator setting is scoped to the shell installation and preserves existing
caller thresholds. Its behavior is described in the
[GNU libc allocator manual](https://sourceware.org/glibc/manual/latest/html_node/Memory-Allocation-Tunables.html).
It reduces retained decode allocations in the measured workload, not the memory
required for an active wallpaper or warm speech model.
