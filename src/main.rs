//! `lintel [--display DISPLAY]`

#![forbid(unsafe_code)]

use lintel::wm::Wm;
use protolith::x11::Connection;

const USAGE: &str = "usage: lintel [--display DISPLAY]

A window manager. Drag a window by its title bar to the left or right edge
of the screen to fill that half, to a corner for a quarter, to the top for
all of it; drop it near the edge of a tiled window to share that window's
place. Drag it away to float it again.

Keys: Super+Left/Right  half the screen     Alt+Tab  next window
      Super+Up          maximize            Alt+F4   close
      Super+Down        float again, or minimize";

fn main() {
    let mut display = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-d" | "--display" => display = args.next(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            "-V" | "--version" => {
                println!("lintel {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("lintel: unknown argument {other:?}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let conn = match &display {
        Some(d) => Connection::open_display(d, None),
        None => Connection::open(),
    };
    let conn = conn.unwrap_or_else(|e| {
        eprintln!("lintel: cannot open the display: {e}");
        std::process::exit(1);
    });
    let mut wm = Wm::new(conn).unwrap_or_else(|e| {
        eprintln!("lintel: {e}");
        std::process::exit(1);
    });
    loop {
        if let Err(e) = wm.step(None) {
            eprintln!("lintel: {e}");
            std::process::exit(1);
        }
    }
}
