//! lintel: a small X11 window manager that floats windows and tiles them
//! where you drop them — the beam over the window.
//!
//! Built on [protolith](https://github.com/tuckermclean/protolith), which
//! speaks the X11 protocol itself: no Xlib, no XCB, no C, no `unsafe`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod layout;
pub mod wm;
