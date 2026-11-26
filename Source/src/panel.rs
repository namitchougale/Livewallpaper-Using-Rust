use super::*;
use windows_sys::Win32::UI::{Controls::*, Input::KeyboardAndMouse::SetFocus};
pub const ADD: i32 = 101;
pub const LIST: i32 = 102;
pub const REMOVE: i32 = 103;
pub const FILL: i32 = 104;
pub const FIT: i32 = 105;
pub const STRETCH: i32 = 106;
pub const APPLY: i32 = 107;
pub const PAUSE: i32 = 108;
pub const MORE: i32 = 109;
pub const QUIT: i32 = 110;
const BG: u32 = 0x18181f;
const CARD: u32 = 0x24242e;
const EDGES: u32 = 0x42414e;
const INK: u32 = 0xf5f2fb;
const MUTED: u32 = 0xb0aebf;
const LAV: u32 = 0xc6b2ff;
const TEAL: u32 = 0x85ddca;
const ACTIVE: u32 = 0x233e3c;
const WARN: u32 = 0xffc68d;
thread_local! {static SCALE:Cell<f64>=const{Cell::new(1.)};static BG_BRUSH:Cell<HBRUSH>=const{Cell::new(null_mut())};}
#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(hwnd: HWND, attr: u32, val: *const std::ffi::c_void, size: u32)
        -> i32;
}
fn col(hex: u32) -> u32 {
    ((hex & 255) << 16) | (hex & 0xff00) | (hex >> 16)
}
fn px(v: i32) -> i32 {
    (v as f64 * SCALE.get()).round() as i32
}
fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: px(x),
        top: px(y),
        right: px(x + w),
        bottom: px(y + h),
    }
}
fn inset(r: RECT, x: i32, y: i32) -> RECT {
    RECT {
        left: r.left + x,
        top: r.top + y,
        right: r.right - x,
        bottom: r.bottom - y,
    }
}
unsafe fn area(dc: HDC, r: RECT, color: u32) {
    SetDCBrushColor(dc, col(color));
    FillRect(dc, &r, GetStockObject(DC_BRUSH) as _);
}
unsafe fn rounded(dc: HDC, r: RECT, fill: u32, edge: u32, rad: i32) {
    let s = SaveDC(dc);
    SelectObject(dc, GetStockObject(DC_PEN));
    SelectObject(dc, GetStockObject(DC_BRUSH));
    SetDCPenColor(dc, col(edge));
    SetDCBrushColor(dc, col(fill));
    RoundRect(
        dc,
        r.left,
        r.top,
        r.right,
        r.bottom,
        px(rad) * 2,
        px(rad) * 2,
    );
    RestoreDC(dc, s);
}
unsafe fn text(dc: HDC, mut r: RECT, s: &str, size: i32, weight: i32, color: u32, flags: u32) {
    let font = CreateFontW(
        -px(size),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        wide("Segoe UI").as_ptr(),
    );
    let old = SelectObject(dc, font as _);
    SetTextColor(dc, col(color));
    SetBkMode(dc, TRANSPARENT as i32);
    let data = wide(s);
    DrawTextW(
        dc,
        data.as_ptr(),
        (data.len() - 1) as i32,
        &mut r,
        flags | DT_NOPREFIX,
    );
    SelectObject(dc, old);
    DeleteObject(font as _);
}
const LEFT: u32 = DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS;
const CENTER: u32 = DT_CENTER | DT_VCENTER | DT_SINGLELINE;
unsafe fn path(dc: HDC, x: i32, y: i32, points: &[(i32, i32)], color: u32) {
    let pen = CreatePen(PS_SOLID, px(2).max(1), col(color));
    let old = SelectObject(dc, pen as _);
    let pts: Vec<POINT> = points
        .iter()
        .map(|(a, b)| POINT {
            x: x + px(*a),
            y: y + px(*b),
        })
        .collect();
    Polyline(dc, pts.as_ptr(), pts.len() as i32);
    SelectObject(dc, old);
    DeleteObject(pen as _);
}
unsafe fn icon(dc: HDC, x: i32, y: i32, id: i32, c: u32) {
    match id {
        ADD => {
            path(
                dc,
                x,
                y,
                &[
                    (1, 5),
                    (1, 3),
                    (7, 3),
                    (9, 5),
                    (17, 5),
                    (17, 15),
                    (1, 15),
                    (1, 5),
                    (16, 5),
                ],
                c,
            );
        }
        APPLY => {
            path(dc, x, y, &[(1, 2), (16, 2), (16, 12), (1, 12), (1, 2)], c);
            path(dc, x, y, &[(9, 13), (9, 16), (4, 16), (14, 16)], c);
        }
        PAUSE if state(|s| s.paused) => path(dc, x, y, &[(4, 2), (14, 8), (4, 15), (4, 2)], c),
        PAUSE => {
            path(dc, x, y, &[(5, 2), (5, 15)], c);
            path(dc, x, y, &[(12, 2), (12, 15)], c);
        }
        QUIT => {
            path(dc, x, y, &[(8, 2), (2, 2), (2, 15), (8, 15)], c);
            path(dc, x, y, &[(6, 8), (16, 8), (12, 4)], c);
            path(dc, x, y, &[(16, 8), (12, 12)], c);
        }
        _ => {}
    }
}
pub unsafe fn work_area() -> RECT {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: GetSystemMetrics(SM_CXSCREEN),
        bottom: GetSystemMetrics(SM_CYSCREEN),
    };
    SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut r as *mut RECT).cast(), 0);
    r
}
struct Layout {
    w: i32,
    h: i32,
    hero: i32,
    add: i32,
    list: i32,
    sizing: i32,
}
impl Layout {
    unsafe fn get(p: HWND) -> Self {
        let mut r = zeroed();
        GetClientRect(p, &mut r);
        let w = (r.right as f64 / SCALE.get()).round() as i32;
        let h = (r.bottom as f64 / SCALE.get()).round() as i32;
        let hero = ((h - 430) / 2).clamp(62, 128);
        let add = 64 + hero + 58;
        let list = add + 78;
        Self {
            w,
            h,
            hero,
            add,
            list,
            sizing: h - 155,
        }
    }
}
unsafe fn arrange(p: HWND) {
    let l = Layout::get(p);
    let w = l.w;
    let inner = w - 36;
    let half = (inner - 8) / 2;
    let mut ctrls = vec![
        (ADD, rect(18, l.add, inner, 39)),
        (REMOVE, rect(w - 85, l.list - 29, 67, 24)),
        (
            LIST,
            rect(18, l.list, inner, (l.sizing - l.list - 10).max(26)),
        ),
        (APPLY, rect(18, l.h - 91, inner - half - 8, 36)),
        (PAUSE, rect(18 + inner - half, l.h - 91, half, 36)),
        (MORE, rect(18, l.h - 43, 109, 30)),
        (QUIT, rect(w - 99, l.h - 43, 81, 30)),
    ];
    let seg = (inner - 12) / 3;
    for i in 0..3 {
        ctrls.push((
            FILL + i,
            rect(
                18 + i * (seg + 6),
                l.sizing + 20,
                if i == 2 { inner - 2 * (seg + 6) } else { seg },
                31,
            ),
        ));
    }
    for (id, r) in ctrls {
        let child = GetDlgItem(p, id);
        if !child.is_null() {
            MoveWindow(child, r.left, r.top, r.right - r.left, r.bottom - r.top, 1);
        }
    }
}
pub unsafe fn draw_scene_content(dc: HDC, r: RECT) {
    let stock = state(|s| match s.active {
        Choice::Stock(i) => Some((i, s.frame as f32 * STOCK_INTERVAL_MS as f32 / 1000.)),
        _ => None,
    });
    if let Some((i, t)) = stock {
        stock::draw(dc, r, i, t)
    } else {
        state(|s| {
            if let Some(v) = &s.player {
                v.draw(dc, r, s.catalog.fit)
            } else {
                area(dc, r, 0x090a0f);
            }
        });
    }
}
unsafe fn draw_panel(p: HWND, dc: HDC) {
    let l = Layout::get(p);
    let w = l.w;
    area(dc, rect(0, 0, w + 2, l.h + 2), BG);
    let logo = LoadImageW(
        GetModuleHandleW(null()),
        101usize as _,
        IMAGE_ICON,
        px(30),
        px(30),
        LR_SHARED,
    );
    if !logo.is_null() {
        DrawIconEx(
            dc,
            px(18),
            px(16),
            logo as _,
            px(30),
            px(30),
            0,
            null_mut(),
            DI_NORMAL,
        );
    }
    text(dc, rect(57, 8, w - 170, 45), "Flow", 25, 650, INK, LEFT);
    let (mode, notice, problem, name) = state(|s| {
        let notice = if !s.problem && s.pending.is_none() && s.auto_paused && !s.paused {
            "Paused while the desktop is out of view.".to_string()
        } else {
            s.notice.clone()
        };
        (s.desk, notice, s.problem, s.active.title())
    });
    let (tag, color) = match mode {
        Desk::Desktop => ("Desktop", TEAL),
        Desk::Preview => ("Preview", LAV),
        Desk::Detached => ("Detached", WARN),
    };
    let tr = rect(w - 101, 19, 83, 23);
    rounded(dc, tr, CARD, EDGES, 10);
    text(dc, tr, tag, 12, 600, color, CENTER);
    let hero = rect(18, 58, w - 36, l.hero);
    let clip = CreateRoundRectRgn(
        hero.left,
        hero.top,
        hero.right + 1,
        hero.bottom + 1,
        px(18),
        px(18),
    );
    let saved = SaveDC(dc);
    ExtSelectClipRgn(dc, clip, RGN_AND);
    DeleteObject(clip);
    draw_scene_content(dc, hero);
    RestoreDC(dc, saved);
    text(
        dc,
        rect(19, 59 + l.hero, w - 38, 24),
        &name,
        16,
        600,
        INK,
        LEFT,
    );
    text(
        dc,
        rect(19, 84 + l.hero, w - 38, l.add - (84 + l.hero) - 2),
        &notice,
        12,
        400,
        if problem { WARN } else { MUTED },
        DT_WORDBREAK | DT_END_ELLIPSIS,
    );
    text(
        dc,
        rect(19, l.list - 29, w - 116, 24),
        "My wallpapers",
        14,
        600,
        INK,
        LEFT,
    );
    text(
        dc,
        rect(19, l.sizing - 2, w - 38, 21),
        "Wallpaper sizing",
        13,
        600,
        MUTED,
        LEFT,
    );
    area(dc, rect(18, l.h - 49, w - 36, 1), EDGES);
}
unsafe fn draw_item(item: &DRAWITEMSTRUCT) {
    let dc = item.hDC;
    let r = item.rcItem;
    let id = item.CtlID as i32;
    let focused = item.itemState & ODS_FOCUS != 0;
    let disabled = item.itemState & ODS_DISABLED != 0;
    if id == LIST {
        if item.itemID == u32::MAX {
            return;
        }
        let row = item.itemID as usize;
        let data = state(|s| {
            s.catalog.choice(row).map(|choice| {
                let on = s.active == choice;
                let waiting = s
                    .pending
                    .as_ref()
                    .is_some_and(|v| choice == Choice::Video(v.path.clone()));
                (choice, on, waiting)
            })
        });
        let Some((choice, on, waiting)) = data else {
            return;
        };
        let selected = item.itemState & ODS_SELECTED != 0;
        let bg = if on {
            ACTIVE
        } else if selected {
            0x383344
        } else {
            CARD
        };
        area(dc, r, bg);
        if on {
            area(
                dc,
                RECT {
                    right: r.left + px(3),
                    ..r
                },
                TEAL,
            );
        }
        let (symbol, tone) = if waiting {
            ("…", LAV)
        } else if on {
            ("●", TEAL)
        } else {
            ("○", MUTED)
        };
        let mut mark = r;
        mark.right = mark.left + px(31);
        text(dc, mark, symbol, 12, 600, tone, CENTER);
        let mut name = r;
        name.left += px(32);
        name.right -= px(61);
        text(
            dc,
            name,
            &choice.title(),
            14,
            if on { 600 } else { 400 },
            INK,
            LEFT,
        );
        let mut badge = r;
        badge.left = badge.right - px(60);
        badge.right -= px(10);
        text(
            dc,
            badge,
            if matches!(choice, Choice::Stock(_)) {
                "Scene"
            } else {
                "MP4"
            },
            11,
            500,
            if on { TEAL } else { MUTED },
            DT_RIGHT | DT_SINGLELINE | DT_VCENTER,
        );
        if focused {
            DrawFocusRect(dc, &inset(r, px(3), px(2)));
        }
        return;
    }
    let selected = state(|s| {
        matches!(
            (id, s.catalog.fit),
            (FILL, Fit::Fill) | (FIT, Fit::Fit) | (STRETCH, Fit::Stretch)
        )
    });
    let pressed = item.itemState & ODS_SELECTED != 0;
    let bare = matches!(id, MORE | QUIT | REMOVE);
    let (fill, stroke, ink) = if disabled {
        (BG, BG, 0x787482)
    } else if id == ADD {
        (if pressed { 0xaa90e9 } else { LAV }, LAV, 0x231d37)
    } else if selected {
        (ACTIVE, 0x477c70, TEAL)
    } else if bare {
        (
            if pressed { CARD } else { BG },
            BG,
            if id == QUIT { 0xf3abb4 } else { MUTED },
        )
    } else {
        (if pressed { 0x343340 } else { CARD }, EDGES, INK)
    };
    area(dc, r, BG);
    rounded(dc, r, fill, stroke, 7);
    let label = match id {
        ADD => "Add MP4",
        REMOVE => "Remove",
        FILL => "Fill",
        FIT => "Fit",
        STRETCH => "Stretch",
        APPLY => "Apply to desktop",
        PAUSE => {
            if state(|s| s.paused) {
                "Resume"
            } else {
                "Pause"
            }
        }
        MORE => "Options…",
        QUIT => "Quit",
        _ => "",
    };
    let has_icon = matches!(id, ADD | APPLY | PAUSE | QUIT);
    let mut tx = r;
    if has_icon {
        let offset = match id {
            ADD => 43,
            APPLY => 63,
            PAUSE => 32,
            QUIT => 24,
            _ => 0,
        };
        let x = (r.left + r.right) / 2 - px(offset);
        icon(dc, x, (r.top + r.bottom) / 2 - px(8), id, ink);
        tx.left += px(21);
    }
    text(
        dc,
        tx,
        label,
        if id == ADD { 16 } else { 13 },
        if id == ADD || selected { 600 } else { 500 },
        ink,
        CENTER,
    );
    if focused {
        DrawFocusRect(dc, &inset(r, px(4), px(4)));
    }
}
pub unsafe fn populate(p: HWND, select: usize) {
    let list = GetDlgItem(p, LIST);
    SendMessageW(list, WM_SETREDRAW, 0, 0);
    SendMessageW(list, LB_RESETCONTENT, 0, 0);
    let names = state(|s| {
        (0..STOCK.len() + s.catalog.files.len())
            .filter_map(|i| s.catalog.choice(i).map(|c| c.title()))
            .collect::<Vec<_>>()
    });
    for name in names {
        SendMessageW(list, LB_ADDSTRING, 0, wide(&name).as_ptr() as isize);
    }
    SendMessageW(list, LB_SETCURSEL, select, 0);
    SendMessageW(list, WM_SETREDRAW, 1, 0);
    refresh(p);
}
pub unsafe fn selected(p: HWND) -> Option<usize> {
    let row = SendMessageW(GetDlgItem(p, LIST), LB_GETCURSEL, 0, 0);
    usize::try_from(row).ok()
}
pub unsafe fn refresh(p: HWND) {
    if p.is_null() {
        return;
    }
    let (paused, video) = state(|s| (s.paused, s.player.is_some()));
    EnableWindow(
        GetDlgItem(p, REMOVE),
        selected(p).is_some_and(|r| r >= STOCK.len()) as i32,
    );
    for i in FILL..=STRETCH {
        EnableWindow(GetDlgItem(p, i), video as i32);
    }
    SetWindowTextW(
        GetDlgItem(p, PAUSE),
        wide(if paused { "Resume" } else { "Pause" }).as_ptr(),
    );
    RedrawWindow(p, null(), null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN);
}
pub unsafe fn icon_handle() -> HICON {
    LoadIconW(GetModuleHandleW(null()), 101usize as _)
}
pub unsafe fn create() -> HWND {
    let frame = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    let work = work_area();
    let dc = GetDC(null_mut());
    let dpi = GetDeviceCaps(dc, LOGPIXELSX as i32);
    ReleaseDC(null_mut(), dc);
    let mut border = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    AdjustWindowRectEx(&mut border, frame, 0, 0);
    let slack = border.bottom - border.top;
    let available = work.bottom - work.top - slack - 16;
    SCALE.set(
        (dpi as f64 / 96.)
            .min(available as f64 / 540.)
            .min((work.right - work.left - 16) as f64 / 362.)
            .clamp(0.6, 3.),
    );
    let h = (available as f64 / SCALE.get()).floor().min(690.) as i32;
    let mut outer = rect(0, 0, 362, h);
    AdjustWindowRectEx(&mut outer, frame, 0, 0);
    let width = outer.right - outer.left;
    let height = outer.bottom - outer.top;
    let p = CreateWindowExW(
        WS_EX_CONTROLPARENT,
        wide(PANEL_CLASS).as_ptr(),
        wide("Flow Live Wallpaper").as_ptr(),
        frame,
        work.right - width - px(10),
        work.top + ((work.bottom - work.top - height) / 2).max(0),
        width,
        height,
        null_mut(),
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    if p.is_null() {
        return p;
    }
    let on = 1i32;
    DwmSetWindowAttribute(p, 20, (&on as *const i32).cast(), 4);
    BG_BRUSH.set(CreateSolidBrush(col(CARD)));
    for (id, name) in [
        (ADD, "Add MP4"),
        (LIST, "My wallpapers"),
        (REMOVE, "Remove"),
        (FILL, "Fill"),
        (FIT, "Fit"),
        (STRETCH, "Stretch"),
        (APPLY, "Apply to desktop"),
        (PAUSE, "Pause"),
        (MORE, "Options"),
        (QUIT, "Quit"),
    ] {
        let (class, style) = if id == LIST {
            (
                "LISTBOX",
                (LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT | LBS_NOTIFY) as u32
                    | WS_VSCROLL,
            )
        } else {
            ("BUTTON", BS_OWNERDRAW as u32)
        };
        let child = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(name).as_ptr(),
            WS_VISIBLE | WS_CHILD | WS_TABSTOP | style,
            0,
            0,
            1,
            1,
            p,
            id as _,
            GetModuleHandleW(null()),
            null(),
        );
        if id == LIST {
            SendMessageW(child, LB_SETITEMHEIGHT, 0, px(31) as isize);
        }
    }
    DragAcceptFiles(p, 1);
    arrange(p);
    p
}
pub unsafe extern "system" fn wndproc(p: HWND, msg: u32, a: usize, b: isize) -> isize {
    match msg {
        WM_PAINT => {
            let mut ps = zeroed();
            let dc = BeginPaint(p, &mut ps);
            draw_panel(p, dc);
            EndPaint(p, &ps);
        }
        WM_ERASEBKGND => return 1,
        WM_CTLCOLORLISTBOX => {
            SetBkColor(a as _, col(CARD));
            SetTextColor(a as _, col(INK));
            return BG_BRUSH.get() as isize;
        }
        WM_DRAWITEM => {
            if b != 0 {
                draw_item(&*(b as *const DRAWITEMSTRUCT));
            }
            return 1;
        }
        WM_MEASUREITEM => {
            if b != 0 {
                (*(b as *mut MEASUREITEMSTRUCT)).itemHeight = px(31) as u32;
            }
            return 1;
        }
        WM_SIZE => {
            if a != SIZE_MINIMIZED as usize {
                arrange(p);
            }
        }
        WM_COMMAND => {
            let id = (a & 65535) as i32;
            let event = (a >> 16) as u32;
            if id == LIST && (event == LBN_SELCHANGE || event == LBN_DBLCLK) {
                if let Some(i) = selected(p) {
                    crate::select_row(i);
                }
            } else if event == BN_CLICKED {
                crate::command(id);
            }
        }
        WM_DROPFILES => crate::drop_files(a as _),
        WM_TIMER => crate::poll(a),
        WM_WTSSESSION_CHANGE if matches!(a as u32, WTS_SESSION_LOCK | WTS_SESSION_UNLOCK) => {
            edit(|s| s.locked = a as u32 == WTS_SESSION_LOCK);
            crate::playback_policy();
        }
        WM_POWERBROADCAST if a as u32 == PBT_POWERSETTINGCHANGE => {
            crate::display_power(b as _);
            return 1;
        }
        playback::VIDEO_EVENT => crate::video_event(a, b),
        TRAY_MSG => match b as u32 {
            WM_LBUTTONUP => crate::show_panel(),
            WM_RBUTTONUP => crate::menu(),
            _ => {}
        },
        WM_DISPLAYCHANGE => {
            if state(|s| s.desk) == Desk::Desktop {
                crate::apply_desktop();
            }
        }
        WM_CLOSE => {
            ShowWindow(p, SW_MINIMIZE);
        }
        WM_DESTROY => {
            WTSUnRegisterSessionNotification(p);
            let brush = BG_BRUSH.replace(null_mut());
            if !brush.is_null() {
                DeleteObject(brush as _);
            }
            PostQuitMessage(0);
        }
        _ if msg != 0 && msg == TASKBAR_MSG.get() => {
            crate::add_tray();
            if state(|s| s.desktop_wanted) {
                crate::apply_desktop();
            }
        }
        _ => return DefWindowProcW(p, msg, a, b),
    }
    0
}
pub unsafe fn focus_import(p: HWND) {
    SetFocus(GetDlgItem(p, ADD));
}
