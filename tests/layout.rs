use ai_quota_zectrix_dashboard::{model::Dashboard, render};

fn fixture(name: &str) -> Dashboard {
    serde_json::from_slice(&std::fs::read(format!("fixtures/{name}.json")).unwrap()).unwrap()
}

#[test]
fn weekly_dial_and_scoped_telemetry_share_the_locked_grid() {
    let frame = render(&fixture("normal")).unwrap();
    for label in [
        "CODEX",
        "WEEKLY",
        "66",
        "% 剩余",
        "重置额度 ×1",
        "11/26 周四",
        "20:42",
        "ANTIGRAVITY",
        "GEMINI",
        "周窗口",
        "88%",
        "CLAUDE/GPT",
        "5h窗口",
        "42%",
        "CODEX · 重置额度已发",
        "AGY · 重置排期",
    ] {
        assert!(
            frame.visible_text.iter().any(|s| s == label),
            "missing {label}"
        );
    }
    let pixels = image::load_from_memory(&frame.png_bytes().unwrap())
        .unwrap()
        .to_luma8();
    for (x, y, value) in [
        (0, 0, 0),
        (213, 245, 0),
        (214, 245, 255),
        (107, 60, 255),
        (226, 58, 0),
        (226, 150, 0),
        (399, 246, 0),
        (12, 253, 0),
        (207, 253, 0),
    ] {
        assert_eq!(pixels.get_pixel(x, y)[0], value, "at ({x}, {y})");
    }
}

#[test]
fn stale_and_unavailable_sources_change_only_their_own_screen_region() {
    let normal = fixture("normal");
    let healthy = render(&normal).unwrap();
    let pixels = |frame: &ai_quota_zectrix_dashboard::Frame| {
        image::load_from_memory(&frame.png_bytes().unwrap())
            .unwrap()
            .to_luma8()
    };
    let reference = pixels(&healthy);
    for degraded in ["stale", "unavailable"] {
        let bad = fixture(degraded);
        for index in 0..4 {
            let mut d = normal.clone();
            let (left, top, right, bottom, marker) = match index {
                0 => {
                    d.codex = bad.codex.clone();
                    (0, 0, 214, 246, "暂不可用")
                }
                1 => {
                    d.antigravity = bad.antigravity.clone();
                    (214, 29, 400, 246, "额度不可用")
                }
                2 => {
                    d.codex_radar = bad.codex_radar.clone();
                    (8, 252, 196, 294, "信号丢失")
                }
                _ => {
                    d.antigravity_radar = bad.antigravity_radar.clone();
                    (204, 252, 392, 294, "信号丢失")
                }
            };
            let frame = render(&d).unwrap();
            assert_ne!(frame.sha256(), healthy.sha256(), "{degraded} {index}");
            let marker = if degraded == "stale" { "旧" } else { marker };
            assert!(frame.visible_text.iter().any(|s| s.contains(marker)));
            let actual = pixels(&frame);
            for (x, y, p) in actual.enumerate_pixels() {
                if !(left..right).contains(&x) || !(top..bottom).contains(&y) {
                    assert_eq!(p, reference.get_pixel(x, y));
                }
            }
        }
    }
    let frame = render(&fixture("unavailable")).unwrap();
    assert!(!frame.visible_text.iter().any(|s| s.contains('%')));
}

#[test]
fn weekly_only_and_real_zero_are_distinct_from_missing_allowances() {
    let frame = render(&fixture("single-window")).unwrap();
    for label in ["100", "100%", "0%", "暂无排期"] {
        assert!(
            frame.visible_text.iter().any(|s| s == label),
            "missing {label}"
        );
    }
    assert!(
        !frame
            .visible_text
            .iter()
            .any(|s| s.contains("额度 ×") || s.contains("5h窗口") && s.starts_with("CODEX"))
    );
}

#[test]
fn fixture_frames_match_the_reviewed_one_bit_layouts() {
    for (name, hash) in [
        (
            "normal",
            "92bdd01d81564509797a2e37f49731cbdaa7618bbb9a4ecba5035c57bb259bb8",
        ),
        (
            "single-window",
            "4e5741463f93e7d1dac529d240cbff7e3c8e4ed7e993ad7dd04da72f001f87ae",
        ),
        (
            "stale",
            "34cc067b5c556d5cea8ee64c45474d5a9fb9bad5e1ed5015745d9aedd53be056",
        ),
        (
            "unavailable",
            "4f775419e15ec8b72ea39007e1322abba7cf56e1f1beafd453f0335d30d13cc6",
        ),
    ] {
        assert_eq!(render(&fixture(name)).unwrap().sha256(), hash, "{name}");
    }
}
