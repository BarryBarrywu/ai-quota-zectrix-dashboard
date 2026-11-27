# Domain glossary

| Term | Meaning |
| --- | --- |
| Quota window | A provider limit with a remaining percentage and a reset timestamp. Zero means an observed exhausted allowance. |
| Codex weekly panel | Primary Codex readout showing only the weekly quota window. A positive reset-credit count appears separately. |
| Gemini windows | The two independently observed Antigravity Gemini pools: 5-hour sprint (`5H`) and weekly baseline (`WK`). |
| Reset credit | A banked Codex discretionary reset entitlement. It is separate from the remaining percentage. |
| Reset radar | One provider's public schedule or most recent qualifying confirmation. Antigravity skips explicit third-party announcements and named model lists containing only recognized third-party IDs (Claude, GPT, or third-party pool IDs). Mixed lists, unknown IDs, and unspecified scopes remain eligible. |
| Observation | One independently validated quota or radar result. Current means the latest check succeeded, stale means the displayed evidence was cached, unavailable means no usable evidence. |
| Screen update time | Local time of the most recent successful screen update. A missing first update displays as `--:--`. |
| Frame hash | SHA-256 over row-major 400×300 luminance samples (one byte per pixel, black 0 and white 255), independent of PNG compression. |
