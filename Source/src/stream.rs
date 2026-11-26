use crate::geometry::{render_size, DISPLAY_FPS};
use std::{
    ptr,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};
#[cfg(test)]
mod tests;
const NO_FRAMES: i32 = 0xc00d36beu32 as i32;
pub const OUT_OF_MEMORY: i32 = 0x8007000eu32 as i32;
#[repr(C)]
#[derive(Default)]
pub struct FrameView {
    top: *const u8,
    width: i32,
    height: i32,
    stride: i32,
    aspect: f64,
    pts: i64,
    duration: i64,
}
#[derive(Default)]
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub aspect: f64,
    pub pixels: Vec<u8>,
}
impl Frame {
    unsafe fn update(&mut self, v: &FrameView) -> Result<(), i32> {
        let (w, h) = render_size(v.width, v.height).ok_or(NO_FRAMES)?;
        if v.top.is_null() || i64::from(v.stride).abs() < i64::from(v.width) * 4 {
            return Err(NO_FRAMES);
        }
        let row = w as usize * 4;
        let size = row * h as usize;
        self.pixels
            .try_reserve_exact(size.saturating_sub(self.pixels.len()))
            .map_err(|_| OUT_OF_MEMORY)?;
        self.pixels.resize(size, 0);
        if h == v.height && v.stride == row as i32 && w == v.width {
            ptr::copy_nonoverlapping(v.top, self.pixels.as_mut_ptr(), self.pixels.len());
        } else {
            for (y, dst) in self.pixels.chunks_exact_mut(row).enumerate() {
                let sy = y * v.height as usize / h as usize;
                let source = v.top.offset(sy as isize * v.stride as isize);
                if w == v.width {
                    ptr::copy_nonoverlapping(source, dst.as_mut_ptr(), row);
                } else {
                    // Basic Windows decoders can return native-resolution RGB. Scale only published frames.
                    for (x, pixel) in dst.chunks_exact_mut(4).enumerate() {
                        let sx = x * v.width as usize / w as usize;
                        ptr::copy_nonoverlapping(source.add(sx * 4), pixel.as_mut_ptr(), 4);
                    }
                }
            }
        }
        self.width = w;
        self.height = h;
        self.aspect = v.aspect;
        Ok(())
    }
}
#[derive(Default)]
struct Gate {
    run: bool,
    stop: bool,
}
#[derive(Default)]
pub struct Shared {
    gate: Mutex<Gate>,
    signal: Condvar,
    pub frame: Mutex<Frame>,
    pub error: Mutex<i32>,
}
impl Shared {
    // Only time spent playing counts toward the next frame deadline.
    fn wait(&self, mut remaining: Duration) -> bool {
        let mut gate = self.gate.lock().unwrap();
        loop {
            if gate.stop {
                return false;
            }
            if !gate.run {
                gate = self.signal.wait(gate).unwrap();
                continue;
            }
            if remaining.is_zero() {
                return true;
            }
            let start = Instant::now();
            let (next, _) = self.signal.wait_timeout(gate, remaining).unwrap();
            remaining = remaining.saturating_sub(start.elapsed());
            gate = next;
        }
    }
    pub fn stopped(&self) -> bool {
        self.gate.lock().unwrap().stop
    }
}
impl Shared {
    pub fn running(&self, on: bool) {
        let mut g = self.gate.lock().unwrap();
        if g.run != on {
            g.run = on;
            self.signal.notify_all();
        }
    }

    pub fn stop(&self) {
        let mut g = self.gate.lock().unwrap();
        g.stop = true;
        self.signal.notify_all();
    }
}
fn ticks(n: i64) -> Duration {
    Duration::from_nanos((n.max(0) as u64).saturating_mul(100))
}
/// Read returns sample timing first. Lock exposes pixels only for a selected sample.
/// # Safety
/// On successful lock, all rows must hold width * 4 readable bytes through unlock
/// or the next read. Dimensions and signed row offsets must fit in isize.
pub unsafe trait Reader {
    fn read(&mut self, view: &mut FrameView) -> i32;
    fn lock(&mut self, view: &mut FrameView) -> i32;
    fn unlock(&mut self);
    fn rewind(&mut self) -> i32;
}
pub fn drive(
    reader: &mut impl Reader,
    state: &Shared,
    first_shown: &mut bool,
    mut present: impl FnMut(bool),
) -> Result<(), i32> {
    let mut previous_pts = None::<i64>;
    let mut previous_duration = 333333;
    let mut timeline = 0i64;
    let mut last_slot = None;
    let mut last_published = 0i64;
    let mut last_clock = Instant::now();
    let mut empty_reads = 0;
    loop {
        if state.stopped() {
            return Ok(());
        }
        if *first_shown && !state.wait(Duration::ZERO) {
            return Ok(());
        }
        let mut v = FrameView::default();
        let code = reader.read(&mut v);
        match code {
            0 => {}
            1 => {
                if previous_pts.is_none() {
                    return Err(NO_FRAMES);
                }
                timeline = timeline.saturating_add(previous_duration);
                let tail = timeline.saturating_sub(last_published);
                if !state.wait(ticks(tail).saturating_sub(last_clock.elapsed())) {
                    return Ok(());
                }
                let hr = reader.rewind();
                if hr < 0 {
                    return Err(hr);
                }
                previous_pts = None;
                continue;
            }
            2 => {
                empty_reads += 1;
                if empty_reads > 4096 {
                    return Err(NO_FRAMES);
                }
                continue;
            }
            hr => return Err(hr),
        }
        empty_reads = 0;
        if let Some(prev) = previous_pts {
            let gap = v.pts.saturating_sub(prev);
            timeline = timeline.saturating_add(if gap > 0 { gap } else { previous_duration });
        }
        previous_pts = Some(v.pts);
        previous_duration = v.duration.max(1);
        // Use media time, with a one-tick allowance for rational frame-rate rounding.
        let slot = timeline
            .saturating_add(1)
            .saturating_mul(i64::from(DISPLAY_FPS))
            / 10_000_000;
        if last_slot == Some(slot) {
            reader.unlock();
            continue;
        }
        let delay = ticks(timeline.saturating_sub(last_published));
        if last_slot.is_some() && !state.wait(delay.saturating_sub(last_clock.elapsed())) {
            return Ok(());
        }
        last_clock = Instant::now();
        let code = reader.lock(&mut v);
        if code < 0 {
            return Err(code);
        }
        let result = unsafe { state.frame.lock().unwrap().update(&v) };
        reader.unlock();
        result?;
        last_slot = Some(slot);
        last_published = timeline;
        let first = !*first_shown;
        *first_shown = true;
        present(first);
    }
}
