use ai_quota_zectrix_dashboard::{
    Frame,
    model::{Dashboard, Source},
    render,
};

fn fixture(name: &str) -> Dashboard {
    serde_json::from_slice(&std::fs::read(format!("fixtures/{name}.json")).unwrap()).unwrap()
}

fn pixels(frame: &Frame) -> image::GrayImage {
    image::load_from_memory(&frame.png_bytes().unwrap())
        .unwrap()
        .to_luma8()
}

#[test]
fn weekly_dial_and_scoped_telemetry_share_the_locked_grid() {
    let frame = render(&fixture("normal")).unwrap();
    for label in ["AI-QUOTA // 01-CODEX 02-GEMINI", "11/26 周四 20:42"] {
        assert!(
            frame.visible_text.iter().any(|s| s == label),
            "missing {label}"
        );
    }
    let pixels = pixels(&frame);
    assert_eq!(pixels.get_pixel(399, 23)[0], 0);
    assert_eq!(pixels.get_pixel(399, 24)[0], 255);
}

#[test]
fn codex_orbit_and_energy_cells_show_weekly_capacity() {
    let frame = render(&fixture("normal")).unwrap();
    for label in ["CODEX // WK", "CR×1", "66", "%", "RST // 4天6h"] {
        assert!(
            frame.visible_text.iter().any(|s| s == label),
            "missing {label}"
        );
    }
    let image = pixels(&frame);
    for (x, y, value) in [
        (10, 32, 255),
        (25, 32, 0),
        (11, 90, 0),
        (30, 70, 255),
        (104, 74, 0),
        (104, 81, 0),
        (104, 187, 255),
        (28, 196, 0),
        (177, 196, 255),
    ] {
        assert_eq!(image.get_pixel(x, y)[0], value, "at ({x},{y})");
    }
}

#[test]
fn low_and_cached_badges_fit_alongside_full_percentages_and_reset_labels() {
    let mut d = fixture("stale");
    d.antigravity_radar = d.codex_radar.clone();
    let Source::Stale(q) = &mut d.codex else {
        unreachable!()
    };
    q.weekly.remaining_percent = 25;
    let Source::Stale(gemini) = &mut d.antigravity else {
        unreachable!()
    };
    gemini.five_hour.as_mut().unwrap().window.remaining_percent = 100;
    let frame = render(&d).unwrap();
    for label in ["[!LOW]", "旧", "100%", "GEMINI // 重置额度已发(旧)"] {
        assert!(
            frame.visible_text.contains(&label.to_owned()),
            "missing {label}: {:?}",
            frame.visible_text
        );
    }
    let Source::Stale(q) = &mut d.codex else {
        unreachable!()
    };
    q.weekly.remaining_percent = 100;
    q.reset_credits = None;
    let empty = pixels(&render(&d).unwrap());
    for n in [10, 12345, 1234567, u32::MAX] {
        let Source::Stale(q) = &mut d.codex else {
            unreachable!()
        };
        q.reset_credits = Some(n);
        let frame = render(&d).unwrap();
        for label in ["CODEX // WK".into(), format!("CR×{n}"), "旧".into()] {
            assert!(frame.visible_text.contains(&label), "{label}");
        }
        let image = pixels(&frame);
        assert_eq!(
            image.get_pixel(187, 39)[0],
            0,
            "credit badge at header right"
        );
        let mut gaps = 0;
        let mut in_gap = false;
        for x in 23..187 {
            let paper = (39..56).all(|y| image.get_pixel(x, y)[0] == 255);
            gaps += usize::from(paper && !in_gap);
            in_gap = paper;
        }
        assert_eq!(
            gaps, 2,
            "title, stale badge and credit count need separate boxes"
        );
        for (x, y, p) in image.enumerate_pixels() {
            if !(18..190).contains(&x) || !(39..56).contains(&y) {
                assert_eq!(p, empty.get_pixel(x, y), "credit outside header at {x},{y}");
            }
        }
    }
}

#[test]
fn stale_and_unavailable_sources_change_only_their_own_screen_region() {
    let normal = fixture("normal");
    let healthy = render(&normal).unwrap();
    let reference = pixels(&healthy);
    for degraded in ["stale", "unavailable"] {
        let bad = fixture(degraded);
        for index in 0..4 {
            let mut d = normal.clone();
            let (left, top, right, bottom, marker) = match index {
                0 => {
                    d.codex = bad.codex.clone();
                    (10, 32, 198, 230, "暂不可用")
                }
                1 => {
                    d.antigravity = bad.antigravity.clone();
                    (210, 32, 384, 230, "暂不可用")
                }
                2 => {
                    d.codex_radar = bad.codex_radar.clone();
                    (10, 246, 196, 292, "信号丢失")
                }
                _ => {
                    d.antigravity_radar = bad.antigravity_radar.clone();
                    (204, 246, 390, 292, "信号丢失")
                }
            };
            let frame = render(&d).unwrap();
            assert_ne!(frame.sha256(), healthy.sha256(), "{degraded} {index}");
            let marker = if degraded == "stale" { "旧" } else { marker };
            assert!(
                frame.visible_text.iter().any(|s| s.contains(marker)),
                "{degraded} {index} {marker} {:?}",
                frame.visible_text
            );
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
fn active_reset_signals_show_striped_docks_and_cached_status_in_each_plate() {
    for name in ["normal", "stale"] {
        let frame = render(&fixture(name)).unwrap();
        let suffix = if name == "stale" { "(旧)" } else { "" };
        for title in ["CODEX // 重置额度已发", "GEMINI // 重置排期"] {
            let label = format!("{title}{suffix}");
            assert!(frame.visible_text.contains(&label), "missing {label}");
        }
        let pixels = pixels(&frame);
        for x in [10, 204] {
            let stripe: Vec<_> = (x..x + 6)
                .map(|col| pixels.get_pixel(col, 260)[0])
                .collect();
            assert_eq!(stripe.iter().filter(|p| **p == 0).count(), 2);
            assert_eq!(pixels.get_pixel(x + 13, 251)[0], 0);
        }
    }
}

#[test]
fn each_gemini_window_renders_or_degrades_inside_its_own_card() {
    let normal = fixture("normal");
    let frame = render(&normal).unwrap();
    for label in ["GEMINI · 5H", "97%", "GEMINI · WK", "91%"] {
        assert!(
            frame.visible_text.contains(&label.to_owned()),
            "missing {label}"
        );
    }
    let reference = pixels(&frame);
    for (five_hour, top) in [(true, 32), (false, 136)] {
        for missing in [false, true] {
            let mut d = normal.clone();
            let Source::Current(q) = &mut d.antigravity else {
                unreachable!()
            };
            let slot = if five_hour {
                &mut q.five_hour
            } else {
                &mut q.weekly
            };
            if missing {
                *slot = None;
            } else {
                slot.as_mut().unwrap().stale = true;
            }
            let frame = render(&d).unwrap();
            let marker = if missing { "暂不可用" } else { "旧" };
            assert_eq!(
                frame
                    .visible_text
                    .iter()
                    .filter(|s| s.as_str() == marker)
                    .count(),
                1
            );
            assert!(
                frame
                    .visible_text
                    .contains(&if five_hour { "91%" } else { "97%" }.to_owned())
            );
            for (x, y, p) in pixels(&frame).enumerate_pixels() {
                if !(210..384).contains(&x) || !(top..top + 94).contains(&y) {
                    assert_eq!(p, reference.get_pixel(x, y));
                }
            }
        }
    }
    let frame = render(&fixture("single-window")).unwrap();
    for label in ["100", "0%", "暂不可用", "暂无排期"] {
        assert!(
            frame.visible_text.iter().any(|s| s == label),
            "missing {label}"
        );
    }
}

#[test]
fn fixture_frames_match_the_reviewed_one_bit_layouts() {
    for (name, hash) in [
        (
            "normal",
            "a6aabd358a0c79b0858e3a686ed18b7505e3ac79dab91088ca6828680e94daee",
        ),
        (
            "single-window",
            "adbf48e2ccfba5901e726a2f4ff60e39b9a8f72d321a27a273f2e8eed073d1de",
        ),
        (
            "stale",
            "c73785390ed7198b91871e69a59f5232e6a217d6587d0ab3f28f810716938a9a",
        ),
        (
            "unavailable",
            "0829f6701e7c20cf58d6554d7bfee9edfeab327fa43f22e8397ae28abd2bf10c",
        ),
    ] {
        assert_eq!(render(&fixture(name)).unwrap().sha256(), hash, "{name}");
    }
}
