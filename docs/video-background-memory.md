# Video background memory — findings and proposals

Research note. Documents what the video background path does to memory and
lists candidate changes, ordered by expected impact. **No code is modified by
this document.** Date: 2026-11-21.

## Scope

In scope:

- The on-disk asset store that feeds backgrounds (`src/util/cache.rs`, the
  `Store`), for the parts the video path depends on.
- The background widget media path (`src/widgets/background.rs`,
  `data/resources/ui/background.ui`).
- The playback stack `GtkVideo` drives on this system.

Out of scope: the irmin/sophon download subsystem (separate concern,
unaffected by this path), and server-side asset generation except for one
flagged suggestion in §7.

## The playback stack

- `gtk4 0.11.5` (features `v4_22`, `gnome_50`). The app talks to video only
  through `GtkVideo::set_filename`, `MediaStream::set_muted`, and a
  `timestamp` notify handler (`src/widgets/background.rs:120`–`212`).
- **There is no `gstreamer-*` crate in `Cargo.lock`.** Decoding is entirely
  inside libgtk, against system GStreamer 1.28.9
  (`pkg-config --modversion gstreamer-1.0` → `1.28.9`). The app has no handle
  to pipeline elements, buffer pools, or caps. The only surfaces available
  from the application are: which file the widget points at, when it is
  started/stopped, and the GTK render target.
- Local plugin inventory (CachyOS, measured 2026-11-21,
  `/usr/lib64/gstreamer-1.0`): `openh264` (CPU H.264 decode),
  `videoparsersbad`, `coreelements`, `opengl`, `waylandsink`,
  `playback` present; **no** `gstreamer-ffmpeg`, **no** `vaapi`, **no**
  `v4l2` hardware decode binaries. On this machine decode is software
  (openh264 on the CPU) and upload/render is GL. Other installations may
  resolve `decodebin` to hardware decoders; the app-side analysis below does
  not depend on which one is chosen.

## Runtime flow (verified)

1. `set_media` (`background.rs:71`): on a game change, `clear_media()` and
   bump the request token. Work runs in `spawn_blocking` behind an
   `async_channel::bounded(1)`. `cache::resolve_media(game, &locale,
   use_video)` selects the background; **when `media.is_video`, a second
   `resolve_media(game, &locale, false)` resolves the poster still** — two
   store operations per request in video mode.
2. Main-thread receiver checks the request token, discards stale results,
   and calls `apply_media` (`background.rs:130`). That decodes the poster
   (and overlay) via `Texture::from_filename` — a **synchronous decode on
   the main thread** — pins all resolved paths through
   `cache::set_active_assets`, and sets the texture on the `GtkPicture`. The
   poster picture stays mapped beneath the video.
3. Video mapping (`background.rs:188`–`212`): `set_autoplay(false)`,
   `set_filename(None)`, opacity 0, then `set_filename(Some(path))`, mute,
   and reveal (fade in, re-enable autoplay) only once the first
   `timestamp > 0`. The loop (5–8 s, see §4) then runs continuously for the
   lifetime of the window.
4. `dispose` (`background.rs:48`): stops autoplay and clears the filename
   before the widget is released.

Late results are dropped at the token check, `Weak` upgrades guard against
disposed widgets, and the bounded channel caps in-flight requests at one
plus one queued. No unbounded accumulation exists in the current code.

## Cache store semantics the video path relies on

From `src/util/cache.rs` (line numbers as of this note):

- Constants: `CATALOG_CONCURRENCY = 3` (catalog fetches; assets and
  publications stay serial), `ASSET_BUDGET = 512 MiB` (asset-byte cap,
  including download staging; the cap is bytes, not filesystem allocation —
  comment `:27`–`31`), `ASSET_TTL = 7 days` (unpinned URLs rechecked after
  expiry; URLs are not assumed immutable), `MAX_CATALOG_SIZE = 4 MiB`.
- Downloads stage into `.download-*` temporaries, commit under the pin lock
  ("Cache pin lock poisoned", `:495`), and rename atomically.
  `TemporaryFile` removes an unpublished staging file on drop; `Publication`
  rolls the catalog back on drop **and on cancellation**, so a cold cache
  keeps its last usable image.
- `resolve` (`:434`–`445`) loops the locale preference and returns the first
  background entry whose media is present for the requested kind; a stale
  catalog is re-validated after publication ("Even a stale-but-valid image
  can be displayed before revalidation").
- `set_active` (`:491`) validates every supplied path — regular file, inside
  the asset tree, no unsafe components — then swaps the whole pin set.
  Referenced and active files are never evicted; an oversized working set
  keeps its last usable catalog instead of blanking.

Conclusion for this path: the displayed files (video, poster, overlay) are
immune to eviction, and the budget cannot starve the current media. Nothing
to fix in the store for the memory question.

## On-disk footprint (measured 2026-11-21)

`~/.local/share/app.elysiae.Elysiae/cache` — **26 MiB** (`du -sh`):

| Asset | File | Size (bytes) |
| --- | --- | ---: |
| bh3 background (still) | `a8e8476a…98e.webp` | 4,621,220 |
| bh3 background (still) | `1ebe1636…84af.webp` | 4,254,502 |
| bh3 background (video) | `edc96571…7adf.mp4` | 4,246,063 |
| bh3 icon | `fd77423c…505.png` | 19,378 |
| bh3 overlay | `5f946200…12.webp` | 452,034 |
| hk4e background (still) | `59d18567…8ce6.webp` | 2,919,676 |
| hk4e background (still) | `d2295fb7…d3425…c0dd.webp` | 2,771,664 |
| hk4e background (video) | `a6f32890…1363…1363df5a…` | 2,772,092 |
| hk4e background (video) | `d58191d0…71ef8…` | 3,718,649 |
| hk4e icon | `043c5f59…9cbdd2.png` | 21,689 |
| hk4e overlay | `3a33a776…851a38…` | 640,922 |
| hkrpg icon | `98a20bde…4ad9447.png` | 22,345 |
| nap icon | `8f684313…8549…b79c…` | 21,891 |

Video encode parameters (`ffprobe`):

| File | Resolution | Frame rate | Bitrate | Duration |
| --- | --- | --- | --- | --- |
| bh3 (4.1 MiB) | 1920×1080 | 60 fps | 6.34 Mbps | 5.35 s |
| hk4e (2.7 MiB) | 1920×1080 | 60 fps | 4.14 Mbps | 5.35 s |
| hk4e (3.6 MiB) | 1920×1080 | 30 fps | 3.69 Mbps | 8.06 s |

Comparators on this machine (not part of the store): a `cache.old`
leftover from Sep 6 holds 44 MiB in the same layout with a non-overlapping
hash set and is outside the store's GC; `~/.cache/app.elysiae.Elysiae/cef`
(176 MiB, CEF/WebKit profile) is the largest elysiae on-disk cache and is
unrelated to the asset store.

## Runtime memory estimate

Per active video background, working set while the loop plays:

- **Decode buffers (dominant term).** A 1080p4:2:0 frame is
  1920 × 1080 × 1.5 ≈ 2.97 MiB. A software H.264 decode bin keeps reference
  frames plus the buffer pool GStreamer allocates through `decodebin`;
  estimated **15–40 MiB** while the pipeline is primed. This pool exists for
  the pipeline's lifetime and is released only when the pipeline is torn
  down (`set_filename(None)` / stop), not by pausing.
- **Render upload.** GL textures for decoded frames; an 1080p RGBA frame
  is ≈ 7.91 MiB, double-buffered at the sink.
- **Poster texture.** `Texture::from_filename` over a 1080p WebP → ≈ 7.91
  MiB RGBA, **kept mapped while the video plays** (by design, it sits
  beneath the initially transparent video).
- **Overlay texture.** The overlay WebP, ≈ 7.91 MiB at 1080p.
- The video file itself is not resident wholesale: the demuxer streams it
  with bounded internal buffers.

Static (image-only) mode comparison: poster texture ≈ 7.91 MiB plus the
overlay, no decode pool, no continuous decode work.

Indicative totals: **video mode ≈ 40–70 MiB**, **static mode ≈ 16–24 MiB**
for the 1080p assets measured above; the band is set by the decode pool,
whose exact size depends on the decoder resolved at runtime.

## Findings

1. **No application-level decode knobs.** No GStreamer crate is linked;
   pools, caps, and element properties are inside libgtk's pipeline. The
   app-side levers are exactly: file choice, start/stop timing, visibility,
   and the render target.
2. **Two resolutions per request in video mode.** The poster path is already
   a field of the same catalog entry (`AedesBackgroundAssets.image` next to
   `.video`); the second `resolve_media` is redundant store work. Cost is
   small I/O, but it is duplicated logging and duplicated path validation.
3. **The decode pool is resident for the entire video lifetime.** The loop
   runs unconditionally whenever the window is mapped — including while the
   window is hidden, minimized, or occluded. There is no idle detection.
4. **The poster is never released after reveal.** ≈ 8 MiB stays decoded
   although the video is the visible layer.
5. **Main-thread decode of the poster and overlay** at `apply_media`
   (synchronous `Texture::from_filename` on 4–4.6 MiB WebPs): a tens-of-ms
   main-thread stall on first display, per game change.
6. **Continuous 1080p software decode** (openh264, 30–60 fps, looping) is a
   steady CPU/energy load while the window lives. Listed for completeness;
   it is a compute cost, not a memory leak.
7. **The store is sound for this workload.** 26 MiB working set against a
   512 MiB budget; displayed files are pinned and unevictable; staging and
   publication roll back correctly. No change indicated.
8. **No accumulation bug in the current widget code.** Token-checked late
   results, weak upgrades, `bounded(1)` channel, and dispose-time teardown
   all check out.

## Proposals (not implemented; ordered by expected impact)

### A. Tear down the pipeline when the window is hidden (recommended)

- Change: on window hide/minimize (window state notification), stop the
  video (`set_filename(None)` + stop autoplay); restore it
  (`set_filename(Some(path))`) on regain of visibility. The poster stays
  mapped beneath, so a stop shows the still, not a black layer; re-prime is
  sub-second for a local 4 MiB file.
- Effect: releases the decode buffer pool (15–40 MiB) and all decode CPU
  while hidden. This is the **only app-side change that frees the decode
  buffers**; pausing alone would save CPU but keep the pool.
- Risk: re-prime latency on reshown, mitigated by the poster underneath and
  the existing reveal-on-first-timestamp logic, which already handles the
  mapping sequence. Must interact with the request/video-generation
  invalidation so a hidden-window dispose cannot resurrect playback.

### B. Release the poster texture after the first frame is confirmed

- Change: in the `timestamp > 0` reveal handler, drop the poster paintable
  (`set_paintable(None)` on the background picture) once the video is
  confidently the visible layer; restore the texture when the pipeline is
  stopped (pairs with proposal A).
- Effect: ≈ 8 MiB during video playback.
- Risk: if the video fails mid-cycle, the fallback layer becomes the wordmark
  box rather than the still. Acceptable for a background, or the texture can
  be cached on the widget to make restoration free.

### C. Server-side: a smaller video variant (flagged to the asset producer)

- Change: add an optional low-variant video to the Aedes catalog (720p, or
  30 fps at 1920×1080); the app prefers the base variant only where
  available.
- Effect: decode pool and render upload scale roughly with pixel count —
  720p4:2:0 is ≈ 1.5 MiB per frame, about half the 1080p buffer cost; 30 fps
  halves decode work. For a looping background this is visually adequate.
- Scope note: asset-side, outside this repository; the app-side read of a
  new catalog field is trivial and already covered by the catalog
  re-validation path.

### D. Move poster/overlay decoding off the main thread

- Change: decode the WebP in `spawn_blocking` (e.g., to a `GdkPixbuf`),
  then construct the `Texture` on the main loop. Keep the existing
  request-token invalidation around the whole operation.
- Effect: removes the tens-of-ms main-thread stall on game change; memory
  unchanged.

### E. Single store call returning video and poster together

- Change: extend resolution so one catalog read returns both the video path
   and the matching still (the `image` field of the same
  `AedesBackgroundAssets` entry), removing the second `resolve_media` call,
  its duplicate logging, and its duplicate path validation.
- Effect: minor I/O and CPU per request; simplifies `set_media`. Pure
  cleanup relative to A–D.

### Deliberately not proposed

- Changing `ASSET_BUDGET`, `ASSET_TTL`, or the pin model: the measured
  working set (26 MiB) is far under the cap and the pin model already
  protects displayed media.
- GStreamer environment-variable tuning: not controllable from inside the
  process, machine-specific, and it would race libgtk's own pipeline
  construction.

## Verification

- No code modified: this commit touches documentation only; `src/`,
  `data/`, `Cargo.toml`, `Cargo.lock` unchanged (46 tests passing at the
  `P6` baseline, carried forward).
- Footprint re-measured 2026-11-21 (`du -sh` on the store, `ffprobe` on all
  three cached videos, byte table above).
- Plugin inventory verified by direct listing of
  `/usr/lib64/gstreamer-1.0` and `pkg-config --modversion gstreamer-1.0`
  (1.28.9) on the development machine; `Cargo.lock` grepped — zero
  `gst*` entries.
- Store semantics quoted from `src/util/cache.rs` as of commit `9fedfa4` on
  `chore/launcher-fixes`; line references are to that revision.