# Product

<!-- impeccable:product-schema 1 -->

## Platform

native-epaper

## Stack

Rust (`embedded-graphics` + `u8g2-fonts` + `image`) rendering 400×300 1-bit monochrome PNG frames uploaded to ZECTRIX Cloud from a standalone macOS companion daemon managed by LaunchAgent.

## Users

macOS developers who use both Codex and Google Antigravity daily and keep a ZECTRIX NOTE4 4.2-inch e-paper display on their desk (~60cm viewing distance) to check remaining quota allowances and public reset signals at a glance without switching windows.

## Product Purpose

Provide a calm, glanceable, always-on desk instrument showing remaining Codex and Antigravity quotas alongside public reset schedules and recent confirmed resets (`didcodexreset.com` and `agyreset.com`), updating the e-paper screen only when visible information changes.

## Positioning

A dedicated dual-AI quota and reset-signal desk instrument with zero task-title privacy leakage and four independently degrading data sources—unlike `codex-zectrix-dashboard` (a Codex-only plugin with a task activity feed) or desktop menu-bar widgets that compete for screen focus.

## Operating Context

- Physical display: ZECTRIX NOTE4, fixed 400×300 resolution, pure 1-bit monochrome (`dither=false`), viewed under ambient desk lighting at ~50–70cm distance.
- Information hierarchy: Codex is primary (upper half); Antigravity is secondary (lower half).
- Surface mode: **Operate** — scanability, tabular precision, and instant anomaly recognition outrank decorative expression.
- Data cadence: Quotas polled every 10 minutes; public reset signals polled at startup and hourly; cloud uploads throttled to at most once every 60 seconds and suppressed when visible content is unchanged.

## Capabilities and Constraints

- Four independent sources: Codex quota (`codex app-server --stdio`, displaying the **Weekly** quota window only—no 5-hour window), Codex reset signal (`didcodexreset.com/openapi/v1/status`), Antigravity quota (local `language_server` Connect-RPC with `statusLine` JSON fallback), and Antigravity reset signal (`agyreset.com/openapi/v1/status`).
- Any single source may be stale or unavailable independently; missing data is never replaced with `0%`.
- Antigravity displays two canonical model groups (`GEMINI` and `CLAUDE/GPT`, mapped from upstream `gemini` and `third_party`/`3p` scopes), each showing its most constrained window (lowest remaining %, tie-breaking by sooner reset time).
- Reset signals follow one unified 3-tier priority: pending schedule > 24h confirmed reset (with affected scope) > fresh no-schedule > signal unavailable.
- No grayscale, no animation, no interactive controls on the e-paper surface; all text must fit within 400×300 using embedded bitmap CJK and numeric fonts without truncation of critical percentages or status labels.

## Product Principles

1. **Glanceable at 60cm**: Remaining percentages and active reset alerts must read instantaneously without squinting.
2. **Honest 1-bit polarity**: Use black-vs-white inversion solely to establish primary/secondary hierarchy and to flag active reset schedules/confirmations—never for ornament.
3. **Truthful degradation**: Stale caches and unavailable sources must remain visually unambiguous without disturbing healthy sections.
4. **Zero noise on quiet days**: When no reset schedule or recent confirmation exists, the reset row recedes so quota numbers own the screen.

## Companion commands

`live-preview --output preview.png` gathers all four observations and writes a local image. The implemented Antigravity quota source reads the official CLI statusLine JSON file named by `AI_QUOTA_STATUSLINE_PATH`. A missing or invalid capture retains the last validated quota as stale when available. Automatic desktop RPC collection remains pending.

`setup` previews the live frame and asks before uploading. `run` starts the companion in the foreground. `diagnostics` prints per-source and publisher health. On macOS, `launch-agent plist` prints the login agent definition, `launch-agent install` copies the executable into Application Support and starts it, and `launch-agent remove` stops and removes it. Installation captures the configured statusLine path and Codex executable path. Pass `--codex-program /absolute/path/to/codex` to select a specific executable.
