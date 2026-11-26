use flow_live_wallpaper::geometry::{placement, Fit, LONG_EDGE, SHORT_EDGE};
use std::{
    ffi::c_void,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, Mutex},
    thread,
};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::PostMessageW,
};

pub const VIDEO_EVENT: u32 = 0x8000 + 16;
pub const READY: isize = 1;
pub const FAILED: isize = 2;
use crate::stream::{drive, Frame, FrameView, Reader, Shared};
// A replacement may keep the previous displayed frame, but only one decoder can own resources.
static DECODER_SLOT: Mutex<()> = Mutex::new(());
extern "C" {
    fn decoder_open(
        path: *const u16,
        enhanced: i32,
        longest: u32,
        shortest: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn decoder_read(decoder: *mut c_void, view: *mut FrameView) -> i32;
    fn decoder_lock(decoder: *mut c_void, view: *mut FrameView) -> i32;
    fn decoder_unlock(decoder: *mut c_void);
    fn decoder_rewind(decoder: *mut c_void) -> i32;
    fn decoder_close(decoder: *mut c_void);
}
struct Decoder(*mut c_void);
impl Decoder {
    fn open(path: &[u16], enhanced: bool) -> Result<Self, i32> {
        let mut raw = ptr::null_mut();
        let code = unsafe {
            decoder_open(
                path.as_ptr(),
                enhanced as i32,
                LONG_EDGE as u32,
                SHORT_EDGE as u32,
                &mut raw,
            )
        };
        if code < 0 {
            Err(code)
        } else {
            Ok(Self(raw))
        }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { decoder_close(self.0) }
    }
}
unsafe impl Reader for Decoder {
    fn read(&mut self, view: &mut FrameView) -> i32 {
        unsafe { decoder_read(self.0, view) }
    }
    fn lock(&mut self, view: &mut FrameView) -> i32 {
        unsafe { decoder_lock(self.0, view) }
    }
    fn unlock(&mut self) {
        unsafe {
            decoder_unlock(self.0);
        }
    }
    fn rewind(&mut self) -> i32 {
        unsafe { decoder_rewind(self.0) }
    }
}
impl Frame {
    unsafe fn draw(&self, dc: HDC, rect: RECT, fit: Fit) {
        let Some((s, d)) = placement(
            (self.width, self.height),
            self.aspect,
            (rect.right - rect.left, rect.bottom - rect.top),
            fit,
        ) else {
            return;
        };
        // Draw the fit-mode bars without clearing the live video underneath.
        if d.x > 0 || d.y > 0 || d.w != rect.right - rect.left || d.h != rect.bottom - rect.top {
            let save = SaveDC(dc);
            ExcludeClipRect(
                dc,
                rect.left + d.x,
                rect.top + d.y,
                rect.left + d.x + d.w,
                rect.top + d.y + d.h,
            );
            FillRect(dc, &rect, GetStockObject(BLACK_BRUSH) as _);
            RestoreDC(dc, save);
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = self.width;
        info.bmiHeader.biHeight = -self.height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        SetStretchBltMode(dc, COLORONCOLOR);
        StretchDIBits(
            dc,
            rect.left + d.x,
            rect.top + d.y,
            d.w,
            d.h,
            s.x,
            s.y,
            s.w,
            s.h,
            self.pixels.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}
pub struct Player {
    pub id: usize,
    pub path: PathBuf,
    pub restoring: bool,
    shared: Arc<Shared>,
}
impl Player {
    pub fn open(path: &Path, id: usize, panel: HWND, restoring: bool) -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let state = shared.clone();
        let file: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let notify = panel as usize;
        thread::Builder::new()
            .name("Flow MP4".into())
            .spawn(move || {
                let _slot = DECODER_SLOT.lock().unwrap();
                let mut first_shown = false;
                let mut result = decode(&file, true, &state, notify, id, &mut first_shown);
                if result.is_err() && !first_shown && !state.stopped() {
                    result = decode(&file, false, &state, notify, id, &mut first_shown);
                }
                if let Err(code) = result {
                    if !state.stopped() {
                        *state.error.lock().unwrap() = code;
                        unsafe {
                            PostMessageW(notify as HWND, VIDEO_EVENT, id, FAILED);
                        }
                    }
                }
            })?;
        Ok(Self {
            id,
            path: path.into(),
            restoring,
            shared,
        })
    }
    pub fn running(&self, on: bool) {
        self.shared.running(on);
    }
    pub fn freeze(&self) {
        self.shared.stop();
    }
    pub fn frozen(&self) -> bool {
        self.shared.stopped()
    }
    pub fn error(&self) -> i32 {
        *self.shared.error.lock().unwrap()
    }
    pub unsafe fn draw(&self, dc: HDC, r: RECT, fit: Fit) {
        self.shared.frame.lock().unwrap().draw(dc, r, fit);
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.shared.stop();
    }
}
fn decode(
    path: &[u16],
    enhanced: bool,
    state: &Shared,
    panel: usize,
    id: usize,
    first_shown: &mut bool,
) -> Result<(), i32> {
    if state.stopped() {
        return Ok(());
    }
    let mut decoder = Decoder::open(path, enhanced)?;
    drive(&mut decoder, state, first_shown, |first| unsafe {
        if first {
            PostMessageW(panel as HWND, VIDEO_EVENT, id, READY);
        } else {
            let wall = crate::wall();
            if !wall.is_null() {
                InvalidateRect(wall, ptr::null(), 0);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{budget, geometry::DISPLAY_FPS};
    use std::{
        hash::{DefaultHasher, Hash, Hasher},
        time::Instant,
    };

    #[test]
    #[ignore = "native Windows Media Foundation and process quota integration"]
    fn native_mp4_under_budget() {
        let limits = budget::Budget::install().expect("Windows process limits");
        budget::verify_allocation_limit(&limits);
        let clip = std::env::var_os("FLOW_TEST_MP4")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("Wallpapers")
                    .join("Violet Drift.mp4")
            });
        let name: Vec<_> = clip.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut decoder = Decoder::open(&name, true)
            .or_else(|_| Decoder::open(&name, false))
            .expect("open bundled H.264 MP4");
        let state = Shared::default();
        state.running(true);
        let frames_per_loop = DISPLAY_FPS as usize * 5;
        let mut hashes = Vec::new();
        let mut peak_commit = 0;
        let start = Instant::now();
        drive(&mut decoder, &state, &mut false, |_| {
            let frame = state.frame.lock().unwrap();
            assert_eq!((frame.width, frame.height), (960, 540));
            let mut h = DefaultHasher::new();
            frame
                .pixels
                .chunks(4096)
                .for_each(|p| p[..p.len().min(4)].hash(&mut h));
            hashes.push(h.finish());
            let memory = budget::usage().unwrap();
            peak_commit = peak_commit.max(memory.PrivateUsage);
            assert!(
                memory.WorkingSetSize <= budget::MAX_BYTES
                    && memory.PrivateUsage <= budget::MAX_BYTES
            );
            if hashes.len() == frames_per_loop + 1 {
                state.stop();
            }
        })
        .unwrap();
        assert_ne!(hashes[0], hashes[frames_per_loop / 2]);
        assert_eq!(hashes[0], hashes[frames_per_loop]);
        println!("Presented {} frames in {:.2}s. Peak working set {:.1} MB, sampled peak private commit {:.1} MB.",
            hashes.len(), start.elapsed().as_secs_f64(), budget::usage().unwrap().PeakWorkingSetSize as f64/1e6,
            peak_commit as f64/1e6);
    }
}
pub fn error_message(code: i32) -> String {
    let reason = match code as u32 {
        0x8007007e | 0x8007007f => {
            "Windows media components are missing. On Windows N, install Media Feature Pack."
        }
        0xc00d5212 | 0xc00d36b4 => "Windows could not decode the video track. Try an H.264 MP4.",
        0xc00d36c4 | 0xc00d36be => {
            "Windows could not read this MP4. It may be incomplete or unsupported."
        }
        0xc00d36b3 => "This MP4 has no readable video track.",
        0x80070002 | 0x80070003 => "This file was moved or deleted.",
        0x80070005 => "Windows denied access to this file.",
        0x8007000e | 0x80070008 | 0x80070718 | 0x800705af => {
            "This video needs more memory than Flow can use. Try a smaller H.264 MP4."
        }
        _ => "Windows could not play this MP4.",
    };
    format!("{reason}  ·  0x{:08X}", code as u32)
}
