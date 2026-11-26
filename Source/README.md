# Flow Live Wallpaper

Flow plays local MP4 wallpapers behind the desktop icons on the primary display in Windows 10 and 11 x64. It uses a native Rust control panel and the Windows media decoder. The portable app runs without installing Rust.

[Download Flow 1.2 for Windows x64](./dist/Flow-Live-Wallpaper-1.2-Windows-x64.zip?raw=1)

![A still from Violet Drift, the purple landscape included with Flow](docs/violet-drift.jpg)

## Try it

1. Quit an older copy of Flow, extract the ZIP, and double-click **FlowLiveWallpaper.exe**.
2. Click **Add MP4** or drop your video from File Explorer onto the panel. You can also pick **Violet Drift** to try the bundled H.264 video, or choose one of the three animated scenes.
3. Click **Apply to desktop**. The **Desktop** badge confirms attachment. **Options → Open preview window** plays the same wallpaper in a window.

**Fill** crops the edges, **Fit** keeps the whole image visible, and **Stretch** uses the full screen. Minimize the panel to access Flow from the system tray. **Pause** holds the current frame and **Quit** closes the app. Files stay in their original locations.

## Resource use

| What Flow controls | Behavior in 1.2 |
| --- | --- |
| Process memory | Windows enforces a 140 MB ceiling on private committed memory and the process working set. Flow rounds the limit down to a memory page and checks that the hard working-set maximum is active before opening the panel. GPU memory and the Windows desktop compositor have separate accounting. |
| Video | Playback targets 12 displayed frames per second. Flow keeps a display frame of at most 1280 × 720, or 720 × 1280 in portrait. Only one decoder runs during wallpaper changes. |
| Built-in scenes | A reusable bitmap capped at the same dimensions replaces the full-screen bitmap that 1.1 created on each frame. |
| Background use | Playback pauses when a maximized or fullscreen foreground app covers the primary desktop, the session locks, or the display switches off. A detached wallpaper holds a still frame until it is applied or previewed. |

On the five-second, 24 fps bundled sample, the playback test counts **60 displayed frame copies in 1.2**, down from **120 in 1.1**, at the same playback duration. For a 1920 × 1080 RGB image, each application display frame drops from 8.29 MB to 3.69 MB. The codec still reads the source video. Actual CPU use depends on the clip, installed decoder, and hardware.

**Options** shows current process RAM and private commit in decimal MB. If a clip exceeds the budget, try a 720p H.264 MP4. Windows N editions may require the Media Feature Pack.

## Build and test

Use a Windows x64 developer terminal with Rust, the Visual Studio C++ build tools, and the Windows SDK. Run these commands from the folder containing `Cargo.toml`.

```powershell
cargo build --release --locked
cargo test --locked --lib
```

The Rust modules in `src/` handle the desktop layer, control panel, library, render timing, and memory limits. `video.cpp` is a small Media Foundation bridge. The window manager and the media decoder use the system components already on Windows.

The portable tests cover MP4 selection, looping, pause, image layout, and bounded frame copies. To include the real sample in the portable test, put FFmpeg on PATH, set `FLOW_TEST_MP4` to its full path, and use `cargo test --locked --lib -- --include-ignored`.

On Windows, the following checks request the process limits, exercise the native H.264 decoder, then measure eight seconds of playback and five seconds of pause in a separate preview session. The measurement uses a temporary settings folder.

```powershell
cargo test --locked --release --bin flow-live-wallpaper native_mp4_under_budget -- --ignored --nocapture --test-threads=1
.\scripts\Measure-Flow.ps1 -Exe .\target\release\flow-live-wallpaper.exe -Mp4 '.\Wallpapers\Violet Drift.mp4'
```
