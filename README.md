Flow Live Wallpaper (made using rust)

extract everything into one folder and open FlowLiveWallpaper.exe. 

Click Add MP4 or drop a local MP4 onto the panel. Click Apply to desktop and look for the Desktop badge. Violet Drift is a bundled H.264 MP4. Polar Aurora, Ember Dunes and Midnight Tide are built-in scenes.

Flow targets 12 displayed frames per second at up to 720p. The 140 MB limit applies to process memory. Options shows current RAM and private commit. Playback pauses when a foreground app covers the primary desktop, your session locks or the display turns off. Minimize keeps Flow in the tray. Quit closes it.

Source contains the Rust project and build instructions. To measure the app, quit Flow first and run the following from PowerShell in the extracted folder.

Evidence:
<img src="Source/Screenshot 2026-11-26 211022.png" alt="image">


    .\Source\scripts\Measure-Flow.ps1 -Exe .\FlowLiveWallpaper.exe -Mp4 '.\Wallpapers\Violet Drift.mp4'

-made using the help of ai
