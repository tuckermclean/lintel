//! The window manager: frames, focus, dragging and dropping, the hints
//! programs give and expect.
//!
//! Each managed window is reparented into a *frame*, a bare X window whose
//! background paints the border (no client-side memory for it), holding
//! the *lintel*: the title bar, a protolith window a few pixels tall, with
//! the title and the maximize and close buttons. Undecorated windows (the
//! Motif hint, as phonolite asks) get a frame with no border or bar.
//!
//! New windows float. Dragged by the bar to the left or right edge of the
//! screen a window takes that half, to a corner that quarter, to the top
//! the whole screen ([`layout`]); dropped near the edge of a tiled window,
//! the two share its place. Dragged off a tile, it floats again at its
//! old size. An outline shows where a drop will land.

use std::collections::HashSet;
use std::time::Duration;

use protolith::canvas::Canvas;
use protolith::font::{Font, Terminus};
use protolith::x11::proto::event::{InputEvent, RawEvent};
use protolith::x11::proto::request::{
    self as req, event_mask, Allow, ConfigureWindow, PropData, PropMode, StackMode, WindowAttributes, WindowClass, ANY_MODIFIER,
};
use protolith::x11::proto::{atom, Atom, Xid};
use protolith::x11::{Connection, Event, MouseButton, WindowId, WindowOptions};
use protolith::{Error, Rect, Result};

use crate::layout::{self, Side, Zone};

/// Border width, in pixels.
pub const BORDER: u32 = 3;
/// Title bar height.
pub const TITLE: u32 = 18;
const BUTTON: i32 = 14;

const BORDER_COLOR: u32 = 0x1c2024;
const BAR_ACTIVE: u32 = 0x2f5574;
const BAR_INACTIVE: u32 = 0x3a3d40;
const TEXT_ACTIVE: u32 = 0xffffff;
const TEXT_INACTIVE: u32 = 0x9aa0a6;
const PREVIEW: u32 = 0x6cb4e4;
const PREVIEW_WIDTH: u32 = 3;

const BUTTON_MOTION: u32 = 1 << 13;
const SHIFT: u16 = 1;
const LOCK: u16 = 2;
const CONTROL: u16 = 4;
const ALT: u16 = 8;
const SUPER: u16 = 0x40;

const ATOM_NAMES: [&str; 27] = [
    "WM_PROTOCOLS",
    "WM_DELETE_WINDOW",
    "WM_TAKE_FOCUS",
    "WM_STATE",
    "WM_CHANGE_STATE",
    "UTF8_STRING",
    "_NET_SUPPORTED",
    "_NET_SUPPORTING_WM_CHECK",
    "_NET_WM_NAME",
    "_NET_CLIENT_LIST",
    "_NET_ACTIVE_WINDOW",
    "_NET_CLOSE_WINDOW",
    "_NET_FRAME_EXTENTS",
    "_NET_WM_STATE",
    "_NET_WM_STATE_FULLSCREEN",
    "_NET_WM_STATE_ABOVE",
    "_NET_WM_STATE_MAXIMIZED_VERT",
    "_NET_WM_STATE_MAXIMIZED_HORZ",
    "_NET_WM_STATE_HIDDEN",
    "_NET_WM_WINDOW_TYPE",
    "_NET_WM_WINDOW_TYPE_DIALOG",
    "_NET_WM_WINDOW_TYPE_UTILITY",
    "_NET_WM_WINDOW_TYPE_SPLASH",
    "_NET_WM_WINDOW_TYPE_DOCK",
    "_NET_WM_WINDOW_TYPE_DESKTOP",
    "_MOTIF_WM_HINTS",
    "WM_HINTS",
];

/// The atoms lintel speaks, in [`ATOM_NAMES`] order.
#[derive(Debug, Clone, Copy)]
struct Atoms([Atom; 27]);

impl Atoms {
    fn get(&self, name: &str) -> Atom {
        self.0[ATOM_NAMES.iter().position(|n| *n == name).expect("a known atom")]
    }
}

/// How a window is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Where it was put or dragged.
    Floating,
    /// In a tile (a half, a quarter, a share of one).
    Tiled,
    /// The whole screen, with its frame.
    Maximized,
    /// The whole screen, nothing else.
    Fullscreen,
}

/// A window under management.
#[derive(Debug)]
pub struct Client {
    /// The program's window.
    pub win: Xid,
    /// The frame lintel put it in.
    pub frame: Xid,
    bar: Option<WindowId>,
    canvas: Canvas,
    /// Placement.
    pub place: Place,
    /// The frame's place on the screen.
    pub rect: Rect,
    /// Where it floats when not tiled.
    pub float: Rect,
    before_fullscreen: (Place, Rect),
    /// The title shown.
    pub title: String,
    delete: bool,
    take_focus: bool,
    input: bool,
    static_gravity: bool,
    /// Its size is fixed (min = max): never tiled, never resized.
    pub fixed: bool,
    /// Kept above the others (`_NET_WM_STATE_ABOVE`).
    pub above: bool,
    /// Minimized.
    pub iconic: bool,
    border_width: u16,
    ignore_unmaps: u32,
}

impl Client {
    fn decorated(&self) -> bool {
        self.bar.is_some() && self.place != Place::Fullscreen
    }

    /// Border and title widths around the program's window.
    fn insets(&self) -> (u32, u32) {
        if self.decorated() {
            (BORDER, TITLE)
        } else {
            (0, 0)
        }
    }

    /// The program's window in screen coordinates.
    pub fn inner(&self) -> Rect {
        let (b, t) = self.insets();
        let r = self.rect;
        Rect::new(r.x + b as i32, r.y + (b + t) as i32, r.width.saturating_sub(2 * b).max(1), r.height.saturating_sub(2 * b + t).max(1))
    }
}

/// Where a drag would drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop {
    /// A zone of the screen.
    Screen(Zone),
    /// Half of a tiled window's place.
    Split(Xid, Side),
}

#[derive(Debug, Clone, Copy)]
enum DragKind {
    Move { frac: u32, grab_y: i32, drop: Option<Drop> },
    Resize { left: bool, right: bool, top: bool, bottom: bool },
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    win: Xid,
    kind: DragKind,
    from: (i32, i32),
    start: Rect,
    moved: bool,
}

/// A keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hotkey {
    /// Super+Left: the left half.
    Left,
    /// Super+Right: the right half.
    Right,
    /// Super+Up: maximize.
    Up,
    /// Super+Down: back to floating, or minimize a floating window.
    Down,
    /// Alt+F4: close.
    Close,
    /// Alt+Tab: the next window.
    Next,
}

const HOTKEYS: [(u32, u16, Hotkey); 6] = [
    (0xff51, SUPER, Hotkey::Left),
    (0xff53, SUPER, Hotkey::Right),
    (0xff52, SUPER, Hotkey::Up),
    (0xff54, SUPER, Hotkey::Down),
    (0xffc1, ALT, Hotkey::Close),
    (0xff09, ALT, Hotkey::Next),
];

/// The window manager.
pub struct Wm {
    /// The display connection.
    pub conn: Connection,
    root: Xid,
    screen: Rect,
    atoms: Atoms,
    font: Terminus,
    /// Managed windows, bottom of the stack first.
    pub clients: Vec<Client>,
    /// The window with the keyboard.
    pub focused: Option<Xid>,
    drag: Option<Drag>,
    pending_button: Option<(Xid, u8)>,
    last_press: Option<(Xid, u32)>,
    preview: [Xid; 4],
    /// Where the drop outline is shown, if it is.
    pub preview_at: Option<Rect>,
    check: Xid,
    dying: HashSet<Xid>,
    keys: Vec<(u8, u16, Hotkey)>,
    cascade: i32,
}

fn coord(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn dim(v: u32) -> u16 {
    v.clamp(1, 32767) as u16
}

impl Wm {
    /// Take over the display: fails if another window manager runs.
    pub fn new(mut conn: Connection) -> Result<Wm> {
        conn.become_window_manager().map_err(|e| match e {
            Error::X(x) if x.code == 10 => Error::InvalidArgument("another window manager is running".into()),
            e => e,
        })?;
        let root = conn.root();
        let (sw, sh) = conn.screen_size();
        let c = conn.client();
        let cookies = ATOM_NAMES.iter().map(|n| c.send_with_reply(&req::intern_atom(n, false))).collect::<Result<Vec<_>>>()?;
        let mut atoms = [0; 27];
        for (slot, cookie) in atoms.iter_mut().zip(cookies) {
            *slot = req::parse_intern_atom(&c.wait_reply(cookie)?).unwrap_or(0);
        }
        let atoms = Atoms(atoms);
        let mut wm = Wm {
            conn,
            root,
            screen: Rect::new(0, 0, sw, sh),
            atoms,
            font: Terminus::REGULAR_12,
            clients: Vec::new(),
            focused: None,
            drag: None,
            pending_button: None,
            last_press: None,
            preview: [0; 4],
            preview_at: None,
            check: 0,
            dying: HashSet::new(),
            keys: Vec::new(),
            cascade: 0,
        };
        wm.check = wm.bare_window(root, Rect::new(-1, -1, 1, 1), BORDER_COLOR, 0, true)?;
        for i in 0..4 {
            wm.preview[i] = wm.bare_window(root, Rect::new(0, 0, 1, 1), PREVIEW, 0, true)?;
        }
        wm.advertise()?;
        wm.grab_keys()?;
        wm.adopt()?;
        wm.conn.client().flush()?;
        Ok(wm)
    }

    fn a(&self, name: &str) -> Atom {
        self.atoms.get(name)
    }

    fn send(&mut self, r: &[u8]) {
        let _ = self.conn.client().send(r);
    }

    fn pixel(&self, rgb: u32) -> u32 {
        self.conn.pixel_format().pack(rgb)
    }

    /// A plain window painted by the server in one color.
    fn bare_window(&mut self, parent: Xid, r: Rect, color: u32, mask: u32, override_redirect: bool) -> Result<Xid> {
        let wid = self.conn.client().generate_id()?;
        let pixel = self.pixel(color);
        let cw = req::CreateWindow {
            wid,
            parent,
            depth: 0,
            x: coord(r.x),
            y: coord(r.y),
            width: dim(r.width),
            height: dim(r.height),
            border_width: 0,
            class: WindowClass::InputOutput,
            visual: 0,
            attributes: WindowAttributes {
                background_pixel: Some(pixel),
                override_redirect: override_redirect.then_some(true),
                event_mask: (mask != 0).then_some(mask),
                ..Default::default()
            },
        };
        self.conn.client().send(&req::create_window(&cw))?;
        Ok(wid)
    }

    /// EWMH: who we are and what we support.
    fn advertise(&mut self) -> Result<()> {
        let (check, root) = (self.check, self.root);
        let wm_check = self.a("_NET_SUPPORTING_WM_CHECK");
        for w in [check, root] {
            self.send(&req::change_property(PropMode::Replace, w, wm_check, atom::WINDOW, PropData::U32(&[check])));
        }
        let (name, utf8) = (self.a("_NET_WM_NAME"), self.a("UTF8_STRING"));
        self.send(&req::change_property(PropMode::Replace, check, name, utf8, PropData::U8(b"lintel")));
        let supported: Vec<Atom> = ATOM_NAMES.iter().filter(|n| n.starts_with("_NET_") && **n != "_NET_SUPPORTED").map(|n| self.a(n)).collect();
        let net_supported = self.a("_NET_SUPPORTED");
        self.send(&req::change_property(PropMode::Replace, root, net_supported, atom::ATOM, PropData::U32(&supported)));
        self.update_client_list();
        Ok(())
    }

    fn grab_keys(&mut self) -> Result<()> {
        let num = self.conn.keymap().num_lock_mask();
        for (sym, mods, key) in HOTKEYS {
            let Some((code, _)) = self.conn.keymap().keycode_for(sym) else { continue };
            for extra in [0, LOCK, num, LOCK | num] {
                let r = req::grab_key(true, self.root, mods | extra, code);
                self.send(&r);
            }
            self.keys.push((code, mods, key));
        }
        Ok(())
    }

    /// Manage the windows already on the screen (lintel started late, or
    /// restarted).
    fn adopt(&mut self) -> Result<()> {
        let reply = self.conn.client().round_trip(&req::query_tree(self.root))?;
        let Some(tree) = req::parse_query_tree(&reply) else { return Ok(()) };
        for w in tree.children {
            if w == self.check || self.preview.contains(&w) {
                continue;
            }
            let Ok(reply) = self.conn.client().round_trip(&req::get_window_attributes(w)) else { continue };
            let Some(attrs) = req::parse_window_attributes(&reply) else { continue };
            if !attrs.override_redirect && attrs.map_state == req::MapState::Viewable {
                self.manage(w, true);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ properties

    fn property(&mut self, w: Xid, prop: Atom, len: u32) -> Option<req::Property> {
        let reply = self.conn.client().round_trip(&req::get_property(false, w, prop, 0, 0, len)).ok()?;
        req::parse_property(&reply).filter(|p| p.format != 0)
    }

    fn u32s(&mut self, w: Xid, prop: Atom) -> Vec<u32> {
        self.property(w, prop, 64).and_then(|p| p.as_u32s()).unwrap_or_default()
    }

    fn title_of(&mut self, w: Xid) -> String {
        let net = self.a("_NET_WM_NAME");
        if let Some(p) = self.property(w, net, 256) {
            return String::from_utf8_lossy(&p.value).into_owned();
        }
        // WM_NAME is Latin-1 (STRING) or close enough.
        self.property(w, atom::WM_NAME, 256).map(|p| p.value.iter().map(|&b| char::from(b)).collect()).unwrap_or_default()
    }

    // ------------------------------------------------------------ managing

    fn index(&self, w: Xid) -> Option<usize> {
        self.clients.iter().position(|c| c.win == w)
    }

    fn by_frame(&self, f: Xid) -> Option<usize> {
        self.clients.iter().position(|c| c.frame == f)
    }

    fn by_bar(&self, b: WindowId) -> Option<usize> {
        self.clients.iter().position(|c| c.bar == Some(b))
    }

    /// Frame `w` and show it. `adopting`: it was already on the screen.
    fn manage(&mut self, w: Xid, adopting: bool) {
        let Ok(reply) = self.conn.client().round_trip(&req::get_window_attributes(w)) else { return };
        let Some(attrs) = req::parse_window_attributes(&reply) else { return };
        if attrs.override_redirect {
            self.send(&req::map_window(w));
            return;
        }
        let Ok(reply) = self.conn.client().round_trip(&req::get_geometry(w)) else { return };
        let Some(g) = req::parse_geometry(&reply) else { return };
        let types = self.u32s(w, self.a("_NET_WM_WINDOW_TYPE"));
        if types.iter().any(|t| *t == self.a("_NET_WM_WINDOW_TYPE_DOCK") || *t == self.a("_NET_WM_WINDOW_TYPE_DESKTOP")) {
            // Panels and desktops place themselves.
            self.send(&req::map_window(w));
            return;
        }
        let protocols = self.u32s(w, self.a("WM_PROTOCOLS"));
        let hints = self.u32s(w, self.a("WM_HINTS"));
        let normal = self.u32s(w, atom::WM_NORMAL_HINTS);
        let motif = self.u32s(w, self.a("_MOTIF_WM_HINTS"));
        let transient = self.u32s(w, atom::WM_TRANSIENT_FOR).first().copied().unwrap_or(0);
        let states = self.u32s(w, self.a("_NET_WM_STATE"));
        let title = self.title_of(w);
        let flags = normal.first().copied().unwrap_or(0);
        let fixed = flags & 16 != 0 && flags & 32 != 0 && normal.get(5) == normal.get(7) && normal.get(6) == normal.get(8);
        let placed = adopting || flags & 5 != 0;
        let static_gravity = flags & 512 != 0 && normal.get(17) == Some(&10);
        let undecorated = motif.first().is_some_and(|f| f & 2 != 0) && motif.get(2) == Some(&0);
        let decorated = !undecorated;
        let (b, t) = if decorated { (BORDER, TITLE) } else { (0, 0) };
        let (cw, ch) = (u32::from(g.width), u32::from(g.height));
        let (fw, fh) = (cw + 2 * b, ch + 2 * b + t);
        let (x, y) = if placed {
            if static_gravity || adopting {
                (i32::from(g.x) - b as i32, i32::from(g.y) - (b + t) as i32)
            } else {
                (i32::from(g.x), i32::from(g.y))
            }
        } else if let Some(p) = self.index(transient) {
            // A dialog: over the middle of its owner.
            let o = self.clients[p].rect;
            (o.x + (o.width as i32 - fw as i32) / 2, o.y + (o.height as i32 - fh as i32) / 2)
        } else {
            self.cascade = (self.cascade + 1) % 8;
            let off = 24 * self.cascade;
            ((self.screen.width as i32 - fw as i32) / 2 + off - 96, (self.screen.height as i32 - fh as i32) / 2 + off - 96)
        };
        // Keep the title bar reachable.
        let (x, y) = (x.clamp(-(fw as i32) + 64, self.screen.width as i32 - 64), y.clamp(0, (self.screen.height as i32 - TITLE as i32).max(0)));
        let rect = Rect::new(x, y, fw, fh);
        let mask = event_mask::SUBSTRUCTURE_REDIRECT | event_mask::SUBSTRUCTURE_NOTIFY | event_mask::BUTTON_PRESS | event_mask::BUTTON_RELEASE | BUTTON_MOTION;
        let Ok(frame) = self.bare_window(self.root, rect, BORDER_COLOR, mask, false) else { return };
        let bar = if decorated {
            self.conn.create_window(WindowOptions::new(fw - 2 * b, TITLE).parent(frame).position(b as i32, b as i32).decorations(false).resizable(false)).ok()
        } else {
            None
        };
        for r in [
            req::change_save_set(true, w),
            req::change_window_attributes(w, &WindowAttributes { event_mask: Some(event_mask::PROPERTY_CHANGE), ..Default::default() }),
            req::configure_window(w, &ConfigureWindow { border_width: Some(0), ..Default::default() }),
            req::reparent_window(w, frame, b as i16, (b + t) as i16),
            req::map_window(w),
            req::map_window(frame),
        ] {
            self.send(&r);
        }
        let wm_state = self.a("WM_STATE");
        self.send(&req::change_property(PropMode::Replace, w, wm_state, wm_state, PropData::U32(&[1, 0])));
        let mut c = Client {
            win: w,
            frame,
            bar,
            canvas: Canvas::new(1, 1),
            place: Place::Floating,
            rect,
            float: rect,
            before_fullscreen: (Place::Floating, rect),
            title,
            delete: protocols.contains(&self.a("WM_DELETE_WINDOW")),
            take_focus: protocols.contains(&self.a("WM_TAKE_FOCUS")),
            input: hints.first().map_or(true, |f| f & 1 == 0 || hints.get(1) != Some(&0)),
            static_gravity,
            fixed,
            above: states.contains(&self.a("_NET_WM_STATE_ABOVE")),
            iconic: false,
            border_width: g.border_width,
            // Reparenting a window on the screen unmaps it first.
            ignore_unmaps: u32::from(adopting),
        };
        if states.contains(&self.a("_NET_WM_STATE_FULLSCREEN")) {
            c.before_fullscreen = (Place::Floating, rect);
            c.place = Place::Fullscreen;
            c.rect = self.screen;
        }
        self.clients.push(c);
        let i = self.clients.len() - 1;
        self.grab_clicks(w);
        self.apply(i);
        self.update_client_list();
        self.focus(Some(w));
        self.raise(i);
    }

    /// Stop managing window `i`. `gone`: it was destroyed (else it
    /// withdrew, and goes back to the root where it is).
    fn unmanage(&mut self, i: usize, gone: bool) {
        let c = self.clients.remove(i);
        if let Some(bar) = c.bar {
            let _ = self.conn.destroy_window(bar);
        }
        if !gone {
            let r = c.inner();
            let wm_state = self.a("WM_STATE");
            for q in [
                req::ungrab_button(0, c.win, ANY_MODIFIER),
                req::reparent_window(c.win, self.root, coord(r.x), coord(r.y)),
                req::configure_window(c.win, &ConfigureWindow { border_width: Some(c.border_width), ..Default::default() }),
                req::change_save_set(false, c.win),
                req::change_property(PropMode::Replace, c.win, wm_state, wm_state, PropData::U32(&[0, 0])),
            ] {
                self.send(&q);
            }
        }
        self.send(&req::destroy_window(c.frame));
        self.dying.insert(c.frame);
        if self.drag.is_some_and(|d| d.win == c.win) {
            self.drag = None;
            self.show_preview(None);
        }
        self.update_client_list();
        if self.focused == Some(c.win) {
            self.focused = None;
            let next = self.clients.iter().rev().find(|c| !c.iconic).map(|c| c.win);
            self.focus(next);
        }
    }

    fn update_client_list(&mut self) {
        let list: Vec<Xid> = self.clients.iter().map(|c| c.win).collect();
        let (root, prop) = (self.root, self.a("_NET_CLIENT_LIST"));
        self.send(&req::change_property(PropMode::Replace, root, prop, atom::WINDOW, PropData::U32(&list)));
    }

    /// Put window `i`'s frame, program window and bar where `rect` says,
    /// and tell the program where it is.
    fn apply(&mut self, i: usize) {
        let c = &self.clients[i];
        let (b, t) = c.insets();
        let (r, inner, win, frame, bar) = (c.rect, c.inner(), c.win, c.frame, c.bar);
        let decorated = c.decorated();
        self.send(&req::configure_window(
            frame,
            &ConfigureWindow { x: Some(coord(r.x)), y: Some(coord(r.y)), width: Some(dim(r.width)), height: Some(dim(r.height)), ..Default::default() },
        ));
        self.send(&req::configure_window(
            win,
            &ConfigureWindow {
                x: Some(b as i16),
                y: Some((b + t) as i16),
                width: Some(dim(inner.width)),
                height: Some(dim(inner.height)),
                ..Default::default()
            },
        ));
        // ICCCM 4.1.5: where it is on the screen, whatever moved.
        self.send(&req::send_event(
            false,
            win,
            event_mask::STRUCTURE_NOTIFY,
            &req::configure_notify(win, win, coord(inner.x), coord(inner.y), dim(inner.width), dim(inner.height)),
        ));
        let extents = self.a("_NET_FRAME_EXTENTS");
        self.send(&req::change_property(PropMode::Replace, win, extents, atom::CARDINAL, PropData::U32(&[b, b, b + t, b])));
        if let Some(bar) = bar {
            if decorated {
                let _ = self.conn.set_size(bar, r.width.saturating_sub(2 * b).max(1), TITLE);
                let _ = self.conn.show(bar);
                self.draw_bar(i);
            } else {
                let _ = self.conn.hide(bar);
            }
        }
        self.publish_state(i);
    }

    fn publish_state(&mut self, i: usize) {
        let c = &self.clients[i];
        let mut s = Vec::new();
        match c.place {
            Place::Fullscreen => s.push(self.a("_NET_WM_STATE_FULLSCREEN")),
            Place::Maximized => s.extend([self.a("_NET_WM_STATE_MAXIMIZED_VERT"), self.a("_NET_WM_STATE_MAXIMIZED_HORZ")]),
            _ => {}
        }
        if c.above {
            s.push(self.a("_NET_WM_STATE_ABOVE"));
        }
        if c.iconic {
            s.push(self.a("_NET_WM_STATE_HIDDEN"));
        }
        let (win, prop) = (c.win, self.a("_NET_WM_STATE"));
        self.send(&req::change_property(PropMode::Replace, win, prop, atom::ATOM, PropData::U32(&s)));
    }

    // ------------------------------------------------------------ drawing

    fn draw_bar(&mut self, i: usize) {
        let active = self.focused == Some(self.clients[i].win);
        let c = &mut self.clients[i];
        let Some(bar) = c.bar else { return };
        let w = c.rect.width.saturating_sub(2 * BORDER).max(1);
        if c.canvas.width() != w || c.canvas.height() != TITLE {
            c.canvas = Canvas::new(w, TITLE);
        }
        let (bg, fg) = if active { (BAR_ACTIVE, TEXT_ACTIVE) } else { (BAR_INACTIVE, TEXT_INACTIVE) };
        c.canvas.fill(bg);
        // The title, cut to fit before the buttons.
        let room = w as i32 - 8 - 2 * (BUTTON + 4);
        let mut title = c.title.clone();
        if self.font.width(&title) as i32 > room {
            while !title.is_empty() && self.font.width(&title) as i32 + self.font.width("...") as i32 > room {
                title.pop();
            }
            title.push_str("...");
        }
        self.font.draw(&mut c.canvas, &title, 6, 3, fg);
        let [max, close] = buttons(w);
        // Maximize: a box. Close: a cross.
        let m = Rect::new(max.x + 3, max.y + 3, max.width - 6, max.height - 6);
        for r in [
            Rect::new(m.x, m.y, m.width, 2),
            Rect::new(m.x, m.y + m.height as i32 - 1, m.width, 1),
            Rect::new(m.x, m.y, 1, m.height),
            Rect::new(m.x + m.width as i32 - 1, m.y, 1, m.height),
        ] {
            c.canvas.fill_rect(r, fg);
        }
        for d in 3..BUTTON - 3 {
            for (x, y) in [(close.x + d, close.y + d), (close.x + BUTTON - 1 - d, close.y + d)] {
                c.canvas.fill_rect(Rect::new(x, y, 2, 1), fg);
            }
        }
        let _ = self.conn.present(bar, &c.canvas.frame());
    }

    fn show_preview(&mut self, r: Option<Rect>) {
        if r == self.preview_at {
            return;
        }
        self.preview_at = r;
        let Some(r) = r else {
            for p in self.preview {
                self.send(&req::unmap_window(p));
            }
            return;
        };
        let t = PREVIEW_WIDTH;
        let sides = [
            Rect::new(r.x, r.y, r.width, t),
            Rect::new(r.x, r.y + r.height as i32 - t as i32, r.width, t),
            Rect::new(r.x, r.y, t, r.height),
            Rect::new(r.x + r.width as i32 - t as i32, r.y, t, r.height),
        ];
        for (p, s) in self.preview.into_iter().zip(sides) {
            let cfg = ConfigureWindow {
                x: Some(coord(s.x)),
                y: Some(coord(s.y)),
                width: Some(dim(s.width)),
                height: Some(dim(s.height)),
                stack_mode: Some(StackMode::Above),
                ..Default::default()
            };
            self.send(&req::configure_window(p, &cfg));
            self.send(&req::map_window(p));
        }
    }

    // ------------------------------------------------------------ focus

    /// Grab clicks on an unfocused window, to focus it on the first.
    fn grab_clicks(&mut self, w: Xid) {
        self.send(&req::grab_button(false, w, (event_mask::BUTTON_PRESS) as u16, true, 0, ANY_MODIFIER));
    }

    /// Give `w` the keyboard (`None`: nobody).
    pub fn focus(&mut self, w: Option<Xid>) {
        let old = self.focused;
        self.focused = w;
        if let Some(o) = old.filter(|o| Some(*o) != w) {
            if let Some(i) = self.index(o) {
                self.grab_clicks(o);
                self.draw_bar(i);
            }
        }
        let active = self.a("_NET_ACTIVE_WINDOW");
        let root = self.root;
        let Some(i) = w.and_then(|w| self.index(w)) else {
            self.send(&req::set_input_focus(root, req::RevertTo::PointerRoot, 0));
            self.send(&req::change_property(PropMode::Replace, root, active, atom::WINDOW, PropData::U32(&[0])));
            return;
        };
        let (win, input, take) = (self.clients[i].win, self.clients[i].input, self.clients[i].take_focus);
        self.send(&req::ungrab_button(0, win, ANY_MODIFIER));
        if input || !take {
            self.send(&req::set_input_focus(win, req::RevertTo::PointerRoot, 0));
        }
        if take {
            let (protocols, take_focus) = (self.a("WM_PROTOCOLS"), self.a("WM_TAKE_FOCUS"));
            self.send(&req::send_event(false, win, 0, &req::client_message(win, protocols, [take_focus, 0, 0, 0, 0])));
        }
        self.send(&req::change_property(PropMode::Replace, root, active, atom::WINDOW, PropData::U32(&[win])));
        self.draw_bar(i);
    }

    /// Bring window `i` to the top (under the ones kept above).
    fn raise(&mut self, i: usize) {
        let c = self.clients.remove(i);
        let frame = c.frame;
        self.clients.push(c);
        self.send(&req::configure_window(frame, &ConfigureWindow { stack_mode: Some(StackMode::Above), ..Default::default() }));
        let above: Vec<Xid> = self.clients.iter().filter(|c| c.above && c.frame != frame).map(|c| c.frame).collect();
        for f in above {
            self.send(&req::configure_window(f, &ConfigureWindow { stack_mode: Some(StackMode::Above), ..Default::default() }));
        }
    }

    fn activate(&mut self, w: Xid) {
        let Some(i) = self.index(w) else { return };
        if self.clients[i].iconic {
            self.deiconify(i);
        }
        let i = self.index(w).expect("still managed");
        self.raise(i);
        self.focus(Some(w));
    }

    // ------------------------------------------------------------ placing

    /// Put window `i` in `place` at `rect`.
    fn place(&mut self, i: usize, place: Place, rect: Rect) {
        let c = &mut self.clients[i];
        if c.fixed && matches!(place, Place::Tiled | Place::Maximized) {
            return; // a fixed-size window does not stretch
        }
        if c.place == Place::Floating && place != Place::Floating {
            c.float = c.rect;
        }
        c.place = place;
        c.rect = rect;
        self.apply(i);
    }

    fn unfloat_to(&mut self, i: usize, zone: Zone) {
        let r = layout::zone_rect(zone, self.screen);
        let place = if zone == Zone::Maximize { Place::Maximized } else { Place::Tiled };
        self.place(i, place, r);
    }

    fn restore(&mut self, i: usize) {
        let f = self.clients[i].float;
        self.place(i, Place::Floating, f);
    }

    fn toggle_maximize(&mut self, i: usize) {
        if self.clients[i].place == Place::Maximized {
            self.restore(i);
        } else {
            self.unfloat_to(i, Zone::Maximize);
        }
    }

    fn set_fullscreen(&mut self, i: usize, on: bool) {
        let c = &mut self.clients[i];
        if on && c.place != Place::Fullscreen {
            c.before_fullscreen = (c.place, c.rect);
            c.place = Place::Fullscreen;
            c.rect = self.screen;
            self.apply(i);
            self.raise(i);
        } else if !on && c.place == Place::Fullscreen {
            let (p, r) = c.before_fullscreen;
            c.place = p;
            c.rect = r;
            self.apply(i);
        }
    }

    fn iconify(&mut self, i: usize) {
        let c = &mut self.clients[i];
        if c.iconic {
            return;
        }
        c.iconic = true;
        c.ignore_unmaps += 1;
        let (w, f) = (c.win, c.frame);
        self.send(&req::unmap_window(w));
        self.send(&req::unmap_window(f));
        let wm_state = self.a("WM_STATE");
        self.send(&req::change_property(PropMode::Replace, w, wm_state, wm_state, PropData::U32(&[3, 0])));
        self.publish_state(i);
        if self.focused == Some(w) {
            let next = self.clients.iter().rev().find(|c| !c.iconic).map(|c| c.win);
            self.focus(next);
        }
    }

    fn deiconify(&mut self, i: usize) {
        let c = &mut self.clients[i];
        c.iconic = false;
        let (w, f) = (c.win, c.frame);
        self.send(&req::map_window(w));
        self.send(&req::map_window(f));
        let wm_state = self.a("WM_STATE");
        self.send(&req::change_property(PropMode::Replace, w, wm_state, wm_state, PropData::U32(&[1, 0])));
        self.publish_state(i);
    }

    /// Ask window `i` to close (or cut its program off, if it cannot be
    /// asked).
    fn close(&mut self, i: usize) {
        let c = &self.clients[i];
        let win = c.win;
        if c.delete {
            let (protocols, delete) = (self.a("WM_PROTOCOLS"), self.a("WM_DELETE_WINDOW"));
            self.send(&req::send_event(false, win, 0, &req::client_message(win, protocols, [delete, 0, 0, 0, 0])));
        } else {
            self.send(&req::kill_client(win));
        }
    }

    /// The screen changed size: tiles keep their share of it.
    fn new_screen(&mut self, w: u32, h: u32) {
        let (from, to) = (self.screen, Rect::new(0, 0, w, h));
        if from == to {
            return;
        }
        self.screen = to;
        for i in 0..self.clients.len() {
            let c = &mut self.clients[i];
            match c.place {
                Place::Tiled => c.rect = layout::rescale(c.rect, from, to),
                Place::Maximized | Place::Fullscreen => c.rect = to,
                Place::Floating => continue,
            }
            self.apply(i);
        }
    }

    // ------------------------------------------------------------ dragging

    /// Where a drag with the pointer at `p` would drop window `win`.
    fn drop_at(&self, win: Xid, p: (i32, i32)) -> Option<Drop> {
        if let Some(z) = layout::edge_zone(p, self.screen) {
            return Some(Drop::Screen(z));
        }
        // The topmost tiled window under the pointer, near one of its edges.
        let target = self.clients.iter().rev().filter(|c| c.win != win && !c.iconic && c.rect.contains(p.0, p.1)).find(|_| true)?;
        if !matches!(target.place, Place::Tiled | Place::Maximized) {
            return None;
        }
        layout::split_side(p, target.rect).map(|s| Drop::Split(target.win, s))
    }

    fn drop_rect(&self, d: Drop) -> Option<Rect> {
        match d {
            Drop::Screen(z) => Some(layout::zone_rect(z, self.screen)),
            Drop::Split(t, side) => self.index(t).map(|i| layout::half(self.clients[i].rect, side)),
        }
    }

    fn drag_to(&mut self, p: (i32, i32)) {
        let Some(mut d) = self.drag else { return };
        let Some(i) = self.index(d.win) else { return };
        let (dx, dy) = (p.0 - d.from.0, p.1 - d.from.1);
        if !d.moved && dx.abs() + dy.abs() < 4 {
            return;
        }
        d.moved = true;
        match d.kind {
            DragKind::Move { frac, grab_y, .. } => {
                if matches!(self.clients[i].place, Place::Tiled | Place::Maximized) {
                    // Off the tile: float again at the old size, still under the pointer.
                    let f = self.clients[i].float;
                    let r = layout::unsnap((f.width, f.height), p, frac, grab_y);
                    d.start = Rect::new(r.x - dx, r.y - dy, r.width, r.height);
                    self.clients[i].place = Place::Floating;
                } else if self.clients[i].place == Place::Fullscreen {
                    return;
                }
                let r = Rect::new(d.start.x + dx, d.start.y + dy, d.start.width, d.start.height);
                let resized = r.width != self.clients[i].rect.width || r.height != self.clients[i].rect.height;
                self.clients[i].rect = r;
                if resized {
                    self.apply(i);
                } else {
                    let f = self.clients[i].frame;
                    self.send(&req::configure_window(f, &ConfigureWindow { x: Some(coord(r.x)), y: Some(coord(r.y)), ..Default::default() }));
                }
                let drop = if self.clients[i].fixed { None } else { self.drop_at(d.win, p) };
                d.kind = DragKind::Move { frac, grab_y, drop };
                let r = drop.and_then(|x| self.drop_rect(x));
                self.show_preview(r);
            }
            DragKind::Resize { left, right, top, bottom } => {
                let s = d.start;
                let (min_w, min_h) = (64 + 2 * BORDER as i32, TITLE as i32 + 32);
                let (mut x0, mut y0, mut x1, mut y1) = (s.x, s.y, s.x + s.width as i32, s.y + s.height as i32);
                if left {
                    x0 = (x0 + dx).min(x1 - min_w);
                }
                if right {
                    x1 = (x1 + dx).max(x0 + min_w);
                }
                if top {
                    y0 = (y0 + dy).min(y1 - min_h);
                }
                if bottom {
                    y1 = (y1 + dy).max(y0 + min_h);
                }
                self.clients[i].rect = Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32);
                self.apply(i);
            }
        }
        self.drag = Some(d);
    }

    fn end_drag(&mut self, p: (i32, i32)) {
        self.drag_to(p);
        let Some(d) = self.drag.take() else { return };
        self.show_preview(None);
        let Some(i) = self.index(d.win) else { return };
        if !d.moved {
            return;
        }
        match d.kind {
            DragKind::Move { drop: Some(Drop::Screen(z)), .. } => self.unfloat_to(i, z),
            DragKind::Move { drop: Some(Drop::Split(t, side)), .. } => {
                let Some(ti) = self.index(t) else { return };
                let whole = self.clients[ti].rect;
                self.clients[i].float = self.clients[i].rect;
                self.place(ti, Place::Tiled, layout::half(whole, layout::opposite(side)));
                let i = self.index(d.win).expect("managed");
                self.place(i, Place::Tiled, layout::half(whole, side));
            }
            _ => {
                let c = &mut self.clients[i];
                c.float = c.rect;
                self.apply(i);
            }
        }
    }

    // ------------------------------------------------------------ events

    /// One turn: wait up to `timeout` (forever: `None`) for events, handle
    /// them all, send what they caused.
    pub fn step(&mut self, timeout: Option<Duration>) -> Result<()> {
        let first = match timeout {
            Some(t) => self.conn.wait_event_timeout(t)?,
            None => Some(self.conn.wait_event()?),
        };
        if let Some(e) = first {
            self.handle(e);
        }
        while let Some(e) = self.conn.poll_event()? {
            self.handle(e);
        }
        self.conn.client().flush()
    }

    /// Handle one event.
    pub fn handle(&mut self, e: Event) {
        match e {
            Event::Foreign(f) => self.foreign(f.event),
            Event::Button { window, button: MouseButton::Left, pressed, x, y, root_x, root_y, time, .. } => {
                if let Some(i) = self.by_bar(window) {
                    self.bar_button(i, pressed, x, y, (root_x, root_y), time);
                }
            }
            Event::Motion { window, root_x, root_y, .. } if self.by_bar(window).is_some() => self.drag_to((root_x, root_y)),
            Event::Resized { window, .. } => {
                if let Some(i) = self.by_bar(window) {
                    self.draw_bar(i);
                }
            }
            _ => {}
        }
    }

    fn bar_button(&mut self, i: usize, pressed: bool, x: i32, y: i32, p: (i32, i32), time: u32) {
        let w = self.clients[i].win;
        let width = self.clients[i].rect.width.saturating_sub(2 * BORDER);
        let [max, close] = buttons(width);
        let on = if max.contains(x, y) {
            Some(0)
        } else if close.contains(x, y) {
            Some(1)
        } else {
            None
        };
        if pressed {
            self.raise(i);
            self.focus(Some(w));
            if let Some(b) = on {
                self.pending_button = Some((w, b));
                return;
            }
            let double = self.last_press.is_some_and(|(lw, lt)| lw == w && time.wrapping_sub(lt) < 400);
            self.last_press = Some((w, time));
            let i = self.index(w).expect("managed");
            if double {
                self.last_press = None;
                self.toggle_maximize(i);
                return;
            }
            let frac = (x.max(0) as u32).saturating_mul(1000) / width.max(1);
            let start = self.clients[i].rect;
            self.drag = Some(Drag { win: w, kind: DragKind::Move { frac, grab_y: BORDER as i32 + y, drop: None }, from: p, start, moved: false });
        } else {
            if let Some((pw, b)) = self.pending_button.take() {
                if pw == w && on == Some(b) {
                    if b == 0 {
                        self.toggle_maximize(i);
                    } else {
                        self.close(i);
                    }
                }
                return;
            }
            self.end_drag(p);
        }
    }

    fn frame_button(&mut self, i: usize, ev: &InputEvent) {
        let w = self.clients[i].win;
        self.raise(i);
        self.focus(Some(w));
        let i = self.index(w).expect("managed");
        let c = &self.clients[i];
        if c.place != Place::Floating || c.fixed || ev.detail != 1 {
            return;
        }
        let (x, y) = (i32::from(ev.x), i32::from(ev.y));
        let (fw, fh) = (c.rect.width as i32, c.rect.height as i32);
        let grip = 16;
        let b = BORDER as i32;
        let left = x < b || (x < grip && (y < b || y >= fh - b));
        let right = x >= fw - b || (x >= fw - grip && (y < b || y >= fh - b));
        let top = y < b || (y < grip && (x < b || x >= fw - b));
        let bottom = y >= fh - b || (y >= fh - grip && (x < b || x >= fw - b));
        if left || right || top || bottom {
            let start = c.rect;
            self.drag = Some(Drag {
                win: w,
                kind: DragKind::Resize { left, right, top, bottom },
                from: (i32::from(ev.root_x), i32::from(ev.root_y)),
                start,
                moved: false,
            });
        }
    }

    fn foreign(&mut self, e: RawEvent) {
        match e {
            RawEvent::MapRequest { window, .. } => match self.index(window) {
                Some(i) => self.activate(self.clients[i].win),
                None => self.manage(window, false),
            },
            RawEvent::ConfigureRequest { window, x, y, width, height, border_width, stack_mode, value_mask, .. } => {
                let Some(i) = self.index(window) else {
                    // Not ours (yet): as asked.
                    let m = |bit: u16| value_mask & bit != 0;
                    let cfg = ConfigureWindow {
                        x: m(1).then_some(x),
                        y: m(2).then_some(y),
                        width: m(4).then_some(width),
                        height: m(8).then_some(height),
                        border_width: m(16).then_some(border_width),
                        stack_mode: m(64).then_some(if stack_mode == 1 { StackMode::Below } else { StackMode::Above }),
                    };
                    self.send(&req::configure_window(window, &cfg));
                    return;
                };
                let dragging = self.drag.is_some_and(|d| d.win == window);
                let c = &mut self.clients[i];
                if c.place == Place::Floating && !dragging {
                    let (b, t) = c.insets();
                    let mut inner = c.inner();
                    if value_mask & 4 != 0 {
                        inner.width = u32::from(width).max(1);
                    }
                    if value_mask & 8 != 0 {
                        inner.height = u32::from(height).max(1);
                    }
                    let (mut fx, mut fy) = (c.rect.x, c.rect.y);
                    // Static gravity (protolith's): the position is the window's own.
                    let (ox, oy) = if c.static_gravity { (b as i32, (b + t) as i32) } else { (0, 0) };
                    if value_mask & 1 != 0 {
                        fx = i32::from(x) - ox;
                    }
                    if value_mask & 2 != 0 {
                        fy = i32::from(y) - oy;
                    }
                    c.rect = Rect::new(fx, fy, inner.width + 2 * b, inner.height + 2 * b + t);
                    c.float = c.rect;
                }
                // Tiled, maximized, fullscreen: refused, and told so (apply
                // sends where it really is).
                self.apply(i);
                if value_mask & 64 != 0 && stack_mode == 0 {
                    self.raise(i);
                }
            }
            RawEvent::UnmapNotify { window } => {
                if let Some(i) = self.index(window) {
                    if self.clients[i].ignore_unmaps > 0 {
                        self.clients[i].ignore_unmaps -= 1;
                    } else {
                        self.unmanage(i, false);
                    }
                }
            }
            RawEvent::DestroyNotify { window } => {
                if let Some(i) = self.index(window) {
                    self.unmanage(i, true);
                } else if self.dying.remove(&window) {
                    self.conn.client().release_id(window);
                }
            }
            RawEvent::PropertyNotify { window, atom: a, .. } => {
                if a == atom::WM_NAME || a == self.a("_NET_WM_NAME") {
                    if let Some(i) = self.index(window) {
                        self.clients[i].title = self.title_of(window);
                        self.draw_bar(i);
                    }
                }
            }
            RawEvent::ClientMessage { window, message_type, data, .. } => self.message(window, message_type, data),
            RawEvent::ButtonPress(ev) => {
                if let Some(i) = self.index(ev.event) {
                    // A click on an unfocused window: focus it, and pass the click on.
                    self.raise(i);
                    self.focus(Some(ev.event));
                    self.send(&req::allow_events(Allow::ReplayPointer, ev.time));
                } else if let Some(i) = self.by_frame(ev.event) {
                    self.frame_button(i, &ev);
                } else {
                    self.send(&req::allow_events(Allow::ReplayPointer, ev.time));
                }
            }
            RawEvent::MotionNotify(ev) if self.by_frame(ev.event).is_some() => self.drag_to((i32::from(ev.root_x), i32::from(ev.root_y))),
            RawEvent::ButtonRelease(ev) if self.by_frame(ev.event).is_some() => self.end_drag((i32::from(ev.root_x), i32::from(ev.root_y))),
            RawEvent::KeyPress(ev) => {
                let mods = ev.state & (SHIFT | CONTROL | ALT | SUPER);
                if let Some(&(_, _, k)) = self.keys.iter().find(|(code, m, _)| *code == ev.detail && *m == mods) {
                    self.hotkey(k);
                }
            }
            RawEvent::ConfigureNotify { window, width, height, .. } if window == self.root => self.new_screen(u32::from(width), u32::from(height)),
            _ => {}
        }
    }

    fn message(&mut self, window: Xid, kind: Atom, data: [u32; 5]) {
        let Some(i) = self.index(window) else { return };
        if kind == self.a("_NET_WM_STATE") {
            for prop in [data[1], data[2]] {
                let c = &self.clients[i];
                let set = |now: bool| match data[0] {
                    0 => false,
                    1 => true,
                    _ => !now,
                };
                if prop == self.a("_NET_WM_STATE_FULLSCREEN") {
                    let on = set(c.place == Place::Fullscreen);
                    self.set_fullscreen(i, on);
                } else if prop == self.a("_NET_WM_STATE_ABOVE") {
                    self.clients[i].above = set(c.above);
                    self.raise(i);
                    let i = self.index(window).expect("managed");
                    self.publish_state(i);
                    return;
                } else if prop == self.a("_NET_WM_STATE_MAXIMIZED_VERT") || prop == self.a("_NET_WM_STATE_MAXIMIZED_HORZ") {
                    if set(c.place == Place::Maximized) != (c.place == Place::Maximized) {
                        self.toggle_maximize(i);
                    }
                    return; // the pair is one change
                }
            }
        } else if kind == self.a("_NET_ACTIVE_WINDOW") {
            self.activate(window);
        } else if kind == self.a("_NET_CLOSE_WINDOW") {
            self.close(i);
        } else if kind == self.a("WM_CHANGE_STATE") && data[0] == 3 {
            self.iconify(i);
        }
    }

    fn hotkey(&mut self, k: Hotkey) {
        if k == Hotkey::Next {
            // The window under the top one: Alt+Tab, Alt+Tab, ... cycles.
            let n = self.clients.len();
            if n == 0 {
                return;
            }
            let w = self.clients[0].win;
            self.activate(w);
            return;
        }
        let Some(i) = self.focused.and_then(|w| self.index(w)) else { return };
        match k {
            Hotkey::Left => self.unfloat_to(i, Zone::Half(Side::Left)),
            Hotkey::Right => self.unfloat_to(i, Zone::Half(Side::Right)),
            Hotkey::Up => self.unfloat_to(i, Zone::Maximize),
            Hotkey::Down => {
                if self.clients[i].place == Place::Floating {
                    self.iconify(i);
                } else {
                    self.restore(i);
                }
            }
            Hotkey::Close => self.close(i),
            Hotkey::Next => {}
        }
    }

    /// The managed window that is `w`, for tests and tools.
    pub fn client(&self, w: Xid) -> Option<&Client> {
        self.index(w).map(|i| &self.clients[i])
    }
}

/// The maximize and close buttons of a bar `width` wide.
fn buttons(width: u32) -> [Rect; 2] {
    let y = (TITLE as i32 - BUTTON) / 2;
    let close = Rect::new(width as i32 - BUTTON - 3, y, BUTTON as u32, BUTTON as u32);
    let max = Rect::new(close.x - BUTTON - 2, y, BUTTON as u32, BUTTON as u32);
    [max, close]
}
