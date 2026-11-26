# lintel

The beam over the window: a small X11 window manager that floats windows
and tiles them where you drop them. Built for The Monolith, to run on a
Pentium and, with luck, a 486.

Nothing to learn. Drag a window by its title bar:

- to the **left or right edge** of the screen: it fills that half;
- to a **corner**: that quarter;
- to the **top**: the whole screen;
- near an **edge of a tiled window**: the two share that window's place,
  split on that side;
- **away** from its tile: it floats again, at the size it had.

An outline shows where it will land before you let go. Double-click the
title bar, or press its box button, to maximize and restore; the cross
closes. Drag a floating window's border to resize it. Clicking a window
focuses and raises it, and the click still reaches it.

| Keys | |
|---|---|
| Super+Left / Super+Right | the left / right half |
| Super+Up | maximize |
| Super+Down | float again (a floating window: minimize) |
| Alt+Tab | the next window (minimized ones come back) |
| Alt+F4 | close |

## How it works

lintel is built on [protolith](https://github.com/tuckermclean/protolith),
which speaks the X11 protocol itself over the socket: no Xlib, no XCB, no
C, no `unsafe`, no dependencies. lintel is about 1,300 lines on top of it.

Each window is reparented into a **frame**: a bare X window whose
background the server paints as the border, so it costs the client no
memory and no drawing. Inside it sits the **lintel**, the title bar: the
one thing lintel draws, a protolith window 18 pixels tall. (A frame drawn
like an ordinary window would carry a backing image the size of the
program it holds — 3 MB per full-screen window.) The drop outline is four
thin server-painted windows. Windows that ask for no decorations (Motif
hints), such as phonolite's, get a frame with no border or bar and go
exactly where they ask, so phonolite's own snapping works under lintel.

Programs get what they expect from a window manager (ICCCM, and the core
of EWMH):

- `WM_DELETE_WINDOW` to close (else `KillClient`), `WM_TAKE_FOCUS`,
  `WM_STATE`, the save-set (if lintel dies, its windows go back to the
  root, unharmed), synthetic `ConfigureNotify` after moves;
- `_NET_SUPPORTING_WM_CHECK`, `_NET_SUPPORTED`, `_NET_CLIENT_LIST`,
  `_NET_ACTIVE_WINDOW`, `_NET_CLOSE_WINDOW`, `_NET_FRAME_EXTENTS`,
  `_NET_WM_STATE` (full screen, above, maximized, hidden);
- floating programs move and resize themselves as they ask; tiled,
  maximized and full-screen ones are told no, and where they are.
  Fixed-size windows are never tiled. Dialogs open over their owner.
  Panels and desktops (`_NET_WM_WINDOW_TYPE`) are left alone. Windows
  already on the screen when lintel starts are adopted.

**Size.** Built for an i486 the way Stele is (`tools/build-i486.sh`: static
musl, std rebuilt from source, panics aborting), lintel is **177 KB**; CI
holds it under 200 KB.

## Building and running

```sh
cargo build --release        # protolith checked out at ../protolith
lintel                       # from ~/.xinitrc: exec lintel, after starting programs
lintel --display :1
```

Rust 1.75 or later. Starting lintel while another window manager runs
fails with "another window manager is running".

## Not (yet) there

- No launcher, taskbar or workspaces: start programs from `.xinitrc` or a
  terminal; minimized windows come back with Alt+Tab.
- Tiled windows are not resized by dragging the edge they share.
- One screen (no Xinerama); no cursor shapes; no keyboard move or resize.
- Titles are drawn in Terminus 12; characters it lacks show as hollow boxes.

## License

MIT or Apache-2.0, at your option. The Terminus font protolith carries is
under the SIL Open Font License 1.1.
