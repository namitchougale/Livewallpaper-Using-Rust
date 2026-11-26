use super::*;
fn named(parent: HWND, after: HWND, class: &str) -> HWND {
    unsafe { FindWindowExW(parent, after, wide(class).as_ptr(), null()) }
}
#[derive(Clone, Copy)]
struct Layer {
    host: HWND,
    below: HWND,
    raised: bool,
}
unsafe extern "system" fn find_classic(w: HWND, data: LPARAM) -> BOOL {
    if !named(w, null_mut(), "SHELLDLL_DefView").is_null() {
        let background = named(null_mut(), w, "WorkerW");
        if !background.is_null() {
            *(data as *mut HWND) = background;
            return 0;
        }
    }
    1
}
unsafe fn layer(prog: HWND) -> Option<Layer> {
    let icons = named(prog, null_mut(), "SHELLDLL_DefView");
    let raised = GetWindowLongW(prog, GWL_EXSTYLE) as u32 & WS_EX_NOREDIRECTIONBITMAP != 0;
    if raised && !icons.is_null() && !named(prog, null_mut(), "WorkerW").is_null() {
        return Some(Layer {
            host: prog,
            below: icons,
            raised: true,
        });
    }
    let mut host: HWND = null_mut();
    EnumWindows(Some(find_classic), &mut host as *mut HWND as isize);
    if host.is_null() {
        None
    } else {
        Some(Layer {
            host,
            below: HWND_TOP,
            raised: false,
        })
    }
}
pub unsafe fn attach(w: HWND) -> Result<(), String> {
    let prog = FindWindowW(wide("Progman").as_ptr(), null());
    if prog.is_null() {
        return Err("Windows Explorer is unavailable.".into());
    }
    let mut target = layer(prog);
    if target.is_none() {
        for (a, b) in [(0xd, 0), (0xd, 1), (0, 0)] {
            SendMessageTimeoutW(prog, 0x052c, a, b, SMTO_ABORTIFHUNG, 400, null_mut());
            target = layer(prog);
            if target.is_some() {
                break;
            }
        }
    }
    let l = target.ok_or_else(|| "Explorer has not exposed its wallpaper layer.".to_string())?;
    let old_style = GetWindowLongW(w, GWL_STYLE);
    let old_ex = GetWindowLongW(w, GWL_EXSTYLE);
    let old_parent = GetParent(w);
    let child = WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS;
    SetWindowLongW(w, GWL_STYLE, child as i32);
    let ex = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | if l.raised { WS_EX_LAYERED } else { 0 };
    SetWindowLongW(w, GWL_EXSTYLE, ex as i32);
    SetParent(w, l.host);
    let fail = |reason: &str| {
        let error = GetLastError();
        SetParent(w, old_parent);
        SetWindowLongW(w, GWL_STYLE, old_style);
        SetWindowLongW(w, GWL_EXSTYLE, old_ex);
        Err(format!("{reason} (Windows {error})."))
    };
    if GetParent(w) != l.host {
        return fail("Explorer declined attachment");
    }
    if l.raised && SetLayeredWindowAttributes(w, 0, 255, LWA_ALPHA) == 0 {
        return fail("The desktop layer could not be displayed");
    }
    let mut origin = POINT { x: 0, y: 0 };
    ScreenToClient(l.host, &mut origin);
    let flags = SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW;
    if SetWindowPos(
        w,
        l.below,
        origin.x,
        origin.y,
        GetSystemMetrics(SM_CXSCREEN),
        GetSystemMetrics(SM_CYSCREEN),
        flags,
    ) == 0
    {
        return fail("The wallpaper could not be placed");
    }
    Ok(())
}
pub unsafe fn preview(w: HWND) {
    ShowWindow(w, SW_HIDE);
    SetParent(w, null_mut());
    SetWindowLongW(w, GWL_EXSTYLE, WS_EX_APPWINDOW as i32);
    SetWindowLongW(w, GWL_STYLE, (WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN) as i32);
    let r = crate::panel::work_area();
    let width = ((r.right - r.left) * 2 / 3).min(960).max(280);
    let height = (width * 9 / 16 + 35).min(r.bottom - r.top - 20);
    SetWindowPos(
        w,
        HWND_TOP,
        r.left + 12,
        r.top + 24,
        width,
        height,
        SWP_FRAMECHANGED | SWP_SHOWWINDOW | SWP_NOACTIVATE,
    );
}
pub unsafe fn covered(w: HWND, p: HWND, include_maximized: bool) -> bool {
    let fg = GetForegroundWindow();
    if fg.is_null() || fg == w || fg == p || fg == GetShellWindow() {
        return false;
    }
    let mut name = [0u16; 80];
    let n = GetClassNameW(fg, name.as_mut_ptr(), 80);
    if [
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
    ]
    .iter()
    .any(|s| s.encode_utf16().eq(name[..n as usize].iter().copied()))
    {
        return false;
    }
    let mut r: RECT = zeroed();
    if GetWindowRect(fg, &mut r) == 0 {
        return false;
    }
    if include_maximized && IsZoomed(fg) != 0 {
        let mut work: RECT = zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0) != 0
            && r.left <= work.left
            && r.top <= work.top
            && r.right >= work.right
            && r.bottom >= work.bottom
        {
            return true;
        }
    }
    r.left <= 0
        && r.top <= 0
        && r.right >= GetSystemMetrics(SM_CXSCREEN)
        && r.bottom >= GetSystemMetrics(SM_CYSCREEN)
}
