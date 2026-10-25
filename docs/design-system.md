# rustkvm web UI design system

The web UI follows the layout and visual language of Claude Desktop: a quiet,
warm, low-contrast frame around the one thing that matters, the remote screen.
This document is the rule book. Any change to `crates/rkvm-web` follows it; if
a change needs something it does not cover, extend this file in the same PR.

Sources: the Claude Desktop design kit in Figma (colors, type, borders, buttons,
inputs, switch, sidebar, titlebar) and the design notes of
[open-claude-ui](https://github.com/Caho1/open-claude-ui) (philosophy only, no
code taken).

## Principles

1. **The video comes first.** Nothing may cost the video path a frame. The
   `<video>` element is mounted once and never unmounted or re-parented; no
   `backdrop-filter`, blur or animated shadow sits over the stage; opening the
   sidebar resizes the stage in one step, never through an animation.
2. **Quiet chrome.** Warm off-white and near-black surfaces, hairline (0.5px)
   borders, one accent used sparingly. Color carries meaning (danger, success,
   info), never decoration.
3. **One border per surface.** A bordered or shadowed surface (card, sheet,
   popover) never contains another bordered box. Controls inside it are ghost
   or filled.
4. **Same thing, same look.** A button, field, switch or icon looks and behaves
   identically on every page. Use the components in `src/ui`; never restyle a
   one-off.
5. **Fine-grained updates.** Leptos signals drive exactly the text or attribute
   that changes. Read signals inside `move ||` closures at the leaf; don't
   rebuild a subtree for a value that changes often (video stats, LEDs,
   quality).
6. **Bilingual by construction.** Every visible string is a
   `i18n.t("中文", "English")` pair at the place it is used, and the UI never
   mixes the two languages.

## Tokens

All values are CSS custom properties in `style/tokens.css`. No other file may
contain a raw color, font size, radius or duration, apart from the two
documented exceptions: the stage and the terminal, which are always dark.

### Color

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `--bg-primary` | `#ffffff` | `#30302e` | cards, sheets, inputs |
| `--bg-secondary` | `#f5f4ed` | `#262624` | page background, gates |
| `--bg-tertiary` | `#faf9f5` | `#141413` | wells (OCR text, switch track) |
| `--bg-sidebar` | `#f9f9f7` | `#1f1e1d` | sidebar and titlebar |
| `--bg-selected` | `#f0efec` | `#30302e` | selected nav row, active icon button |
| `--bg-hover` / `--bg-press` | 5% / 8% ink | 6% / 10% ink | hover and press fills |
| `--bg-inverse` | `#141413` | `#faf9f5` | primary button |
| `--text-primary` | `#141413` | `#faf9f5` | body text |
| `--text-secondary` | `#3d3d3a` | `#c2c0b6` | labels, idle icons |
| `--text-tertiary` | `#73726c` | `#9c9a92` | hints, captions, placeholders |
| `--border-primary/secondary/tertiary` | ink at 40/30/15% | ink at 40/30/15% | hairlines, strongest to faintest |
| `--border-info` | `#4682d5` | `#4682d5` | switch on, links |
| `--bg-danger` / `--text-danger` | `#b53333` / `#7f2c28` | `#dd5353` / `#ee8884` | destructive actions, errors |
| `--bg-success` / `--text-success` | `#e9f1dc` / `#265b19` | `#1b4614` / `#7ab948` | lit LED badges |
| `--accent` | `#c6613f` | `#c6613f` | brand mark only |
| `--stage` | `#141413` | `#141413` | video background, both themes |

Focus is always visible: `--focus-ring` is a 2px gap in the surface color plus
a 1.5px info ring. Primary buttons use `--ring-primary`, danger uses
`--ring-danger`.

### Type

System sans with CJK fallbacks (`PingFang SC`, `Microsoft YaHei`, `Noto Sans CJK
SC`), monospace `JetBrains Mono` stack.

| Role | Size / line | Weight |
| --- | --- | --- |
| Card title | 20 / 25 | 600 |
| Page title (titlebar), sheet title | 16 / 22.4 | 500–600 |
| Body, nav rows, inputs | 14 / 19.6 | 400 |
| Labels, hints, captions, small buttons | 12 / 16.8 | 400–600 |
| LED badges | 10 mono | 600 |

### Spacing, radius, motion

- Spacing on a 4px grid: 4, 8, 12, 16, 20, 24, 32.
- Radius: 4 (badges), 6 (small buttons), 8 (buttons, inputs, nav rows), 10
  (callouts), 12 (toast, popovers), 16 (cards), full (switch, chips).
- Motion: `--ease` `cubic-bezier(0.16, 1, 0.3, 1)` at 150ms for state changes,
  ~200ms for panels. Press feedback is `scale(.98)`. Everything respects
  `prefers-reduced-motion`.
- Shadows: `--shadow-lg` for cards and sheets, `--shadow-popover` for the
  toast and the focus-mode titlebar. Never stack a border and a shadow.

### Theme

Light is the default palette; `:root[data-theme="dark"]` swaps it. An inline
script in `index.html` sets `data-theme` before first paint from the saved
choice (`localStorage["rkvm.theme"]`) or the OS setting, and follows OS changes
until the user picks one. `src/theme.rs` flips it from the sidebar.

## Layout

```
┌──────┬─────────────────────────────────────────────────────┐
│ rail │ titlebar 48px: title · chip        actions (right)  │
│ 52px ├─────────────────────────────────────────────────────┤
│      │                                                     │
│ nav  │ stage: <video> (object-fit: contain), always        │
│      │ mounted; gates, toast, callout, terminal overlay it │
│      │                                                     │
│ foot │                                                     │
└──────┴─────────────────────────────────────────────────────┘
```

- **Sidebar** (`shell/sidebar.rs`). Collapsed: 52px icon rail.
  - *Peek*: hovering the rail (120ms in, 300ms out) shows the 256px sidebar
    floating over the content with a shadow; leaving hides it. It stays while
    keyboard focus (`:focus-visible`) is inside, and Tab into the rail opens
    it.
  - *Pinned*: clicking the logo or the sidebar button keeps it open and
    pushes the content as Claude Desktop does (`.app.sb-open` widens the
    first grid column), so the titlebar chip and actions are never covered.
    The column changes in one step, so the video rescales once. Below 720px
    a pinned sidebar floats and a transparent scrim closes it.
  - `.expanded` (pinned or peeking) shows labels; `.open` is pinned only. Top row: logo (click to expand), brand,
  collapse button. Nav rows (36px, radius 8, icon + label): Remote desktop,
  Device terminal, Settings. Footer: connection status (dot + label + host)
  only. Rows are disabled until connected. The sidebar holds navigation and
  status, nothing else.
- **Preferences and device actions live in Settings**, as in Claude
  Desktop: theme (follow system / light / dark) and language on 通用,
  reboot on 系统 with a confirm. Preferences show the current state with a
  segmented control; never label a toggle with the value it switches to.
  Destructive actions are never in a titlebar corner, where a near miss of
  fullscreen or focus lands on them.
- **Titlebar** (`shell/titlebar.rs`). 48px (Figma uses 68px; KVM gives the
  space to the video). Left: page title and a chip (`1920×1080 · 60 fps` or
  "无信号"). Right: page actions, grouped with `.vsep` hairlines. No
  destructive action.
- **Stage**. Near-black in both themes. The picture keeps its aspect ratio
  (`object-fit: contain`); pointer positions map through the letterboxed
  content box (`hid::content_rect`).
- **Gates** (setup, sign-in, connect) are a centered card on
  `--bg-secondary` over the stage, so the video element stays mounted.
- **Focus mode** hides sidebar and titlebar. A small handle at the top center
  peeks the titlebar back in as a floating bar; it hides 600ms after the
  pointer leaves. When the sidebar is hidden, `.app.focus` switches the grid to
  a single `minmax(0, 1fr)` track: a hidden grid item frees its track, so
  keeping the two-column template would auto-place `.main` into the 0px
  column and collapse the whole stage into the top-left corner.
- **Settings is a modal dialog** (`settings.rs`, `settings_nav.rs`), as in
  Claude Desktop: centered panel (max 960×720, radius 16) over a scrim that
  covers the whole app, sidebar included (z 50/51). Left column (240px,
  sidebar color): search box, then the pages (通用, 视频, 键盘与鼠标,
  虚拟媒体, 网络, 访问与安全, 硬件, 系统). Right column: page title and that page's
  sections; close button top right. Esc, the close button and a click on
  the backdrop close it. Search matches section and page titles and lists
  every hit across pages. Sections stay mounted and are hidden with
  `section.off`; a new section is added to `SECTIONS` in `settings_nav.rs`.
  Below 720px the dialog is full screen and the pages become a horizontal
  strip.
- **Other dialogs** (OCR) sit above the content on a scrim that starts right
  of the rail.
- **OCR dialog** (`ocr.rs` state, markup in `app.rs`). Title "识别到的文字". A
  status line (`.ocr-status`, `aria-live`) under the title says what is
  happening, never a blank box:
  - reading: spinner + "正在识别屏幕文字…"; before the first finished read
    of the page it adds that the models load first and take a few seconds;
  - done: "56 行 · 1.2 秒", the time the user waited, plus "· 全分辨率" when
    the device re-read at native resolution and "· 画面未变，沿用上次结果"
    when it answered from its cache; zero lines says so in words;
  - errors are actionable: no models installed points to docs/vision.md and
    the install path; other device errors show the device message.
  The text well (textarea, mono) appears only when there is text. Actions:
  关闭 (secondary), 重新识别 (secondary, disabled while reading), 复制
  (primary, disabled without text); "已复制" joins the status line. Esc
  closes; a read still running when the dialog closes is dropped.

Remote-desktop titlebar actions, in order: Num/Caps/Scroll LED badges |
mouse mode (absolute/relative) · quality − % + · audio | Ctrl+Alt+Del · OCR |
focus mode · fullscreen. Focus mode and fullscreen use clearly different
glyphs (window with chrome marked vs four corners).

### Keyboard ownership

While connected every key goes to the host (Tab, Esc and shortcuts
included), except when the page owns it (`hid::ui_owns_keyboard`):

- a text field, select, the terminal, or anything inside `[data-modal]`
  (settings dialog, OCR dialog);
- chrome (`[data-chrome]`: sidebar, titlebar) focused with the keyboard
  (`:focus-visible`). A button that was only clicked keeps focus but not
  `:focus-visible`, so typing after a click still reaches the host.

Clicking the video focuses it (`tabindex=-1`). Sheets and dialogs take focus
when they open and close on Esc. A key pressed for the host is always
released to the host, even if focus moved meanwhile. Focus mode does not
take Esc, because the host needs it; leave it from the top handle.

### Targets

Pointer targets are 32px (28px small) for mouse. Under `(pointer: coarse)`
every button, nav row and segment is at least 44px.

## Components (`src/ui`)

| Component | Spec | Use |
| --- | --- | --- |
| `Icon` | 20×20 viewBox, 1.5px round stroke, `currentColor`; add new glyphs to `IconName` | every icon |
| `Button` | `Primary` (inverse fill), `Secondary` (hairline), `Ghost`; 36px, or 32px `small`; `block` for full width | text actions |
| `IconButton` | 32px (28px `small`), radius 8/6, ghost; `active` → selected fill; `danger` → red glyph; `label` is tooltip and aria-label (required) | toolbar actions |
| `Field` | 12px label above, control below, 16px bottom margin; inputs 40px, radius 8, 1px `--border-secondary`, focus ring | forms |
| `Callout` | tinted fill, hairline, leading alert icon, 12px text | warnings (no-password) |
| `.toggle` | switch 36×20, knob 16px, off = `--border-tertiary` track, on = `--border-info`; hover darkens the track, press stretches the knob, focus ring, disabled 50%. In settings the label sits left and the switch right | boolean settings |
| `.seg` | segmented radio group: `button[role=radio]`, `.on` + `aria-checked` on the current one (raised `--bg-primary` chip), ghost hover on the others, press `scale(.97)`, focus ring. Icon-only segments carry `title` and `aria-label` | one-of-few choices (theme, language, codec, rotation) |
| `.set-row` | label + hint left, control right | settings rows |
| `.badge` | 20px mono, hairline; `.on` = success tint | LEDs |
| `.chip` | 24px pill, hover fill, tabular numbers | status in titlebar |
| `.dot` | 8px; `.on` green, `.warn` amber, default grey | connection state |
| `.toast` | bottom-center, popover shadow, auto-clears after 6s | transient errors |
| `.spinner` | 14px, `currentColor` | in-progress buttons |

Rules:

- Controls update optimistically: set the signal first, then send with
  `util::fire_revert`, which restores the old value and shows the toast if
  the device refuses. `util::fire` reports failures too. The toast sits
  above dialogs (z 60).
- Selected state must survive hover: hover styles never apply to the
  selected segment. Plain settings buttons use a `:where()` selector so
  component classes override them without `!important`.
- Event handlers go straight on components: `<IconButton … on:click=handler/>`.
- One primary button per card. Secondary next to it, ghost elsewhere.
- Icon-only buttons always carry a translated `label`.
- Disabled is 50% opacity and no hover, never hidden.

## Copy

- One language at a time: with 中文 selected every visible string is Chinese,
  with English every string is English. That includes settings, option
  labels (keyboard layouts, extensions), status lines built from device
  replies, confirm dialogs and error messages. Only proper nouns and
  protocol names stay as they are (H.264, EDID, MQTT, HTTPS, NUM/CAPS/SCR).
- Views use `i18n.t(zh, en)` inside a closure so text switches live. Code
  without a reactive owner (async results, `util::fire_status`, `net/`
  errors) uses `i18n::tr(zh, en)`. Device replies are stored as data and
  described at render time, so they follow a language switch too.
- Short, plain, sentence case. Name the action on the button ("连接", "完成设置",
  "复制").
- Warnings say what can happen and what to do: "未设置密码：…请在 设置 → 密码 中设置。"
- Navigation paths use `→`: "设置 → USB 设备".
- No emoji or symbol glyphs as icons (`⚙`, `⏻`, `✕`); use `Icon`.

## Performance rules

- `<video>` lives in `.stage` permanently; never inside `<Show>`.
- No `backdrop-filter`, `filter` or animated `box-shadow` over the stage.
- Overlays above the video are solid surfaces.
- Values that change often (fps, LEDs, quality, audio state) are read at the
  leaf text node or attribute.
- CSS is static files linked by Trunk (hashed, cacheable); no runtime style
  generation.

## Roadmap

- **Phase 1** (this spec's first PR): tokens, base components, the shell
  (sidebar, titlebar, stage, gates, toast, focus mode), zh/en for the shell,
  token restyle of settings sheet, terminal and OCR dialog.
- **Phase 2**: settings rows in the Claude style (label and hint left,
  control right, hairline between rows) built from `Field`, `Button` and a
  `Switch` component. (zh/en and the settings dialog landed in phase 1.)
- **Phase 3**: virtual media page, terminal and OCR polish, remaining copy.
