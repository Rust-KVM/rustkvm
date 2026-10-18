# Demo media

Screenshots and a short clip of the RustKVM web UI (English, light and dark
themes), used by the top-level README.

| File | What it shows |
| --- | --- |
| `light-01-connect.webp` | Connect card on a fresh page load |
| `{light,dark}-02-remote-desktop.webp` | Remote desktop: live target screen, titlebar with LEDs and actions |
| `{light,dark}-03-sidebar-pinned.webp` | Sidebar pinned open (navigation and connection status) |
| `{light,dark}-04-settings-general.webp` | Settings dialog, General page (appearance and language) |
| `{light,dark}-09-focus-mode.webp` | Focus mode: sidebar and titlebar hidden |
| `rustkvm-demo.gif` | 11 s loop: pin sidebar, open Settings, switch to dark, close, unpin (1280 px wide) |
| `rustkvm-demo.webm` | The same clip at 1920×1080 (VP9) |

## How they were made

- Device: RK3588 running RustKVM, capturing a Windows 11 target at
  3840×2160, 60 fps.
- Browser: headless Chromium driven by Playwright at 1920×1080, device scale
  factor 1. Theme and language were set in the browser's localStorage only.
- The scripts clicked only UI elements (sidebar, titlebar, settings dialog)
  and sent no keyboard or mouse input to the target.
- Stills were saved as WebP (quality 85). The clip was recorded with the
  Chrome DevTools screencast and encoded with ffmpeg; the GIF drops frames
  identical to the previous one, because the UI changes in single steps.

## Sources and licenses

- **RustKVM UI**: this repository, GPL-2.0-only.
- **Target screen**: Google Chrome on Windows 11 showing this repository's
  GitHub page (<https://github.com/Rust-KVM/rustkvm>). The page content is this
  project's README and social preview image (GPL-2.0-only). Windows, Chrome and
  GitHub interface elements belong to Microsoft, Google and GitHub, and appear
  here as ordinary product screenshots.
- No third-party photos, videos or wallpapers are visible.
