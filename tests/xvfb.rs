//! lintel against a real X server: each test starts its own Xvfb
//! (`-displayfd`), runs the window manager in-process, and plays the
//! programs with protolith connections of their own. What is checked is
//! what the server says: parents, positions, sizes, properties.
//!
//! Skips without Xvfb unless `LINTEL_REQUIRE_XVFB` is set (as in CI).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use lintel::wm::{Place, Wm, BORDER, TITLE};
use protolith::x11::proto::event::{Event as Wire, InputEvent, RawEvent};
use protolith::x11::proto::request as req;
use protolith::x11::{Connection, Event, Modifiers, MouseButton, MouseButtons, WindowId, WindowOptions};
use protolith::Rect;

struct Xvfb {
    child: Child,
    display: String,
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn xvfb() -> Option<Xvfb> {
    let spawned =
        Command::new("Xvfb").args(["-displayfd", "1", "-screen", "0", "1024x768x24", "-nolisten", "tcp"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            assert!(std::env::var_os("LINTEL_REQUIRE_XVFB").is_none(), "Xvfb is required: {e}");
            eprintln!("skipping: no Xvfb ({e})");
            return None;
        }
    };
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).expect("Xvfb display number");
    Some(Xvfb { child, display: format!(":{}", line.trim()) })
}

fn connect(x: &Xvfb) -> Connection {
    Connection::open_display(&x.display, None).expect("connect")
}

/// Run the window manager until `done` holds (or fail after 5 s).
fn run_until(wm: &mut Wm, what: &str, mut done: impl FnMut(&mut Wm) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done(wm) {
        assert!(Instant::now() < deadline, "timed out: {what}");
        wm.step(Some(Duration::from_millis(10))).expect("step");
    }
}

/// Let everything in flight land: the WM's requests, the programs'.
fn settle(wm: &mut Wm, apps: &mut [&mut Connection]) {
    for _ in 0..5 {
        wm.step(Some(Duration::from_millis(10))).unwrap();
        for a in apps.iter_mut() {
            while a.poll_event().unwrap().is_some() {}
        }
    }
    wm.conn.client().sync().unwrap();
}

/// A program's window, mapped, managed.
fn open_app(wm: &mut Wm, app: &mut Connection, opts: WindowOptions) -> WindowId {
    let w = app.create_window(opts).unwrap();
    run_until(wm, "the window managed", |wm| wm.client(w.xid()).is_some());
    settle(wm, &mut [app]);
    w
}

fn parent_of(conn: &mut Connection, w: u32) -> u32 {
    let reply = conn.client().round_trip(&req::query_tree(w)).unwrap();
    req::parse_query_tree(&reply).unwrap().parent
}

/// Where a window is on the screen and how big, as the server has it.
fn on_screen(app: &mut Connection, w: WindowId) -> Rect {
    let (x, y) = app.window_position(w).unwrap();
    let reply = app.client().round_trip(&req::get_geometry(w.xid())).unwrap();
    let g = req::parse_geometry(&reply).unwrap();
    Rect::new(x, y, u32::from(g.width), u32::from(g.height))
}

/// Press on a window's title bar at `at` (bar coordinates), move the
/// pointer to `to` (screen coordinates) in steps, release.
fn drag_bar(wm: &mut Wm, app: &mut Connection, w: WindowId, at: (i32, i32), to: (i32, i32)) {
    // Each press a second after the last: no double clicks.
    static CLOCK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(100_000);
    let time = CLOCK.fetch_add(1000, std::sync::atomic::Ordering::Relaxed);
    let r = wm.client(w.xid()).unwrap().rect;
    let bar = bar_of(wm, w);
    let from = (r.x + BORDER as i32 + at.0, r.y + BORDER as i32 + at.1);
    let button = |pressed, (root_x, root_y): (i32, i32)| Event::Button {
        window: bar,
        button: MouseButton::Left,
        pressed,
        x: at.0,
        y: at.1,
        root_x,
        root_y,
        modifiers: Modifiers::default(),
        time,
    };
    wm.handle(button(true, from));
    for k in 1..=6 {
        let p = (from.0 + (to.0 - from.0) * k / 6, from.1 + (to.1 - from.1) * k / 6);
        wm.handle(Event::Motion {
            window: bar,
            x: 0,
            y: 0,
            root_x: p.0,
            root_y: p.1,
            buttons: MouseButtons { left: true, ..Default::default() },
            modifiers: Modifiers::default(),
        });
    }
    wm.handle(button(false, to));
    settle(wm, &mut [app]);
}

/// The title bar: the frame's protolith child.
fn bar_of(wm: &mut Wm, w: WindowId) -> WindowId {
    let frame = wm.client(w.xid()).unwrap().frame;
    let reply = wm.conn.client().round_trip(&req::query_tree(frame)).unwrap();
    let kids = req::parse_query_tree(&reply).unwrap().children;
    WindowId::from_xid(*kids.iter().find(|k| **k != w.xid()).expect("a bar"))
}

fn click_bar(wm: &mut Wm, w: WindowId, at: (i32, i32), time: u32) {
    let bar = bar_of(wm, w);
    for pressed in [true, false] {
        wm.handle(Event::Button {
            window: bar,
            button: MouseButton::Left,
            pressed,
            x: at.0,
            y: at.1,
            root_x: 0,
            root_y: 0,
            modifiers: Modifiers::default(),
            time,
        });
    }
}

const SCREEN: Rect = Rect::new(0, 0, 1024, 768);
const B: i32 = BORDER as i32;
const T: i32 = TITLE as i32;

#[test]
fn frames_a_window_titles_it_and_gives_it_the_keyboard() {
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    match Wm::new(connect(&x)) {
        Err(e) => assert!(e.to_string().contains("another window manager"), "{e}"),
        Ok(_) => panic!("two window managers"),
    }
    let mut app = connect(&x);
    assert!(app.has_window_manager());
    let w = open_app(&mut wm, &mut app, WindowOptions::new(200, 100).title("Hello"));
    let c = wm.client(w.xid()).unwrap();
    assert_eq!((c.place, c.title.as_str()), (Place::Floating, "Hello"));
    let frame = c.frame;
    assert_eq!(parent_of(&mut app, w.xid()), frame, "reparented into the frame");
    let f = c.rect;
    assert_eq!((f.width, f.height), (200 + 2 * BORDER, 100 + 2 * BORDER + TITLE));
    assert_eq!(on_screen(&mut app, w), Rect::new(f.x + B, f.y + B + T, 200, 100));
    // The title bar is drawn, in the focused color.
    let bar = bar_of(&mut wm, w);
    let px = wm.conn.read_pixels(bar, Rect::new(0, 0, 2, 2)).unwrap();
    assert_eq!(px[0] & 0xffffff, 0x2f5574);
    // The keyboard is the program's.
    let reply = app.client().round_trip(&req::get_input_focus()).unwrap();
    assert_eq!(u32::from_le_bytes(reply[8..12].try_into().unwrap()), w.xid());
    // EWMH: the WM names itself; the window is listed and active.
    let root = app.root();
    let atom = |app: &mut Connection, n: &str| req::parse_intern_atom(&app.client().round_trip(&req::intern_atom(n, false)).unwrap()).unwrap();
    let prop =
        |app: &mut Connection, w: u32, a: u32| req::parse_property(&app.client().round_trip(&req::get_property(false, w, a, 0, 0, 64)).unwrap()).unwrap();
    let check = atom(&mut app, "_NET_SUPPORTING_WM_CHECK");
    let check_win = prop(&mut app, root, check).as_u32s().unwrap()[0];
    let name = atom(&mut app, "_NET_WM_NAME");
    assert_eq!(prop(&mut app, check_win, name).value, b"lintel");
    let list = atom(&mut app, "_NET_CLIENT_LIST");
    assert_eq!(prop(&mut app, root, list).as_u32s().unwrap(), [w.xid()]);
    let active = atom(&mut app, "_NET_ACTIVE_WINDOW");
    assert_eq!(prop(&mut app, root, active).as_u32s().unwrap(), [w.xid()]);
    let extents = atom(&mut app, "_NET_FRAME_EXTENTS");
    assert_eq!(prop(&mut app, w.xid(), extents).as_u32s().unwrap(), [BORDER, BORDER, BORDER + TITLE, BORDER]);
}

#[test]
fn dragging_to_the_edges_tiles_and_dragging_away_floats_again() {
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut app = connect(&x);
    let a = open_app(&mut wm, &mut app, WindowOptions::new(300, 200).title("A"));
    // To the left edge: the left half.
    drag_bar(&mut wm, &mut app, a, (40, 8), (0, 400));
    assert_eq!(wm.client(a.xid()).unwrap().place, Place::Tiled);
    assert_eq!(wm.client(a.xid()).unwrap().rect, Rect::new(0, 0, 512, 768));
    assert_eq!(on_screen(&mut app, a), Rect::new(B, B + T, 512 - 2 * BORDER, 768 - 2 * BORDER - TITLE), "the program was resized too");
    // Another, dropped near the tiled one's right edge... no: its bottom edge.
    let b = open_app(&mut wm, &mut app, WindowOptions::new(300, 200).title("B"));
    drag_bar(&mut wm, &mut app, b, (40, 8), (250, 750));
    assert_eq!(wm.client(a.xid()).unwrap().rect, Rect::new(0, 0, 512, 384), "the tiled one keeps the top half");
    assert_eq!(wm.client(b.xid()).unwrap().rect, Rect::new(0, 384, 512, 384));
    assert_eq!(wm.client(b.xid()).unwrap().place, Place::Tiled);
    // B to the top right corner: a quarter.
    drag_bar(&mut wm, &mut app, b, (40, 8), (1023, 5));
    assert_eq!(wm.client(b.xid()).unwrap().rect, Rect::new(512, 0, 512, 384));
    // A off its tile: floating, at its old size, still under the pointer.
    drag_bar(&mut wm, &mut app, a, (40, 8), (600, 500));
    let ca = wm.client(a.xid()).unwrap();
    assert_eq!(ca.place, Place::Floating);
    assert_eq!((ca.rect.width, ca.rect.height), (300 + 2 * BORDER, 200 + 2 * BORDER + TITLE));
    assert!(ca.rect.contains(600, 500), "under the pointer: {:?}", ca.rect);
    assert_eq!(on_screen(&mut app, a).width, 300);
    // To the top: maximized; a double click on the bar restores it.
    drag_bar(&mut wm, &mut app, a, (40, 8), (500, 0));
    assert_eq!((wm.client(a.xid()).unwrap().place, wm.client(a.xid()).unwrap().rect), (Place::Maximized, SCREEN));
    click_bar(&mut wm, a, (40, 8), 5000);
    click_bar(&mut wm, a, (40, 8), 5200);
    settle(&mut wm, &mut [&mut app]);
    assert_eq!(wm.client(a.xid()).unwrap().place, Place::Floating);
    // The maximize button maximizes.
    let w = wm.client(a.xid()).unwrap().rect.width as i32 - 2 * B;
    click_bar(&mut wm, a, (w - 3 - 14 - 2 - 7, 9), 9000);
    assert_eq!(wm.client(a.xid()).unwrap().place, Place::Maximized);
    // Again (the bar is wider now): restored.
    let w = wm.client(a.xid()).unwrap().rect.width as i32 - 2 * B;
    click_bar(&mut wm, a, (w - 3 - 14 - 2 - 7, 9), 12000);
    assert_eq!(wm.client(a.xid()).unwrap().place, Place::Floating);
    // A plain drop in the middle of the screen moves a floating window, no more.
    let before = wm.client(a.xid()).unwrap().rect;
    drag_bar(&mut wm, &mut app, a, (40, 8), (before.x + B + 40 + 100, before.y + B + 8 + 50));
    let after = wm.client(a.xid()).unwrap().rect;
    assert_eq!((after.x, after.y, after.width), (before.x + 100, before.y + 50, before.width));
    assert_eq!(wm.preview_at, None, "the outline is gone");
}

#[test]
fn the_outline_shows_where_a_drop_will_land() {
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut app = connect(&x);
    let a = open_app(&mut wm, &mut app, WindowOptions::new(300, 200).title("A"));
    let c = wm.client(a.xid()).unwrap().rect;
    let bar = bar_of(&mut wm, a);
    let from = (c.x + B + 40, c.y + B + 8);
    let press = Event::Button {
        window: bar,
        button: MouseButton::Left,
        pressed: true,
        x: 40,
        y: 8,
        root_x: from.0,
        root_y: from.1,
        modifiers: Modifiers::default(),
        time: 1,
    };
    wm.handle(press);
    let motion =
        |p: (i32, i32)| Event::Motion { window: bar, x: 0, y: 0, root_x: p.0, root_y: p.1, buttons: MouseButtons::default(), modifiers: Modifiers::default() };
    wm.handle(motion((from.0 + 50, from.1)));
    assert_eq!(wm.preview_at, None, "in the open: nothing");
    wm.handle(motion((1023, 300)));
    assert_eq!(wm.preview_at, Some(Rect::new(512, 0, 512, 768)));
    settle(&mut wm, &mut [&mut app]);
    // The outline is on the screen, as four bars above everything.
    let root = app.root();
    let reply = app.client().round_trip(&req::query_tree(root)).unwrap();
    let top: Vec<u32> = req::parse_query_tree(&reply).unwrap().children.into_iter().rev().take(4).collect();
    for w in top {
        let reply = app.client().round_trip(&req::get_window_attributes(w)).unwrap();
        assert_eq!(req::parse_window_attributes(&reply).unwrap().map_state, req::MapState::Viewable);
    }
}

#[test]
fn programs_move_and_resize_themselves_only_while_floating() {
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut app = connect(&x);
    let w = open_app(&mut wm, &mut app, WindowOptions::new(200, 100).title("self-sizing"));
    app.set_size(w, 250, 120).unwrap();
    app.set_position(w, 100, 90).unwrap();
    settle(&mut wm, &mut [&mut app]);
    // protolith asks for static gravity: the position is the window's own.
    assert_eq!(on_screen(&mut app, w), Rect::new(100, 90, 250, 120));
    assert_eq!(wm.client(w.xid()).unwrap().rect, Rect::new(100 - B, 90 - B - T, 250 + 2 * BORDER, 120 + 2 * BORDER + TITLE));
    // Tiled: refused (and told where it is).
    drag_bar(&mut wm, &mut app, w, (40, 8), (0, 300));
    let tiled = on_screen(&mut app, w);
    app.set_size(w, 50, 50).unwrap();
    settle(&mut wm, &mut [&mut app]);
    assert_eq!(on_screen(&mut app, w), tiled);
}

#[test]
fn undecorated_windows_get_no_bar_and_go_exactly_where_they_ask() {
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut app = connect(&x);
    // As phonolite's windows are: no decorations, a position.
    let w = open_app(&mut wm, &mut app, WindowOptions::new(275, 116).decorations(false).position(300, 200));
    let frame = wm.client(w.xid()).unwrap().frame;
    let reply = app.client().round_trip(&req::query_tree(frame)).unwrap();
    assert_eq!(req::parse_query_tree(&reply).unwrap().children, [w.xid()], "no bar");
    assert_eq!(on_screen(&mut app, w), Rect::new(300, 200, 275, 116));
    app.set_position(w, 310, 220).unwrap();
    settle(&mut wm, &mut [&mut app]);
    assert_eq!(on_screen(&mut app, w), Rect::new(310, 220, 275, 116));
}

#[test]
fn closing_asks_politely_and_a_withdrawn_window_is_let_go() {
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut app = connect(&x);
    let w = open_app(&mut wm, &mut app, WindowOptions::new(200, 100).title("Bye"));
    // The close button: WM_DELETE_WINDOW, which protolith reports.
    let width = wm.client(w.xid()).unwrap().rect.width as i32 - 2 * B;
    click_bar(&mut wm, w, (width - 3 - 7, 9), 100);
    settle(&mut wm, &mut []);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(Event::CloseRequested { window }) = app.wait_event_timeout(Duration::from_millis(50)).unwrap() {
            assert_eq!(window, w);
            break;
        }
        assert!(Instant::now() < deadline, "no close request");
    }
    // The program hides the window: lintel lets it go, back to the root.
    app.hide(w).unwrap();
    run_until(&mut wm, "unmanaged", |wm| wm.client(w.xid()).is_none());
    settle(&mut wm, &mut [&mut app]);
    let root = app.root();
    assert_eq!(parent_of(&mut app, w.xid()), root);
    // Shown again: managed again.
    app.show(w).unwrap();
    run_until(&mut wm, "managed again", |wm| wm.client(w.xid()).is_some());
    // Destroyed: its frame goes too.
    let frame = wm.client(w.xid()).unwrap().frame;
    app.destroy_window(w).unwrap();
    run_until(&mut wm, "unmanaged after destruction", |wm| wm.client(w.xid()).is_none());
    settle(&mut wm, &mut [&mut app]);
    assert!(app.client().round_trip(&req::get_geometry(frame)).is_err(), "the frame is destroyed");
}

#[test]
fn a_full_screen_request_and_the_keyboard_shortcuts() {
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut app = connect(&x);
    let w = open_app(&mut wm, &mut app, WindowOptions::new(200, 100).title("Video"));
    let atom = |app: &mut Connection, n: &str| req::parse_intern_atom(&app.client().round_trip(&req::intern_atom(n, false)).unwrap()).unwrap();
    let (state, full) = (atom(&mut app, "_NET_WM_STATE"), atom(&mut app, "_NET_WM_STATE_FULLSCREEN"));
    let root = app.root();
    let ask = |app: &mut Connection, action: u32| {
        let m = req::client_message(w.xid(), state, [action, full, 0, 1, 0]);
        app.client().send(&req::send_event(false, root, (1 << 19) | (1 << 20), &m)).unwrap();
        app.client().flush().unwrap();
    };
    ask(&mut app, 1);
    run_until(&mut wm, "full screen", |wm| wm.client(w.xid()).unwrap().place == Place::Fullscreen);
    settle(&mut wm, &mut [&mut app]);
    assert_eq!(on_screen(&mut app, w), SCREEN, "the whole screen, no frame");
    ask(&mut app, 0);
    run_until(&mut wm, "back", |wm| wm.client(w.xid()).unwrap().place == Place::Floating);
    settle(&mut wm, &mut [&mut app]);
    assert_eq!(on_screen(&mut app, w).width, 200);
    // Super+Right: the right half. Super+Down: floating again.
    let key = |wm: &mut Wm, sym: u32, state: u16| {
        let (code, _) = wm.conn.keymap().keycode_for(sym).expect("a keycode");
        let i = InputEvent { detail: code, time: 0, root, event: root, child: 0, root_x: 0, root_y: 0, x: 0, y: 0, state };
        wm.handle(Event::Foreign(Wire { event: RawEvent::KeyPress(i), send_event: false, sequence: None }));
    };
    key(&mut wm, 0xff53, 0x40);
    assert_eq!(wm.client(w.xid()).unwrap().rect, Rect::new(512, 0, 512, 768));
    key(&mut wm, 0xff54, 0x40 | 0x10); // with Num Lock on
    assert_eq!(wm.client(w.xid()).unwrap().place, Place::Floating);
}

#[test]
fn when_lintel_goes_its_windows_stay() {
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut app = connect(&x);
    let w = open_app(&mut wm, &mut app, WindowOptions::new(200, 100).title("Survivor"));
    drop(wm);
    let root = app.root();
    let deadline = Instant::now() + Duration::from_secs(5);
    while parent_of(&mut app, w.xid()) != root {
        assert!(Instant::now() < deadline, "not back on the root");
        std::thread::sleep(Duration::from_millis(20));
    }
    // And a new lintel takes it over again.
    let mut wm = Wm::new(connect(&x)).unwrap();
    assert!(wm.client(w.xid()).is_some(), "adopted");
    settle(&mut wm, &mut [&mut app]);
    assert_eq!(parent_of(&mut app, w.xid()), wm.client(w.xid()).unwrap().frame);
}

/// Real input, through XTEST: `xdotool` with these arguments.
fn xdotool(x: &Xvfb, args: &str) {
    let ok = Command::new("xdotool").env("DISPLAY", &x.display).args(args.split_whitespace()).status().is_ok_and(|s| s.success());
    assert!(ok, "xdotool {args}");
}

fn have_xdotool() -> bool {
    let have = Command::new("xdotool").arg("version").stdout(Stdio::null()).status().is_ok_and(|s| s.success());
    assert!(have || std::env::var_os("LINTEL_REQUIRE_XVFB").is_none(), "xdotool is required");
    have
}

#[test]
fn real_pointer_and_keyboard() {
    let Some(x) = xvfb() else { return };
    if !have_xdotool() {
        return;
    }
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut app = connect(&x);
    let a = open_app(&mut wm, &mut app, WindowOptions::new(300, 200).title("A"));
    let b = open_app(&mut wm, &mut app, WindowOptions::new(300, 200).title("B"));
    assert_eq!(wm.focused, Some(b.xid()));
    // A click in A's content focuses A, and A still gets the click.
    let ra = wm.client(a.xid()).unwrap().inner();
    let rb = wm.client(b.xid()).unwrap().rect;
    let spot = (ra.x + 5, ra.y + ra.height as i32 - 5);
    assert!(!rb.contains(spot.0, spot.1), "a spot of A not under B");
    xdotool(&x, &format!("mousemove {} {} click 1", spot.0, spot.1));
    run_until(&mut wm, "A focused by a click", |wm| wm.focused == Some(a.xid()));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match app.wait_event_timeout(Duration::from_millis(50)).unwrap() {
            Some(Event::Button { window, pressed: true, .. }) => {
                assert_eq!(window, a);
                break;
            }
            _ => assert!(Instant::now() < deadline, "the click was not passed on"),
        }
        wm.step(Some(Duration::from_millis(5))).unwrap();
    }
    // Super+Left: A to the left half.
    xdotool(&x, "key super+Left");
    run_until(&mut wm, "A tiled left", |wm| wm.client(a.xid()).unwrap().rect == Rect::new(0, 0, 512, 768));
    // B by its title bar to the right edge.
    // (A, tiled and on top, covers B's left part: take B by its right.)
    let rb = wm.client(b.xid()).unwrap().rect;
    let grab = (rb.x + rb.width as i32 - 70, rb.y + 10);
    assert!(grab.0 >= 512, "on B's visible part");
    xdotool(&x, &format!("mousemove {} {} mousedown 1", grab.0, grab.1));
    for k in 1..=4 {
        xdotool(&x, &format!("mousemove {} {}", grab.0 + (1023 - grab.0) * k / 4, 300));
    }
    xdotool(&x, "mouseup 1");
    run_until(&mut wm, "B tiled right", |wm| wm.client(b.xid()).unwrap().rect == Rect::new(512, 0, 512, 768));
    // Super+Down floats A again; its bottom right corner resizes it.
    xdotool(&x, &format!("mousemove {} {} click 1", 100, 100));
    run_until(&mut wm, "A focused", |wm| wm.focused == Some(a.xid()));
    xdotool(&x, "key super+Down");
    run_until(&mut wm, "A floating", |wm| wm.client(a.xid()).unwrap().place == Place::Floating);
    let r = wm.client(a.xid()).unwrap().rect;
    let corner = (r.x + r.width as i32 - 1, r.y + r.height as i32 - 1);
    xdotool(&x, &format!("mousemove {} {} mousedown 1 mousemove {} {} mouseup 1", corner.0, corner.1, corner.0 + 50, corner.1 + 30));
    run_until(&mut wm, "A resized", |wm| wm.client(a.xid()).unwrap().rect.width == r.width + 50);
    settle(&mut wm, &mut [&mut app]);
    assert_eq!(on_screen(&mut app, a).height, 200 + 30);
    // Alt+Tab: the other window.
    xdotool(&x, "key alt+Tab");
    run_until(&mut wm, "B focused by Alt+Tab", |wm| wm.focused == Some(b.xid()));
}

#[test]
fn a_moved_window_is_told_where_it_is_on_the_screen() {
    // ICCCM 4.1.5: moved without being resized, a framed window hears of
    // it only from the window manager — a synthetic ConfigureNotify in
    // screen coordinates. Toolkits place their popups by it.
    let Some(x) = xvfb() else { return };
    let mut wm = Wm::new(connect(&x)).unwrap();
    let mut raw = protolith::x11::Client::connect(&protolith::x11::DisplayName::parse(&x.display).unwrap(), None).unwrap();
    let win = raw.generate_id().unwrap();
    let root = raw.screen().root;
    let attrs = req::WindowAttributes { event_mask: Some(1 << 17), ..Default::default() }; // StructureNotify
    let cw = req::CreateWindow {
        wid: win,
        parent: root,
        depth: 0,
        x: 0,
        y: 0,
        width: 120,
        height: 80,
        border_width: 0,
        class: req::WindowClass::InputOutput,
        visual: 0,
        attributes: attrs,
    };
    raw.send(&req::create_window(&cw)).unwrap();
    raw.send(&req::map_window(win)).unwrap();
    raw.flush().unwrap();
    run_until(&mut wm, "managed", |wm| wm.client(win).is_some());
    let w = WindowId::from_xid(win);
    let mut dummy = connect(&x);
    drag_bar(&mut wm, &mut dummy, w, (40, 8), (600, 400));
    let inner = wm.client(win).unwrap().inner();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "no synthetic ConfigureNotify at {inner:?}");
        let Some(protolith::x11::Message::Event(e)) = raw.wait_message(Some(Duration::from_millis(50))).unwrap() else { continue };
        if let RawEvent::ConfigureNotify { window, x, y, width, height } = e.event {
            if e.send_event && window == win && (i32::from(x), i32::from(y)) == (inner.x, inner.y) {
                assert_eq!((u32::from(width), u32::from(height)), (120, 80));
                break;
            }
        }
    }
}
