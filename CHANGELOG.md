# Changelog

## 0.1.0 — unreleased

First version.

- **Frames**: each window reparented into a server-painted frame with a
  protolith title bar (title, maximize, close); undecorated windows
  (Motif hints) framed bare and placed exactly as asked; save-set, so
  lintel's death leaves windows unharmed; windows on the screen at start
  adopted; withdrawn windows let go, back on the root.
- **Dropping**: title-bar drags to the left/right edge (half), a corner
  (quarter), the top (maximize), near a tiled window's edge (split it);
  dragging off a tile floats the window at its old size under the
  pointer; an outline of four thin windows shows the drop. Double click
  or the box button maximizes; border drags resize floating windows.
- **Focus**: click to focus and raise, the click passed on
  (`AllowEvents(ReplayPointer)`); `WM_TAKE_FOCUS`; `_NET_ACTIVE_WINDOW`.
- **Keys**: Super+arrows, Alt+Tab, Alt+F4 (grabbed with Caps and Num Lock
  variants).
- **Hints**: ICCCM (`WM_DELETE_WINDOW`, `WM_STATE`, `WM_CHANGE_STATE`,
  synthetic `ConfigureNotify`, `WM_NORMAL_HINTS` fixed size and static
  gravity, `WM_TRANSIENT_FOR`), EWMH core (`_NET_SUPPORTED`,
  `_NET_SUPPORTING_WM_CHECK`, `_NET_CLIENT_LIST`, `_NET_FRAME_EXTENTS`,
  `_NET_WM_STATE` full screen/above/maximized/hidden, window types).
- **Size**: 177 KB on the i486 build; budget 200 KB.
- **Tests**: drop geometry; on Xvfb, framing, drops and splits, the
  outline, self-placement while floating only, undecorated windows,
  closing and withdrawal, full screen and shortcuts, survival of lintel's
  exit and re-adoption, the synthetic ConfigureNotify, and real XTEST
  input (click-to-focus with the click passed on, key grabs, a title
  drag, a border resize, Alt+Tab).
