# AGENTS.md

lintel is a small X11 window manager on protolith (`../protolith`, a path
dependency): floating by default, tiling by drag-to-edge as Windows and
macOS do it — the model every computer user already knows, which is the
design goal ("like a caveman knows how to pick up a rock"). Part of The
Monolith family with protolith, phonolite and Stele.

## The non-negotiables

- **No dependencies** but protolith; **no `unsafe`** (`#![forbid]`).
- **Size is priced**: the i486 build (`tools/build-i486.sh`, Stele's
  pinned builder image) is held to a budget; raise it only with a reason
  in the commit. Target: Pentium floor, 486DX2 stretch.
- **Rust 1.75** (MSRV) and stable.
- **Cheap on the server, too**: frames, borders and the drop outline are
  server-painted window backgrounds; only the title bar is drawn
  client-side (a protolith window inside the frame, so its backing
  pixmap is bar-sized). Don't make the frame a protolith window.
- Policy lives here; X11 mechanism lives in protolith. If lintel needs a
  request or event protolith lacks, add it there (with its tests), not a
  workaround here.

## The gate (before every commit)

```sh
cargo fmt --check
RUSTFLAGS="-D warnings" cargo clippy --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
LINTEL_REQUIRE_XVFB=1 cargo test          # needs Xvfb and xdotool
LINTEL_REQUIRE_XVFB=1 cargo +1.75 test
for i in $(seq 1 10); do cargo test -q -- --test-threads=16 || break; done
tools/build-i486.sh                       # size budget
```

Mutation-check new tests (break the code, see the test fail). Docs and
CHANGELOG change in the same commit as the code.

## Tests

- `src/layout.rs`: the drop rules, pure geometry.
- `tests/xvfb.rs`: the WM in-process on its own Xvfb, programs played by
  protolith connections; checks read the server's state (parents,
  positions, sizes, properties). `real_pointer_and_keyboard` drives XTEST
  through xdotool: passive grabs, the replayed click, key grabs, border
  resize only happen with real input.
- phonolite's `tools/check-windows.sh` (its drag script) also passes with
  lintel as the window manager.

## Lessons

- A click on an unfocused window is caught by a synchronous passive grab;
  always answer it with `AllowEvents(ReplayPointer)`, or the pointer stays
  frozen.
- Reparenting a window that is already mapped unmaps it first: count that
  UnmapNotify (`ignore_unmaps`) or the adopted window is let go at once.
- protolith's windows ask for static gravity: a position they request is
  the window's own, not the frame's.
- Xvfb resets when its last client leaves.
- In simulated drags, give each press its own time, or two drags make a
  double click.
