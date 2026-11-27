# Design System — Akira Manga Cyber (400×300 1-bit E-Paper)

> Locked visual specification for `ai-quota-zectrix-dashboard` on ZECTRIX NOTE4 (400×300, pure 1-bit monochrome, `dither=false`).

## 1. Visual World & Polarity Principles

- **Direction**: **Akira Manga Cyber（阿基拉白底机甲）** — a white-ground, high-contrast 1-bit cyberpunk HUD inspired by Japanese mecha manga printmaking and tactical telemetry. White paper ground minimizes e-paper ghosting while heavy 2–3px chamfered armored frames, 1-bit halftone drop shadows, slanted energy cells, and black hazard-striped alert docks deliver unmistakable visual punch from 60cm+ away.
- **Color Tokens (1-bit Binary Only)**:
  - `PAPER` (`BinaryColor::Off` / white pixel): full-screen canvas ground, armored card interiors, inverted text inside black header/tag/alert fills.
  - `INK` (`BinaryColor::On` / black pixel): top telemetry bar, 2–3px chamfered armor borders, 1-bit halftone offset shadows, slanted energy cells, hero numerals, and active reset alert banners.
  - **Zero grayscale, zero anti-aliasing, zero dithering** (`dither=false`).
- **1-bit Graphic Primitives**:
  - **45° Chamfered Frame**: Rectangle with 45° diagonal cuts on top-left and bottom-right corners (`cut = 14px` on primary Codex panel, `10px` on Gemini panels, `6px` on tags/docks).
  - **1-bit Halftone Drop Shadow**: Checkerboard/staggered 1px dot matrix (`step = 3px`, alternating row offset) offset `+6px, +6px` behind each armored card.
  - **Slanted Energy Bar (`////`)**: 10 discrete parallelogram segments (`slant = 5px`, `gap = 3px`), filled solid `INK` for remaining capacity (`ceil(pct / 10)` segments) and 1px `INK` outline for consumed capacity.
  - **Hazard Alert Banner**: 45° diagonal 2px `INK` zebra stripes (`step = 6px`) flanking a centered solid `INK` title/detail plate with `PAPER` text when a reset signal is active.

## 2. Spatial Grid (400 × 300 px)

```text
+=======================================================================+  y = 0
| AI-QUOTA // 01-CODEX 02-GEMINI                        11/27 周五 09:10 |  y = 24 (solid INK)
+=======================================================================+
  /-------------------------------+   /---------------------------+        y = 32
 / [CODEX // WK]          [CR ×1] |  / [GEMINI · 5H]         [旧] |
 |                                |  | 97%                 4h45m  |
 |             66%                |  | //////////                 |
 |                                |  +---------------------------/ ..      y = 126 (+6px halftone)
 |  /////// / / /                 |   /---------------------------+        y = 136
 |  ----------------------------- |  / [GEMINI · WK]              |
 |  重置倒计时  4天6h              |  | 91%                 6天8h  |
 +-------------------------------/   | ///////// /                |
   ...............................   +---------------------------/ ..      y = 230 (+6px halftone)
                                                                           y = 246
 [///][CODEX // 重置额度已发  ][///]   [///][GEMINI // 重置排期  ][///]      y = 246..292
 [///][Banked · 5h前          ][///]   [///][Gemini 今日22:00    ][///]
                                                                           y = 300
 x=10                          x=198  x=210                     x=384
```

## 3. Component & Geometry Specifications

### A. Top Telemetry Bar (`x: 0..400, y: 0..24`)
- Solid `INK` background with `PAPER` text.
- Left (`x = 10, baseline = 16`): `AI-QUOTA // 01-CODEX 02-GEMINI`.
- Right (`x = 390, baseline = 16`): `<MM/DD 周X> <HH:MM>` (current date + last successful screen update time, or `--:--` before first upload).

### B. Left Primary Armored Panel: Codex Weekly (`x: 10..198, y: 32..230`)
- **Shadow**: 1-bit halftone (`step = 3`) at `x: 16..204, y: 38..236`.
- **Frame**: `PAPER` fill + 2.5–3px `INK` stroke, `cut = 14px` (top-left & bottom-right).
- **Header Tag (`x: 18..132, y: 40..58`)**: Chamfered `INK` pill (`cut = 6px`) with `PAPER` text `CODEX // WK`.
- **Optional Right Badges (`x: 140..190, y: 40..58`)**:
  - When stale: outlined `INK` badge `旧`.
  - When `reset_credits > 0`: outlined `INK` badge `CR×N`.
- **Hero Numeral**: Large bold tabular numeral (`66`, `100`) centered at `x = 98, baseline = 146` with `%` at `x = 164` (`u8g2_font_logisoso42_tn` or `logisoso50_tn`/`logisoso58_tn`).
- **Slanted Energy Bar**: `x: 20..188, y: 164..184` (height `20px`, 10 slanted segments).
- **Footer Readout**: 2px `INK` rule at `y = 194`, followed by `重置倒计时  <countdown>` at `x = 20, baseline = 214`.
- **Unavailable State**: Keep armored frame + header tag; center reads `暂不可用` / `等待 Codex` (never `0%`).

### C. Right Twin Armored Panels: Gemini 5h & Weekly (`x: 210..384`)
- **Upper Panel (`y: 32..126`, height `94px`)**: `GEMINI · 5H` (Gemini 5-hour window).
- **Lower Panel (`y: 136..230`, height `94px`)**: `GEMINI · WK` (Gemini weekly window).
- **Anatomy per Panel**:
  - **Shadow**: 1-bit halftone (`step = 3`) offset `+6px, +6px`.
  - **Frame**: `PAPER` fill + 2px `INK` stroke, `cut = 10px`.
  - **Header Tag (`x: 216..310, y + 6..y + 22`)**: Chamfered `INK` pill (`cut = 4px`) with `PAPER` text `GEMINI · 5H` / `GEMINI · WK`.
  - **Per-Window Stale Badge (`x: 344..376, y + 6..y + 22`)**: Outlined 1.5px `INK` box reading `旧` (never overlaps header tag).
  - **Readout Row (`y + 56`)**: Left (`x = 218`) bold percentage (`97%`, `91%`, `u8g2_font_logisoso26_tn` / `logisoso30_tn`); Right (`x = 376`) reset countdown (`4h45m`, `6天8h`, or `--`).
  - **Slanted Energy Bar (`x: 218..376, y + 66..y + 82`, height `16px`)**: 10 slanted parallelogram segments.
  - **Single-Window Unavailable**: Keep armored frame + header tag; body centered reads `暂不可用`.
- **Both Windows Unavailable**: Retain both armored outlines or fill right bay (`x: 210..384, y: 32..230`) with 1-bit halftone matrix and centered inset armored box reading `GEMINI 暂不可用` / `等待读取额度`.

### D. Bottom Dual Reset-Radar Dock (`y: 246..292`, height `46px`)
- Two equal cells:
  - **Left (`x: 10..196`, width `186px`)**: `CODEX` (`didcodexreset.com`).
  - **Right (`x: 204..390`, width `186px`)**: `GEMINI` (`agyreset.com`, ignoring third-party-only `CLAUDE/GPT` announcements).
- **Active Alert (`Scheduled` or within-24h `Confirmed`)**:
  - Full-cell 45° diagonal `INK` hazard stripes (`step = 6px`) on `PAPER` ground, with a solid `INK` center plate (`x + 12..x + 174, y: 250..288`) and `PAPER` text:
    - Line 1: `<PROVIDER> // <状态标签>` (append `(旧)` if stale cache).
    - Line 2: concise window/scope summary (e.g., `Banked · 5h前`, `Gemini 今日22:00`).
- **Quiet (`NoSchedule`) / Unavailable (`信号丢失`)**:
  - Chamfered 1.5px `INK` outline (`cut = 6px`) on `PAPER` ground:
    - Line 1: `<PROVIDER> RADAR`.
    - Line 2: `暂无排期` or `信号丢失`.

## 4. Typography & Anti-Churn Cadence

- **Bitmap Fonts (`u8g2-fonts`)**:
  - Codex hero numeral: `u8g2_font_logisoso50_tn` / `u8g2_font_logisoso42_tn`
  - Gemini percentage numerals: `u8g2_font_logisoso26_tn` / `u8g2_font_logisoso30_tn`
  - Labels & CJK copy: `u8g2_font_wqy13_t_gb2312` & `u8g2_font_wqy14_t_gb2312`
- **Coarse Countdown Cadence**: `>= 24h` hourly steps, `1h..24h` 15-minute steps, `15m..1h` 5-minute steps, `< 15m` 1-minute steps; confirmed reset elapsed labels step hourly once `>= 1h` old.
