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
   sidebar never resizes the stage (it overlays it).
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
   `i18n.t("中文", "English")` pair at the place it is used.

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

- **Sidebar** (`shell/sidebar.rs`). Collapsed: 52px icon rail. Expanded:
  256px panel that floats over the stage with a shadow; a transparent scrim
  closes it. Top row: logo (click to expand), brand, collapse button. Nav rows
  (36px, radius 8, icon + label): Remote desktop, Device terminal, Settings.
  Footer: theme, language, connection status (dot + label + host). Rows are
  disabled until connected.
- **Titlebar** (`shell/titlebar.rs`). 48px (Figma uses 68px; KVM gives the
  space to the video). Left: page title and a chip (`1920×1080 · 60 fps` or
  "无信号"). Right: page actions, grouped with `.vsep` hairlines, destructive
  action last.
- **Stage**. Near-black in both themes. The picture keeps its aspect ratio
  (`object-fit: contain`); pointer positions map through the letterboxed
  content box (`hid::content_rect`).
- **Gates** (setup, sign-in, connect) are a centered card on
  `--bg-secondary` over the stage, so the video element stays mounted.
- **Focus mode** hides sidebar and titlebar. A small handle at the top center
  peeks the titlebar back in as a floating bar; it hides 600ms after the
  pointer leaves.
- **Sheets and dialogs** (settings, OCR) sit above everything on a scrim.

Remote-desktop titlebar actions, in order: Num/Caps/Scroll LED badges · mouse
mode (absolute/relative) · quality − % + · audio | Ctrl+Alt+Del · OCR · focus
mode · fullscreen | reboot (danger).

## Components (`src/ui`)

| Component | Spec | Use |
| --- | --- | --- |
| `Icon` | 20×20 viewBox, 1.5px round stroke, `currentColor`; add new glyphs to `IconName` | every icon |
| `Button` | `Primary` (inverse fill), `Secondary` (hairline), `Ghost`; 36px, or 32px `small`; `block` for full width | text actions |
| `IconButton` | 32px (28px `small`), radius 8/6, ghost; `active` → selected fill; `danger` → red glyph; `label` is tooltip and aria-label (required) | toolbar actions |
| `Field` | 12px label above, control below, 16px bottom margin; inputs 40px, radius 8, 1px `--border-secondary`, focus ring | forms |
| `Callout` | tinted fill, hairline, leading alert icon, 12px text | warnings (no-password) |
| `.toggle` | switch 36×20, knob 16px, off = tertiary + hairline, on = `--border-info` | boolean settings |
| `.badge` | 20px mono, hairline; `.on` = success tint | LEDs |
| `.chip` | 24px pill, hover fill, tabular numbers | status in titlebar |
| `.dot` | 8px; `.on` green, `.warn` amber, default grey | connection state |
| `.toast` | bottom-center, popover shadow, auto-clears after 6s | transient errors |
| `.spinner` | 14px, `currentColor` | in-progress buttons |

Rules:

- Event handlers go straight on components: `<IconButton … on:click=handler/>`.
- One primary button per card. Secondary next to it, ghost elsewhere.
- Icon-only buttons always carry a translated `label`.
- Disabled is 50% opacity and no hover, never hidden.

## Copy

- Chinese first; English is the alternative, never mixed in one string.
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
- **Phase 2**: settings as a Claude-style page (section list on the left,
  content on the right) built from `Field`, `Button` and a `Switch` component;
  zh/en for all settings.
- **Phase 3**: virtual media page, terminal and OCR polish, remaining copy.
