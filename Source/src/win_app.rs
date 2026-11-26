use flow_live_wallpaper::{
    catalog::{dialog_paths, mp4, Catalog, Choice, STOCK},
    geometry::{self, Fit},
};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW, Power::*, Registry::*, RemoteDesktop::*,
        SystemServices::GUID_CONSOLE_DISPLAY_STATE, Threading::*,
    },
    UI::{
        Controls::Dialogs::*, Input::KeyboardAndMouse::EnableWindow, Shell::*,
        WindowsAndMessaging::*,
    },
};
mod budget;
mod desktop;
mod panel;
mod playback;
mod stock;
mod stream;

const PANEL_CLASS: &str = "Ypar.Flow.MP4.Panel.11";
const WALL_CLASS: &str = "Ypar.Flow.MP4.Wall.11";
const TRAY_MSG: u32 = WM_APP + 1;
const HEALTH: usize = 1;
const ANIMATE: usize = 2;
const STOCK_INTERVAL_MS: u32 = 1000u32.div_ceil(geometry::DISPLAY_FPS);
static CANVAS: AtomicUsize = AtomicUsize::new(0);
thread_local! {static PANEL:Cell<HWND>=const{Cell::new(null_mut())};static TASKBAR_MSG:Cell<u32>=const{Cell::new(0)};static APP:RefCell<Option<App>>=const{RefCell::new(None)};}
#[derive(Clone, Copy, PartialEq)]
enum Desk {
    Desktop,
    Preview,
    Detached,
}
struct App {
    dir: PathBuf,
    catalog: Catalog,
    active: Choice,
    player: Option<playback::Player>,
    pending: Option<playback::Player>,
    serial: usize,
    desk: Desk,
    desktop_wanted: bool,
    notice: String,
    desk_error: bool,
    problem: bool,
    paused: bool,
    locked: bool,
    screen_off: bool,
    auto_paused: bool,
    playing_stock: bool,
    frame: u32,
    health_ticks: u32,
}
fn state<R>(f: impl FnOnce(&App) -> R) -> R {
    APP.with_borrow(|a| f(a.as_ref().expect("Flow state")))
}
fn edit<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with_borrow_mut(|a| f(a.as_mut().expect("Flow state")))
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn wall() -> HWND {
    CANVAS.load(Ordering::Relaxed) as HWND
}
fn message(s: impl Into<String>, error: bool) {
    edit(|a| {
        a.notice = s.into();
        a.problem = error;
        a.desk_error = false;
    });
}
fn save() {
    let result = state(|s| s.catalog.save(&s.dir));
    if result.is_err() {
        message(
            "Changes apply for this session. Windows could not save the library.",
            true,
        );
    }
}
unsafe fn refresh() {
    panel::refresh(PANEL.get());
}
unsafe fn show_panel() {
    let p = PANEL.get();
    ShowWindow(p, SW_RESTORE);
    SetForegroundWindow(p);
}
unsafe fn new_canvas() -> HWND {
    let w = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        wide(WALL_CLASS).as_ptr(),
        wide("Flow wallpaper preview").as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        50,
        50,
        880,
        495,
        null_mut(),
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    CANVAS.store(w as usize, Ordering::Relaxed);
    w
}
unsafe fn ensure_canvas() -> HWND {
    let w = wall();
    if w.is_null() || IsWindow(w) == 0 {
        new_canvas()
    } else {
        w
    }
}
unsafe fn apply_desktop() {
    let w = ensure_canvas();
    edit(|s| s.desktop_wanted = true);
    let result = if w.is_null() {
        Err("The wallpaper window could not be created.".into())
    } else {
        desktop::attach(w)
    };
    match result {
        Ok(()) => {
            edit(|s| s.desk = Desk::Desktop);
            if state(|s| !s.problem || s.desk_error) {
                message("Wallpaper is behind your desktop icons.", false);
            }
        }
        Err(e) => {
            if !w.is_null() {
                ShowWindow(w, SW_HIDE);
            }
            edit(|s| s.desk = Desk::Detached);
            if state(|s| !s.problem || s.desk_error) {
                message(format!("{e} Choose Apply to retry."), true);
                edit(|s| s.desk_error = true);
            }
        }
    }
    playback_policy();
    refresh();
}
unsafe fn preview() {
    let w = ensure_canvas();
    if w.is_null() {
        message("The preview window could not open.", true);
        refresh();
        return;
    }
    edit(|s| {
        s.desktop_wanted = false;
        s.desk = Desk::Preview;
    });
    desktop::preview(w);
    message("Preview window open. Apply to desktop when ready.", false);
    playback_policy();
    refresh();
}
unsafe fn select_row(row: usize) {
    let Some(item) = state(|s| s.catalog.choice(row)) else {
        return;
    };
    match item {
        Choice::Stock(i) => {
            edit(|s| {
                s.pending.take();
                s.player.take();
                s.active = Choice::Stock(i);
                s.catalog.selected = row;
                s.paused = false;
                s.frame = 0;
            });
            let w = wall();
            if !w.is_null() {
                InvalidateRect(w, null(), 0);
            }
            message("Built-in scene selected.", false);
            save();
            playback_policy();
            refresh();
        }
        Choice::Video(path) => {
            if !path.is_file() {
                if edit(|s| s.pending.take().is_some()) {
                    restore_previous();
                }
                message(
                    format!(
                        "File unavailable: {}. Add it again from its current folder.",
                        item_name(&path)
                    ),
                    true,
                );
                refresh();
                return;
            }
            if begin_video(&path, false) {
                message(format!("Opening {}…", item_name(&path)), false);
            } else {
                message("Windows could not start the video decoder.", true);
            }
            refresh();
        }
    }
}
fn begin_video(path: &Path, restoring: bool) -> bool {
    let id = edit(|s| {
        s.serial = s.serial.wrapping_add(1);
        s.serial
    });
    let Ok(next) = playback::Player::open(path, id, PANEL.get(), restoring) else {
        return false;
    };
    edit(|s| {
        if let Some(old) = &s.player {
            old.freeze();
        }
        s.pending = Some(next);
    });
    true
}

fn restore_previous() {
    let path = state(|s| {
        s.player
            .as_ref()
            .filter(|p| p.frozen())
            .map(|p| p.path.clone())
    });
    if let Some(path) = path {
        if !begin_video(&path, true) {
            edit(|s| s.paused = true);
        }
    }
}
fn item_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
unsafe fn video_event(id: usize, event: isize) {
    let pending = state(|s| s.pending.as_ref().is_some_and(|p| p.id == id));
    if pending && event == playback::READY {
        let restoring = edit(|s| {
            let player = s.pending.take().unwrap();
            let restoring = player.restoring;
            s.active = Choice::Video(player.path.clone());
            s.catalog.selected = s.catalog.index(&s.active).unwrap_or(0);
            s.player = Some(player);
            if !restoring {
                s.paused = false;
            }
            restoring
        });
        stock::clear();
        if !restoring {
            message(
                match state(|s| s.desk) {
                    Desk::Desktop => "MP4 playing on your desktop.",
                    Desk::Preview => "MP4 playing in the preview window.",
                    Desk::Detached => "MP4 ready. Choose Apply to attach it to the desktop.",
                },
                false,
            );
        }
        save();
        playback_policy();
        let w = wall();
        if !w.is_null() {
            InvalidateRect(w, null(), 0);
        }
        refresh();
    } else if pending && event == playback::FAILED {
        let failed = edit(|s| s.pending.take().unwrap());
        if failed.restoring {
            edit(|s| s.paused = true);
            message(
                format!(
                    "{} Previous clip paused. Use Resume to retry.",
                    state(|s| s.notice.clone())
                ),
                true,
            );
        } else {
            message(playback::error_message(failed.error()), true);
            restore_previous();
        }
        refresh();
    } else if event == playback::FAILED && state(|s| s.player.as_ref().is_some_and(|p| p.id == id))
    {
        let code = edit(|s| {
            s.paused = true;
            s.player.as_ref().unwrap().error()
        });
        message(playback::error_message(code), true);
        refresh();
    }
}
unsafe fn import(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let mut first = None;
    edit(|s| {
        for path in paths {
            if mp4(&path) && path.is_file() {
                if let Some(row) = s.catalog.add(path) {
                    first.get_or_insert(row);
                }
            }
        }
    });
    if let Some(row) = first {
        save();
        panel::populate(PANEL.get(), row);
        select_row(row);
    } else {
        message("Choose an existing .mp4 video file.", true);
        refresh();
    }
}
unsafe fn choose() {
    let mut buffer = vec![0u16; 131072];
    let filter = wide("MP4 video (*.mp4)\0*.mp4\0");
    let title = wide("Add MP4 wallpapers");
    let mut ofn: OPENFILENAMEW = zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = PANEL.get();
    ofn.lpstrFilter = filter.as_ptr();
    ofn.nFilterIndex = 1;
    ofn.lpstrFile = buffer.as_mut_ptr();
    ofn.nMaxFile = buffer.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_FILEMUSTEXIST
        | OFN_PATHMUSTEXIST
        | OFN_NOCHANGEDIR
        | OFN_EXPLORER
        | OFN_ALLOWMULTISELECT;
    if GetOpenFileNameW(&mut ofn) != 0 {
        import(dialog_paths(&buffer));
    } else {
        let e = CommDlgExtendedError();
        if e != 0 {
            message(
                format!("File picker error 0x{e:04X}. You can drop an MP4 on this panel."),
                true,
            );
            refresh();
        }
    }
}
unsafe fn drop_files(drop: HDROP) {
    let count = DragQueryFileW(drop, u32::MAX, null_mut(), 0);
    let mut files = Vec::new();
    for i in 0..count {
        let n = DragQueryFileW(drop, i, null_mut(), 0);
        let mut data = vec![0u16; n as usize + 1];
        let got = DragQueryFileW(drop, i, data.as_mut_ptr(), n + 1);
        if got > 0 {
            files.push(PathBuf::from(OsString::from_wide(&data[..got as usize])));
        }
    }
    DragFinish(drop);
    import(files);
}
unsafe fn remove_selected() {
    let Some(row) = panel::selected(PANEL.get()) else {
        return;
    };
    if row < STOCK.len() {
        return;
    }
    let mut cancelled = false;
    let is_current = edit(|s| {
        let i = row - STOCK.len();
        if i >= s.catalog.files.len() {
            return false;
        }
        let path = s.catalog.files.remove(i);
        if s.pending.as_ref().is_some_and(|p| p.path == path) {
            s.pending.take();
            cancelled = true;
        }
        let active = s.active == Choice::Video(path);
        s.catalog.selected = s.catalog.index(&s.active).unwrap_or(0);
        active
    });
    if is_current {
        select_row(0);
    } else if cancelled {
        restore_previous();
    }
    save();
    panel::populate(PANEL.get(), state(|s| s.catalog.selected));
    message(
        "Removed from the list. The video file stays on your PC.",
        false,
    );
    refresh();
}
unsafe fn playback_policy() {
    let p = PANEL.get();
    let w = wall();
    if p.is_null() {
        return;
    }
    let auto = state(|s| s.locked || s.screen_off)
        || desktop::covered(w, p, state(|s| s.desk) == Desk::Desktop);
    let visible = match state(|s| s.desk) {
        Desk::Preview => !w.is_null() && IsIconic(w) == 0,
        Desk::Desktop => true,
        Desk::Detached => false,
    };
    let (new_stock, old_stock, changed) = edit(|s| {
        let running = !s.paused && !auto && visible;
        if let Some(v) = &s.player {
            v.running(running);
        }
        let new_stock = running && s.player.is_none();
        let old = s.playing_stock;
        let change = s.auto_paused != auto;
        s.auto_paused = auto;
        s.playing_stock = new_stock;
        (new_stock, old, change)
    });
    if old_stock != new_stock {
        if new_stock {
            SetTimer(p, ANIMATE, STOCK_INTERVAL_MS, None);
        } else {
            KillTimer(p, ANIMATE);
        }
    }
    if changed {
        refresh();
    }
}
unsafe fn poll(timer: usize) {
    if timer == ANIMATE {
        if !state(|s| s.playing_stock) {
            return;
        }
        edit(|s| s.frame = s.frame.wrapping_add(1));
        let w = wall();
        if !w.is_null() {
            InvalidateRect(w, null(), 0);
        }
        return;
    }
    if timer != HEALTH {
        return;
    }
    edit(|s| s.health_ticks = s.health_ticks.wrapping_add(1));
    if state(|s| s.desk) == Desk::Desktop && (wall().is_null() || GetParent(wall()).is_null()) {
        edit(|s| s.desk = Desk::Detached);
        message("Desktop changed. Reconnecting…", false);
        refresh();
    }
    if state(|s| s.desktop_wanted && s.desk == Desk::Detached && s.health_ticks % 24 == 0) {
        apply_desktop();
    }
    playback_policy();
}
unsafe fn display_power(setting: *const POWERBROADCAST_SETTING) {
    let Some(s) = setting.as_ref() else { return };
    let id = s.PowerSetting;
    let wanted = GUID_CONSOLE_DISPLAY_STATE;
    if (id.data1, id.data2, id.data3, id.data4)
        == (wanted.data1, wanted.data2, wanted.data3, wanted.data4)
        && s.DataLength == 4
    {
        let value = s.Data.as_ptr().cast::<u32>().read_unaligned();
        edit(|a| a.screen_off = value == 0);
        playback_policy();
    }
}
unsafe fn command(id: i32) {
    match id {
        panel::ADD => choose(),
        panel::REMOVE => remove_selected(),
        panel::FILL..=panel::STRETCH => {
            edit(|s| {
                s.catalog.fit = match id {
                    panel::FIT => Fit::Fit,
                    panel::STRETCH => Fit::Stretch,
                    _ => Fit::Fill,
                }
            });
            save();
            let w = wall();
            if !w.is_null() {
                InvalidateRect(w, null(), 0);
            }
            refresh();
        }
        panel::APPLY => {
            let row = panel::selected(PANEL.get());
            let needs =
                row.is_some_and(|r| state(|s| s.catalog.selected != r && s.pending.is_none()));
            if needs {
                select_row(row.unwrap());
            }
            if !state(|s| s.problem) {
                message("Applying wallpaper…", false);
            }
            apply_desktop();
        }
        panel::PAUSE => {
            let retry = state(|s| {
                s.paused
                    && s.player
                        .as_ref()
                        .is_some_and(|v| v.error() != 0 || v.frozen())
                    && s.pending.is_none()
            });
            if retry {
                select_row(state(|s| s.catalog.selected));
                return;
            }
            let paused = edit(|s| {
                s.paused = !s.paused;
                s.paused
            });
            message(
                if paused {
                    "Playback paused."
                } else {
                    "Playback resumed."
                },
                false,
            );
            playback_policy();
            refresh();
        }
        panel::MORE => menu(),
        panel::QUIT => {
            DestroyWindow(PANEL.get());
        }
        _ => {}
    }
}
unsafe fn menu() {
    let p = PANEL.get();
    let m = CreatePopupMenu();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = wide("FlowLiveWallpaper");
    let mut size = 0;
    let on = RegGetValueW(
        HKEY_CURRENT_USER,
        key.as_ptr(),
        name.as_ptr(),
        RRF_RT_REG_SZ,
        null_mut(),
        null_mut(),
        &mut size,
    ) == ERROR_SUCCESS;
    for (id, label) in [
        (1, "Show panel"),
        (2, "Add MP4…"),
        (3, "Open preview window"),
        (4, "Apply to desktop"),
        (5, "Start with Windows"),
        (6, "Quit Flow"),
    ] {
        AppendMenuW(
            m,
            if id == 5 && on { MF_CHECKED } else { 0 },
            id,
            wide(label).as_ptr(),
        );
    }
    if let Some(stats) = budget::usage() {
        AppendMenuW(
            m,
            MF_GRAYED,
            0,
            wide(&format!(
                "RAM {:.1} MB  ·  Private commit {:.1} MB  ·  Limit 140 MB",
                stats.WorkingSetSize as f64 / 1_000_000.,
                stats.PrivateUsage as f64 / 1_000_000.
            ))
            .as_ptr(),
        );
    }
    let mut pt = zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(p);
    let choice = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, p, null());
    DestroyMenu(m);
    PostMessageW(p, WM_NULL, 0, 0);
    match choice {
        1 => show_panel(),
        2 => {
            show_panel();
            choose()
        }
        3 => preview(),
        4 => apply_desktop(),
        5 => {
            let result = if on {
                RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) == ERROR_SUCCESS
            } else {
                std::env::current_exe().is_ok_and(|exe| {
                    use std::os::windows::ffi::OsStrExt;
                    let quoted: Vec<_> = [34]
                        .into_iter()
                        .chain(exe.as_os_str().encode_wide())
                        .chain([34, 0])
                        .collect();
                    RegSetKeyValueW(
                        HKEY_CURRENT_USER,
                        key.as_ptr(),
                        name.as_ptr(),
                        REG_SZ,
                        quoted.as_ptr().cast(),
                        (quoted.len() * 2) as u32,
                    ) == ERROR_SUCCESS
                })
            };
            message(
                if !result {
                    "Could not update the Windows startup setting."
                } else if on {
                    "Start with Windows is off."
                } else {
                    "Start with Windows is on."
                },
                !result,
            );
            refresh();
        }
        6 => {
            DestroyWindow(p);
        }
        _ => {}
    }
}
unsafe fn tray(cmd: u32) {
    let mut t: NOTIFYICONDATAW = zeroed();
    t.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    t.hWnd = PANEL.get();
    t.uID = 1;
    t.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    t.uCallbackMessage = TRAY_MSG;
    t.hIcon = panel::icon_handle();
    let tip = wide("Flow Live Wallpaper");
    t.szTip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(cmd, &t);
}
unsafe fn add_tray() {
    tray(NIM_ADD);
}
unsafe extern "system" fn wallpaper_proc(w: HWND, msg: u32, a: usize, b: isize) -> isize {
    match msg {
        WM_PAINT => {
            let mut ps = zeroed();
            let dc = BeginPaint(w, &mut ps);
            let mut r = zeroed();
            GetClientRect(w, &mut r);
            let scene = state(|s| match s.active {
                Choice::Stock(id) => Some((id, s.frame)),
                _ => None,
            });
            if let Some((id, frame)) = scene {
                stock::paint(dc, r, id, frame);
            } else {
                panel::draw_scene_content(dc, r);
            }
            EndPaint(w, &ps);
        }
        WM_ERASEBKGND => return 1,
        WM_NCHITTEST if state(|s| s.desk) == Desk::Desktop => return HTTRANSPARENT as isize,
        WM_SIZE => {
            InvalidateRect(w, null(), 0);
        }
        WM_RBUTTONUP => menu(),
        WM_CLOSE => {
            if state(|s| s.desk) == Desk::Preview {
                ShowWindow(w, SW_HIDE);
                edit(|s| s.desk = Desk::Detached);
                playback_policy();
                message(
                    "Preview closed. Apply to desktop to show the wallpaper.",
                    false,
                );
                refresh();
            }
            return 0;
        }
        WM_DESTROY => {
            if wall() == w {
                stock::clear();
                CANVAS.store(0, Ordering::Relaxed);
            }
        }
        _ => return DefWindowProcW(w, msg, a, b),
    }
    0
}
unsafe fn register(class: &str, proc: WNDPROC) -> bool {
    let name = wide(class);
    let mut c: WNDCLASSW = zeroed();
    c.lpfnWndProc = proc;
    c.hInstance = GetModuleHandleW(null());
    c.hIcon = panel::icon_handle();
    c.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    c.lpszClassName = name.as_ptr();
    RegisterClassW(&c) != 0
}
fn main() {
    unsafe {
        let mutex = CreateMutexW(null(), 0, wide("Local\\Ypar.Flow.Mp4Panel.11").as_ptr());
        if !mutex.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
            let old = FindWindowW(wide(PANEL_CLASS).as_ptr(), null());
            if !old.is_null() {
                ShowWindow(old, SW_RESTORE);
                SetForegroundWindow(old);
            }
            CloseHandle(mutex);
            return;
        }
        let _budget = match budget::Budget::install() {
            Ok(budget) => budget,
            Err(e) => {
                MessageBoxW(
                    null_mut(),
                    wide(&format!(
                        "Windows could not enable Flow's 140 MB memory limit.\n\n{e}"
                    ))
                    .as_ptr(),
                    wide("Flow Live Wallpaper").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
                if !mutex.is_null() {
                    CloseHandle(mutex);
                }
                return;
            }
        };
        let dir = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("FlowLiveWallpaper");
        let sample = std::env::current_exe().ok().and_then(|exe| {
            exe.parent()
                .map(|d| d.join("Wallpapers").join("Violet Drift.mp4"))
        });
        let catalog = Catalog::load(&dir, sample);
        let saved = catalog.selected;
        APP.with_borrow_mut(|a| {
            *a = Some(App {
                dir,
                catalog,
                active: Choice::Stock(0),
                player: None,
                pending: None,
                serial: 0,
                desk: Desk::Detached,
                desktop_wanted: false,
                notice: String::new(),
                desk_error: false,
                problem: false,
                paused: false,
                locked: false,
                screen_off: false,
                auto_paused: false,
                playing_stock: false,
                frame: 0,
                health_ticks: 0,
            })
        });
        if !register(PANEL_CLASS, Some(panel::wndproc))
            || !register(WALL_CLASS, Some(wallpaper_proc))
        {
            MessageBoxW(
                null_mut(),
                wide("Flow could not register its windows.").as_ptr(),
                wide("Flow Live Wallpaper").as_ptr(),
                MB_OK,
            );
            return;
        }
        let p = panel::create();
        if p.is_null() {
            MessageBoxW(
                null_mut(),
                wide("Flow could not open its control panel.").as_ptr(),
                wide("Flow Live Wallpaper").as_ptr(),
                MB_OK,
            );
            return;
        }
        PANEL.set(p);
        let power = RegisterPowerSettingNotification(
            p,
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        WTSRegisterSessionNotification(p, NOTIFY_FOR_THIS_SESSION);
        new_canvas();
        TASKBAR_MSG.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
        add_tray();
        panel::populate(p, saved);
        ShowWindow(p, SW_SHOWNORMAL);
        if std::env::args_os().any(|a| a == "--preview") {
            preview();
        } else {
            apply_desktop();
        }
        if saved != 0 {
            select_row(saved);
        } else if state(|s| !s.problem) {
            message("Add an MP4 or drop one onto this panel.", false);
            refresh();
        }
        SetTimer(p, HEALTH, 1000, None);
        panel::focus_import(p);
        let mut msg = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(p, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        KillTimer(p, HEALTH);
        KillTimer(p, ANIMATE);
        tray(NIM_DELETE);
        if power != 0 {
            UnregisterPowerSettingNotification(power);
        }
        edit(|s| {
            s.pending.take();
            s.player.take();
        });
        if !wall().is_null() {
            DestroyWindow(wall());
        }
        if !mutex.is_null() {
            CloseHandle(mutex);
        }
    }
}
