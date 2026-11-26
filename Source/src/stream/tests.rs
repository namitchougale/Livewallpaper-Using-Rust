use super::*;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    process::Command,
    sync::{mpsc, Arc},
    thread,
};

struct Movie {
    bytes: Vec<u8>,
    width: i32,
    height: i32,
    index: usize,
    cycles: usize,
    locks: usize,
    fail: Option<i32>,
}
impl Movie {
    fn bytes_per_frame(&self) -> usize {
        (self.width * self.height * 4) as usize
    }
}
unsafe impl Reader for Movie {
    fn read(&mut self, out: &mut FrameView) -> i32 {
        if let Some(code) = self.fail {
            return code;
        }
        let size = self.bytes_per_frame();
        let offset = self.index * size;
        if offset + size > self.bytes.len() {
            return 1;
        }
        *out = FrameView {
            top: self.bytes[offset..].as_ptr(),
            width: self.width,
            height: self.height,
            stride: self.width * 4,
            aspect: self.width as f64 / self.height as f64,
            pts: self.index as i64 * 10_000_000 / 24,
            duration: 416667,
        };
        self.index += 1;
        0
    }
    fn lock(&mut self, _: &mut FrameView) -> i32 {
        self.locks += 1;
        0
    }
    fn unlock(&mut self) {}
    fn rewind(&mut self) -> i32 {
        self.index = 0;
        self.cycles += 1;
        0
    }
}
#[test]
fn bottom_up_padded_rows_are_presented_top_down() {
    let source = [9, 8, 7, 6, 0, 0, 0, 0, 1, 2, 3, 4, 0, 0, 0, 0];
    let mut frame = Frame::default();
    let v = FrameView {
        top: source[8..].as_ptr(),
        width: 1,
        height: 2,
        stride: -8,
        aspect: 0.5,
        ..Default::default()
    };
    unsafe { frame.update(&v).unwrap() }
    assert_eq!(frame.pixels, [1, 2, 3, 4, 9, 8, 7, 6]);
}
#[test]
fn fallback_rgb_is_scaled_before_the_display_buffer_grows() {
    let (width, height) = (1600, 900);
    let mut source = vec![0; width * height * 4];
    for (i, pixel) in source.chunks_exact_mut(4).enumerate() {
        pixel.copy_from_slice(&((i as u32).to_ne_bytes()));
    }
    let mut frame = Frame::default();
    let view = FrameView {
        top: source.as_ptr(),
        width: width as i32,
        height: height as i32,
        stride: (width * 4) as i32,
        aspect: 16. / 9.,
        ..Default::default()
    };
    unsafe {
        frame.update(&view).unwrap();
    }
    assert_eq!(
        (frame.width, frame.height, frame.pixels.len()),
        (1280, 720, 3_686_400)
    );
    let at = (360 * 1280 + 640) * 4;
    assert_eq!(
        u32::from_ne_bytes(frame.pixels[at..at + 4].try_into().unwrap()),
        450 * 1600 + 800
    );
}
#[test]
fn empty_or_failed_input_never_announces_ready() {
    for failure in [None, Some(0x80004005u32 as i32)] {
        let mut reader = Movie {
            bytes: vec![],
            width: 2,
            height: 2,
            index: 0,
            cycles: 0,
            locks: 0,
            fail: failure,
        };
        let mut first = false;
        let result = drive(&mut reader, &Shared::default(), &mut first, |_| {
            panic!("A failed replacement replaced the current wallpaper")
        });
        assert_eq!(result, Err(failure.unwrap_or(NO_FRAMES)));
        assert!(!first);
    }
}
#[test]
fn paused_worker_stops_and_resumes_without_busy_rendering() {
    let player = Arc::new(Shared::default());
    let worker = player.clone();
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut file = Movie {
            bytes: vec![42; 4 * 6],
            width: 1,
            height: 1,
            index: 0,
            cycles: 0,
            locks: 0,
            fail: None,
        };
        drive(&mut file, &worker, &mut false, |first| {
            tx.send(first).unwrap()
        })
        .unwrap();
    });
    assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(90)).is_err());
    player.running(true);
    assert!(!rx.recv_timeout(Duration::from_secs(2)).unwrap());
    player.running(false);
    while rx.try_recv().is_ok() {}
    assert!(rx.recv_timeout(Duration::from_millis(120)).is_err());
    player.stop();
    handle.join().unwrap();
}
#[test]
#[ignore = "requires FFmpeg and FLOW_TEST_MP4 pointing to the bundled 24 FPS clip"]
fn bundled_mp4_frames_loop_through_flow() {
    let sample = std::env::var_os("FLOW_TEST_MP4").expect("set FLOW_TEST_MP4");
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(sample)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            "scale=96:54",
            "-pix_fmt",
            "bgra",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let mut reader = Movie {
        bytes: output.stdout,
        width: 96,
        height: 54,
        index: 0,
        cycles: 0,
        locks: 0,
        fail: None,
    };
    let frames = reader.bytes.len() / reader.bytes_per_frame();
    assert_eq!(frames, 120);
    let state = Shared::default();
    state.running(true);
    let mut hashes = Vec::new();
    let per_loop = DISPLAY_FPS as usize * 5;
    let start = Instant::now();
    drive(&mut reader, &state, &mut false, |first| {
        assert_eq!(first, hashes.is_empty());
        let mut h = DefaultHasher::new();
        state.frame.lock().unwrap().pixels.hash(&mut h);
        hashes.push(h.finish());
        if hashes.len() == per_loop + 2 {
            state.stop();
        }
    })
    .unwrap();
    assert_eq!(reader.cycles, 1);
    assert_eq!(reader.locks, hashes.len());
    assert_eq!(hashes[0..2], hashes[per_loop..]);
    assert_ne!(hashes[0], hashes[per_loop / 2]);
    assert!(start.elapsed() >= Duration::from_millis(4950));
    println!("{frames} source frames, {per_loop} displayed per five-second loop");
}
