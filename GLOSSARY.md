# Domain glossary

| Term | Meaning |
| --- | --- |
| Quota window | A provider limit with a remaining percentage and a reset timestamp. Zero means an observed exhausted allowance. |
| Weekly dial | Primary Codex readout. Only the weekly window contributes to the dial. A positive reset-credit count appears separately. |
| Constrained window | The lowest remaining percentage in an Antigravity model group. Equal percentages are resolved by earliest reset. |
| Model group | GEMINI or CLAUDE/GPT. The latter represents the upstream third-party pool. |
| Reset credit | A banked discretionary reset. It is separate from the remaining percentage. |
| Reset radar | One provider’s public schedule or most recent qualifying confirmation. Its scope identifies the affected model group. |
| Observation | One independently validated quota or radar result. Current means the latest check succeeded, stale means the displayed evidence was cached, unavailable means no usable evidence. |
| Screen update time | Local time of the most recent successful screen update. A missing first update displays as two dashes on either side of a colon. |
| Frame hash | SHA-256 over row-major 400×300 luminance samples (one byte per pixel, black 0 and white 255), independent of PNG compression. |
