# SPEC: lintel

## What this is
lintel is the small X11 window manager of The Monolith, built on protolith. Windows float; dragged by the title bar
to an edge, a corner or the top of the screen, or near the edge of a tiled window, they tile where they are dropped.
It speaks ICCCM and the core of EWMH, so programs get what they expect from a window manager.

Non-goals: a launcher, a taskbar and workspaces (other programs' job).

Not yet there, planned only when an issue asks for it: resizing tiled windows by the edge they share, more than one
screen, cursor shapes, keyboard move and resize, fonts other than Terminus 12.

## Principles
From AGENTS.md (its non-negotiables and the rules under its gate), as they stand:
- No dependencies but protolith; no `unsafe` (`#![forbid(unsafe_code)]`).
- Size is priced: the i486 build (`tools/build-i486.sh`) is held to a budget of 200 KB; it is raised only with a
  reason in the commit that raises it.
- Rust 1.75 (the MSRV) and stable.
- Frames, borders and the drop outline are server-painted window backgrounds. Only the title bar is drawn
  client-side, as a protolith window inside the frame; the frame is never a protolith window.
- Policy lives in lintel; X11 mechanism lives in protolith. A request or event protolith lacks is added to
  protolith, with its tests, not worked around here.
- Docs and CHANGELOG change in the same commit as the code.
- New tests are mutation-checked: break the code, see the test fail.

## State
Done: everything listed under 0.1.0 in `CHANGELOG.md` (frames, dropping and splitting, the outline, focus, the
keys, the ICCCM and EWMH hints, the i486 build under its budget). Nothing is half-built.

Planned: issue #1, one title-bar canvas shared by all windows instead of one per window, to cut the heap with many
windows open. Not started.

Known failure: the test `real_pointer_and_keyboard` (`tests/xvfb.rs`) fails here and in CI. Its probe
`have_xdotool()` runs `xdotool version` without the test's `DISPLAY`; that xdotool opens a display even for
`version`, fails, and with `LINTEL_REQUIRE_XVFB` set the test asserts "xdotool is required", so real input is never
tested. It is listed in the graph's `gate.known_red`.

Priorities: first fix that test's harness (give the probe the Xvfb display) and remove it from `known_red`; then
issue #1; later issues in the owner's order.

## Architecture
- `src/layout.rs`: where a drop lands, pure geometry (edge and corner zones, halves, splits, rescaling to a new
  screen size, leaving a tile under the pointer), with its unit tests inline.
- `src/wm.rs`: the window manager, `Wm`. It frames each window (a bare server-painted frame holding the title bar,
  a protolith window), handles focus, drags and drops, the drop outline, the key grabs, and the ICCCM and EWMH
  hints; `Wm::step` waits for events, handles all that are queued and sends what they caused.
- `src/main.rs`: the `lintel` binary: its arguments (`--display`, `--help`, `--version`), then `Wm::step` in a
  loop. `src/lib.rs` makes `layout` and `wm` public modules, so tests in `tests/` reach both.
- `tests/xvfb.rs`: the window manager in-process on its own Xvfb per test, programs played by protolith
  connections; `real_pointer_and_keyboard` drives real input through xdotool (XTEST).
- `tools/build-i486.sh` and `targets/i486-monolith-linux-musl.json`: the i486 build in the pinned builder image,
  with the size budget.
- `.github/workflows/ci.yml`: the `gate` job (fmt, clippy, doc, `cargo test`, ten parallel test runs, on stable
  and 1.75) and the `i486` job.
- protolith is a path dependency at `../protolith`; CI checks out its default branch beside lintel.

## The regime
The graph, `docs/graph.yaml`, holds the gate (the test command, the pattern of its failure lines and the failures
it already has), the paths out of scope, and the nodes. A node is built with `/noodlebot:node <ID>`, as
`AGENTS.md` describes.

## Node spec template
A node spec `nodes/<ID>-<name>.md` has the headings Purpose, Tests, Changes, Gate and Risks, a header row
`| Track | Deps | Unlocks | Size |`, an **Owns:** line, and criteria `- **<ID>-AC-<n>** — … *Verify:* …`.

## Open decisions
- **The gate stops at the first failing test binary.** `cargo test` runs the test binaries in turn (the library's
  unit tests, then `tests/*.rs` by name, then doc tests) and stops after the first that fails. While
  `real_pointer_and_keyboard` is red, a test file whose name sorts after `xvfb.rs`, and the doc tests, do not run
  in the gate. Default: the gate stays as confirmed; until that test is fixed, a node that adds such a file records
  its own run of that file beside the gate run.
- **Doc tests' failure lines.** A failing doc test prints `test src/<file> - <item> (line <n>) ... FAILED`, which
  `fail_pattern` does not match. The crate has no doc tests today. Default: none are added; a node that adds one
  widens the pattern in its spec PR.
- **phonolite's drag script.** `AGENTS.md` says phonolite's `tools/check-windows.sh` also passes with lintel as the
  window manager; neither CI nor the gate runs it. Default: it is not required evidence; a node that changes drags
  or drops records its run when phonolite is at hand, else says it was not run.
- **The harness fix and the roles.** The fix of `real_pointer_and_keyboard` is in `tests/xvfb.rs`, a test path,
  which the role guard leaves to the tester. Default: that node's red target is the test itself
  (`LINTEL_REQUIRE_XVFB=1 cargo test --test xvfb real_pointer_and_keyboard`), and its spec says which role writes
  the harness change.

## Conventions
Branches `node/<ID>-<name>` and `spec/<topic>`; sizes S (under a day), M (two or three days), L (more); a node is
done when its gate passes in CI on its PR and the PR is merged. A node that needs something protolith lacks stops
and asks: the change goes to protolith first, and no lintel node builds on unmerged protolith work.
