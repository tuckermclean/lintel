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

## Building a node

- **Building.** Build a node with `/noodlebot:node <ID>`. The role guard needs no `.noodlebot/roles.yaml` here:
  `tests/` matches its default test globs. The unit tests inside `src/layout.rs` count as code to the guard, so
  new tests go into `tests/` (`layout` and `wm` are public modules); the inline tests stay where they are.
- **Authority order.** `SPEC.md`, then `docs/graph.yaml`, then the node spec `nodes/<ID>-<name>.md`, then this
  file. When two disagree, the earlier one wins and the later one is fixed by a spec PR.
- **Roles.** The plugin's tester writes the failing tests, the implementer makes them pass, the reviewer judges
  the result. This repository has no role agents of its own.
- **Spec checkers.** `python3 -I <plugin dir>/skills/node/check-regime.py` and
  `python3 -I <plugin dir>/skills/node/check-coverage.py` from the repository root must print `PROBLEMS 0` before
  a spec PR is opened.
- **protolith.** Before the gate, put protolith's default branch at `../protolith`, beside the checkout, as CI
  does; the gate does not build without it. A node that needs something protolith lacks stops and asks: the change
  goes to protolith first, and no lintel node builds on unmerged protolith work.
- **Gate command.** `LINTEL_REQUIRE_XVFB=1 cargo test` from the repository root runs the whole suite; it needs
  Xvfb and xdotool. It runs in CI on the PR (the `cargo test` step of the `gate` job in
  `.github/workflows/ci.yml`, with `LINTEL_REQUIRE_XVFB` set for the job). Its failure lines are libtest's
  `test <name> ... FAILED`, which `fail_pattern` in the graph matches.
- **The other checks.** The rest of "The gate (before every commit)" above (fmt, clippy, doc, the 1.75 run, the
  ten parallel runs, `tools/build-i486.sh`) is not part of the gate command; every node's PR passes them in CI,
  and the PR records them as evidence.
- **CI output.** CI's lines may add to the gate's output, for example colour escape codes (`\x1b\[[0-9;]*m`;
  cargo's own status lines are coloured in CI) and, in a fetched job log, a timestamp prefix (`^\S+Z `);
  `fail_pattern` matches once they are stripped.
- **Quick gate.** In a session, run the gate command itself locally: the whole suite takes about a second once
  built, and it prints the same `test <name> ... FAILED` lines.
- **Bounded workers.** Locally, bound both: `cargo test -j <n> -- --test-threads=<n>`, with `<n>` at most 2
  (`-j` bounds cargo's build jobs, `--test-threads` libtest's threads; each Xvfb test starts its own Xvfb). CI's
  ten-run loop uses `--test-threads=16` on purpose.
- **Known red.** The graph's `gate.known_red` lists the tests that fail today (`real_pointer_and_keyboard`). A
  failing test that is not listed is a failure of the change. A listed failure stays until a node fixes it; that
  node removes it from the list in its own PR.
- **Owns.** A node changes only the files it owns (its `owns` globs) and its direct deps' files, and places every
  file it adds where the graph's globs cover it.
- **Red target.** A command that exits non-zero before the node's change and 0 after, normally
  `LINTEL_REQUIRE_XVFB=1 cargo test --test <file> <test name>` for the node's own test.
- **Tests and fixtures.** Tests are `tests/*.rs` (integration tests; `tests/xvfb.rs` runs the window manager on its
  own Xvfb per test) and the inline `#[cfg(test)]` module of `src/layout.rs`. There are no fixtures.
- **Spec files:** `SPEC.md`, `docs/graph.yaml`, `nodes/` (a trailing `/` marks a folder); a spec PR changes only these.
- **Porting rules.** None.
- **CI evidence.** In the node's PR, record the CI run on the PR's head commit: the `cargo test` output of both
  `gate` jobs (stable and 1.75), the other checks' results, the `i486` job's result, and the red target's output
  before and after.
