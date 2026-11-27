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
- Information hierarchy: the left panel features Codex Weekly, the right panel shows the two Gemini quota windows, and the bottom dock holds the reset radars.
- Surface mode: **Operate** — scanability, tabular precision, and instant anomaly recognition outrank decorative expression.
- Data cadence: Quotas polled every 10 minutes; public reset signals polled at startup and hourly; cloud uploads throttled to at most once every 60 seconds and suppressed when visible content is unchanged.

## Capabilities and Constraints

- Four independent sources: Codex quota (`codex app-server --stdio`, displaying the **Weekly** quota window only—no 5-hour window), Codex reset signal (`didcodexreset.com/openapi/v1/status`), Antigravity quota (official CLI `/usage` report or a configured `statusLine` JSON capture), and Antigravity reset signal (`agyreset.com/openapi/v1/status`).
- Any single source may be stale or unavailable independently; missing data is never replaced with `0%`.
- Antigravity displays Gemini's two native quota windows (`GEMINI · 5H` on top and `GEMINI · WK` on bottom), each observed, cached, and degraded independently; third-party (`CLAUDE/GPT`) quotas are omitted and third-party-only `agyreset.com` events are ignored.
- Reset signals follow one unified 3-tier priority: pending schedule > 24h confirmed reset (with affected scope) > fresh no-schedule > signal unavailable.
- No grayscale, no animation, no interactive controls on the e-paper surface; all text must fit within 400×300 using embedded bitmap CJK and numeric fonts without truncation of critical percentages or status labels.

## Product Principles

1. **Glanceable at 60cm**: Remaining percentages and active reset alerts must read instantaneously without squinting.
2. **Honest 1-bit polarity**: Use black-vs-white inversion solely to establish primary/secondary hierarchy and to flag active reset schedules/confirmations—never for ornament.
3. **Truthful degradation**: Stale caches and unavailable sources must remain visually unambiguous without disturbing healthy sections.
4. **Zero noise on quiet days**: When no reset schedule or recent confirmation exists, the reset row recedes so quota numbers own the screen.

## Companion commands

`live-preview --output preview.png` gathers all four observations and writes a local image. The companion reads Antigravity quota from the signed-in official CLI with `agy -p /usage --output-format json` (verified on 1.5.18). Installation resolves `agy` from `AI_QUOTA_AGY_BIN`, then PATH, with `~/.local/bin/agy` as the fallback. A configured `AI_QUOTA_STATUSLINE_PATH` supplies Gemini windows from captures up to 20 minutes old. The CLI fills missing or expired windows. Each window retains its last successful observation independently. Temporary network failures, timeouts, and partial reports retry after 15, 60, then 300 seconds. Successful reads and permanent failures use the normal ten-minute interval. The `diagnostics` command reports `gemini_five_hour` and `gemini_weekly`, each with a status, last successful observation (Unix seconds), and sanitized error category. The first successful poll replaces an old two-group cache. For a CLI-specific proxy, set `AI_QUOTA_AGY_PROXY` to a credential-free proxy address before installing the LaunchAgent. The CLI can report different quota buckets from the desktop app.

`setup` previews the live frame and asks before uploading. `run` starts the companion in the foreground. `diagnostics` prints per-source and publisher health. On macOS, `launch-agent plist` prints the login agent definition, `launch-agent install` copies the executable into Application Support and starts it, and `launch-agent remove` stops and removes it. Installation captures the configured statusLine path and Codex executable path. Pass `--codex-program /absolute/path/to/codex` to select a specific executable.
