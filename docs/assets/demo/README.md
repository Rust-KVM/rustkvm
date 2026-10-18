# Demo media

Screenshots and two short clips of the RustKVM web UI (English, light and
dark themes), used by the top-level README.

| File | What it shows |
| --- | --- |
| `light-01-connect.webp` | Connect card on a fresh page load |
| `{light,dark}-02-remote-desktop.webp` | Remote desktop: live target screen, titlebar with LEDs and actions |
| `{light,dark}-03-sidebar-pinned.webp` | Sidebar pinned open (navigation and connection status) |
| `{light,dark}-04-settings-general.webp` | Settings dialog, General page (appearance and language) |
| `{light,dark}-09-focus-mode.webp` | Focus mode: sidebar and titlebar hidden |
| `rustkvm-demo.gif` | 17 s control demo: the target's cursor glides over the page, the README scrolls down and back up, the cursor clicks GitHub's search box and types `WebRTC`, Esc closes it (1280 px wide, 12 fps) |
| `rustkvm-demo.webm` | The same clip at 1920×1080 (VP9) |
| `rustkvm-ui-tour.gif` | 11 s loop: pin sidebar, open Settings, switch to dark, close, unpin (1280 px wide) |
| `rustkvm-ui-tour.webm` | The same clip at 1920×1080 (VP9) |

## How they were made

- Device: RK3588 running RustKVM, capturing a Windows 11 target at
  3840×2160, 60 fps.
- Browser: headless Chromium driven by Playwright at 1920×1080, device scale
  factor 1. Theme and language were set in the browser's localStorage only.
- Control demo: every action on the target is real USB HID input sent
  through the web UI. Mouse moves over the video become absolute pointer
  reports, wheel steps become wheel reports, and key presses become keyboard
  reports. Nothing was typed into the target beyond the search query, no
  link was followed, and the page was left as it started. The script paused
  once after the click to check that the search box had opened; that pause
  is cut from the clip.
- UI tour and stills: the scripts clicked only UI elements (sidebar,
  titlebar, settings dialog) and sent no keyboard or mouse input to the
  target.
- Stills were saved as WebP (quality 85). Clips were recorded with the
  Chrome DevTools screencast and encoded with ffmpeg; the GIFs drop frames
  identical to the previous one to stay small.

## Sources and licenses

- **RustKVM UI**: this repository, GPL-2.0-only.
- **Target screen**: Google Chrome on Windows 11 showing this repository's
  GitHub page (<https://github.com/Rust-KVM/rustkvm>). The page content is this
  project's README and social preview image (GPL-2.0-only). Windows, Chrome and
  GitHub interface elements belong to Microsoft, Google and GitHub, and appear
  here as ordinary product screenshots.
- No third-party photos, videos or wallpapers are visible.
