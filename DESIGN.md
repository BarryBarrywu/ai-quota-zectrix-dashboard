# Design System — Braun Orbital Dial (400×300 1-bit E-Paper)

> Locked visual specification for `ai-quota-zectrix-dashboard` on ZECTRIX NOTE4 (400×300, pure 1-bit monochrome, `dither=false`).

## 1. Visual World & Polarity Principles

- **Direction**: **Braun Orbital Dial（博朗仪表圆盘）** — a precision physical desk instrument combining a high-contrast dark orbital gauge for Codex (primary) with a crisp paper-ground dual telemetry bay for Antigravity (secondary) and a unified dual reset-radar dock along the bottom.
- **Color Tokens (1-bit Binary Only)**:
  - `INK` (`BinaryColor::On` / black pixel): primary dial face background, Antigravity bay labels/borders/bars, active reset alert fill on paper ground.
  - `PAPER` (`BinaryColor::Off` / white pixel): primary dial arc/numerals/ticks on black face, Antigravity bay background, active reset alert text on black dock.
  - **Zero grayscale, zero anti-aliasing, zero dithering** (`dither=false`).
- **Polarity Grammar**:
  - **Dark face (`0..214 × 0..246`)** anchors the primary provider (**Codex Weekly**).
  - **Light face (`214..400 × 0..246`)** anchors the secondary provider (**Antigravity: GEMINI + CLAUDE/GPT**).
  - **Bottom radar dock (`0..400 × 246..300`)** stays quiet (outlined white boxes with black text) on normal days and **inverts to solid black fill with white text** the instant a `pending` schedule or 24h confirmed reset is active.

## 2. Spatial Grid (400 × 300 px)

```text
+-------------------------------+---------------------------+  y = 0
| CODEX                  WEEKLY | 11/26 周四          20:45 |  y = 28
|                               | ------------------------- |
|         . - ~ ~ ~ - .         | ANTIGRAVITY               |  y = 46
|      , '  270° ARC   ' ,      | +-----------------------+ |  y = 58
|     ,       66          ,     | | GEMINI          周窗口 | |
|     ,     % 剩余        ,     | | 88%          重置4天6h | |
|      ' .             . '      | | [===============    ] | |  y = 140
|         [重置额度 ×1]          | +-----------------------+ |
|         重置 4天6小时          | +-----------------------+ |  y = 150
|                               | | CLAUDE/GPT     5h窗口 | |
|                               | | 42%         重置2h40m | |
|                               | | [=======            ] | |  y = 232
+-------------------------------+---------------------------+  y = 246 (2px solid INK rule)
| [CODEX · 重置额度已发         ] | [AGY · 排期              ] |  y = 252..294
| [Banked · 5h前               ] | [Gemini 今日22点        ] |
+-------------------------------+---------------------------+  y = 300
x = 0                        x = 214                     x = 400
```

## 3. Component & Geometry Specifications

### A. Left Primary Instrument: Codex Weekly Dial (`x: 0..214, y: 0..246`)
- **Background**: Solid `INK` fill.
- **Header (`y = 20`)**: `CODEX` (left, `x = 14`, bold mono/label) and `WEEKLY` (right, `x = 200`). When Codex quota is stale, append `[旧]` next to `CODEX`.
- **Orbital Arc Gauge**:
  - Center: `(107, 128)`, outer track radius `r = 68`, angular span `270°` (`135°` to `405°`, i.e., bottom-left 7:30 clockwise to bottom-right 4:30).
  - **20 radial scale ticks** around `r + 7` to `r + 11` (major ticks every 5th step—0%, 25%, 50%, 75%, 100%—extend to `r + 15`, 2px stroke).
  - **Base track**: 2px `PAPER` arc across full `270°`.
  - **Remaining fill arc**: 14px thick `PAPER` arc centered at `r - 6`, sweeping `270° × (remaining_percent / 100)`.
- **Center Readout**:
  - Giant tabular numeral (`remaining_percent`, e.g., `66` or `100`) centered at `(107, 140)` using largest crisp bitmap numeral font (`u8g2_font_logisoso42_tn` or `logisoso46_tn`).
  - Subtitle `% 剩余` centered at `(107, 160)`.
- **Footer Readout inside Dark Face**:
  - If `reset_credits > 0`: inverted `PAPER` pill (`x: 62..152, y: 200..214`) with `INK` text `重置额度 ×N`.
  - Reset countdown (`重置 4天6小时`) centered at `(107, 226)`.
- **Unavailable State (no Codex cache)**:
  - Keep empty 270° track + ticks (no filled arc, never `0%`); center reads `暂不可用` / `等待 Codex`.

### B. Right Secondary Bay: Antigravity Telemetry (`x: 214..400, y: 0..246`)
- **Background**: Solid `PAPER` fill.
- **Top Meta Bar (`y = 20`, rule at `y = 28`)**:
  - Left (`x = 226`): Current date (`11/26 周四`).
  - Right (`x = 388`): Last successful screen update time (`HH:MM`).
- **Section Label (`y = 46`)**: `ANTIGRAVITY` (left `x = 226`); if stale, small outlined badge `旧` right beside it.
- **Two Model-Group Cards** (`x: 226..388`, width `162px`, height `82px`, 1.5–2px `INK` border):
  - **Card 1 (`y: 58..140`)**: Header bar (`height: 18px`, solid `INK` fill, `PAPER` text) showing `GEMINI` (left) and constrained window tag `周窗口` / `5h窗口` (right).
  - **Card 2 (`y: 150..232`)**: Same anatomy with header `CLAUDE/GPT` (left) and constrained window tag (right).
  - **Card Body**:
    - Left: bold `30–32px` tabular percentage (`88%`, `42%`).
    - Right: two-line stacked reset countdown (`重置` above `4天6h` / `2h40m`).
    - Bottom (`height: 8px`, `146px` wide): 1px `INK` bordered bar with solid `INK` fill proportional to `remaining_percent`.
- **Unavailable / Offline State (Antigravity closed & no cache)**:
  - Fill card bay (`x: 226..388, y: 56..234`) with 1-bit 5px dot-matrix pattern, centered inset box (`PAPER` fill, 2px `INK` border) reading `额度不可用` / `等待读取额度`.

### C. Bottom Dual Reset-Radar Dock (`x: 0..400, y: 246..300`)
- Separated by a full-width 2px `INK` horizontal rule at `y = 246`.
- Two equal-width dock cells (`y: 252..294`, height `42px`):
  - **Left (`x: 8..196`, width `188px`)**: Codex reset signal (`didcodexreset.com`).
  - **Right (`x: 204..392`, width `188px`)**: Antigravity reset signal (`agyreset.com`).
- **Visual States per Dock Cell**:
  - **Active Alert (`Scheduled` or within-24h `Confirmed`)**: Solid `INK` fill, `PAPER` text. Line 1: `<PROVIDER> · <状态标签>` (e.g., `CODEX · 重置额度已发`, `AGY · 重置排期`). Line 2: concise window/scope summary (e.g., `Banked · 5h前`, `Gemini 今日22点`). If stale cache during network error, append `(旧)`.
  - **Quiet (`NoSchedule`)**: `PAPER` fill with 1px `INK` border. Line 1: `<PROVIDER> RADAR`. Line 2: `暂无排期`.
  - **Unavailable (`Signal unavailable`)**: `PAPER` fill with 1px `INK` border. Line 1: `<PROVIDER> RADAR`. Line 2: `信号丢失`.

## 4. Typography & Bitmap Font Mapping (Rust `u8g2-fonts`)

- **Dial Hero Numeral**: `u8g2_font_logisoso42_tn` (tabular digits, zero jitter between `0`–`100`).
- **Antigravity Card Numeral**: `u8g2_font_logisoso26_tn` or `u8g2_font_logisoso30_tn`.
- **Chinese & Labels**: `u8g2_font_wqy13_t_gb2312` (metadata, reset countdowns, dock line 2), `u8g2_font_wqy14_t_gb2312` / `u8g2_font_wqy15_t_gb2312` (headers, alert titles, unavailable notices).
- Every dynamic string must be measured and clipped with `fit_text` against its exact pixel box so no text ever bleeds across the `x = 214` or `y = 246` structural rules.
