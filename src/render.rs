use crate::model::Dashboard;
use sha2::{Digest, Sha256};

const WIDTH: usize = 400;
const HEIGHT: usize = 300;
pub struct Frame {
    packed: Vec<u8>,
    pub visible_text: Vec<String>,
}
impl Frame {
    pub fn png_bytes(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, WIDTH as u32, HEIGHT as u32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::One);
        encoder.write_header()?.write_image_data(&self.packed)?;
        Ok(bytes)
    }
    pub fn sha256(&self) -> String {
        let mut hash = Sha256::new();
        for byte in &self.packed {
            hash.update(std::array::from_fn::<_, 8, _>(|bit| {
                if byte & (0x80 >> bit) != 0 { 255u8 } else { 0 }
            }));
        }
        format!("{:x}", hash.finalize())
    }
}

use crate::model::{CodexQuota, Radar, ResetKind, Scope, Source, WindowKind};
use chrono::{Datelike, TimeZone};
use embedded_graphics::{
    draw_target::DrawTargetExt,
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Arc, Line, PrimitiveStyle, Rectangle},
};
use std::{convert::Infallible, ops::Range};
use u8g2_fonts::{
    FontRenderer,
    fonts::*,
    types::{
        FontColor,
        HorizontalAlignment::{self, *},
        VerticalPosition,
    },
};

const INK: BinaryColor = BinaryColor::On;
const PAPER: BinaryColor = BinaryColor::Off;
const BODY: FontRenderer = FontRenderer::new::<u8g2_font_wqy13_t_gb2312>();
const LABEL: FontRenderer = FontRenderer::new::<u8g2_font_wqy14_t_gb2312>();
const COMPACT: FontRenderer = FontRenderer::new::<u8g2_font_wqy12_t_gb2312>();
const THREE_DIGITS: FontRenderer = FontRenderer::new::<u8g2_font_logisoso34_tn>();
const HERO: FontRenderer = FontRenderer::new::<u8g2_font_logisoso42_tn>();
const NUMBER: FontRenderer = FontRenderer::new::<u8g2_font_logisoso26_tf>();

impl OriginDimensions for Frame {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}
impl DrawTarget for Frame {
    type Color = BinaryColor;
    type Error = Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Self::Color>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Self::Error> {
        for Pixel(p, c) in pixels {
            if (0..WIDTH as i32).contains(&p.x) && (0..HEIGHT as i32).contains(&p.y) {
                let i = p.y as usize * WIDTH + p.x as usize;
                let mask = 0x80 >> (i % 8);
                if c == PAPER {
                    self.packed[i / 8] |= mask
                } else {
                    self.packed[i / 8] &= !mask
                }
            }
        }
        Ok(())
    }
}

impl Frame {
    fn rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: BinaryColor) {
        self.fill_solid(&Rectangle::new(Point::new(x, y), Size::new(w, h)), color)
            .unwrap();
    }
    fn label(
        &mut self,
        font: &FontRenderer,
        text: &str,
        region: Range<i32>,
        baseline: i32,
        align: HorizontalAlignment,
        color: BinaryColor,
    ) -> Result<(), String> {
        let width = region.end - region.start;
        // The embedded CJK face lacks a middle dot. Reserve its advance and draw the dot.
        let glyphs = text.replace('·', " ");
        let mut fitted = glyphs.as_str();
        while font
            .get_rendered_dimensions(fitted, Point::zero(), VerticalPosition::Baseline)
            .map_err(|e| format!("font measurement: {e:?}"))?
            .advance
            .x
            > width
        {
            fitted = &fitted[..fitted.char_indices().last().unwrap().0];
        }
        let anchor = match align {
            Left => region.start,
            Center => (region.end + region.start) / 2,
            Right => region.end - 1,
        };
        let clip = Rectangle::new(
            Point::new(region.start, 0),
            Size::new(width as u32, HEIGHT as u32),
        );
        font.render_aligned(
            fitted,
            Point::new(anchor, baseline),
            VerticalPosition::Baseline,
            align,
            FontColor::Transparent(color),
            &mut self.clipped(&clip),
        )
        .map_err(|e| format!("font render: {e:?}"))?;
        let visible: String = text.chars().take(fitted.chars().count()).collect();
        if let Some((before, _)) = visible.split_once('·') {
            let advance = font
                .get_rendered_dimensions(before, Point::zero(), VerticalPosition::Baseline)
                .unwrap()
                .advance
                .x;
            self.rect(anchor + advance + 1, baseline - 6, 2, 2, color);
        }
        self.visible_text.push(visible);
        Ok(())
    }
    fn chamfer(&mut self, x: Range<i32>, y: Range<i32>, cut: i32, color: BinaryColor) {
        for row in y.clone() {
            let left = x.start + (cut - (row - y.start)).max(0);
            let right = x.end - (cut - (y.end - 1 - row)).max(0);
            self.rect(left, row, (right - left) as u32, 1, color);
        }
    }

    fn armor(&mut self, x: Range<i32>, y: Range<i32>, cut: i32, border: i32) {
        for (i, row) in (y.start + 6..y.end + 6).step_by(3).enumerate() {
            let offset = (i % 2) as i32;
            for col in (x.start + 6 + offset..x.end + 6).step_by(3) {
                if col - x.start - 6 + row - y.start - 6 >= cut
                    && x.end + 5 - col + y.end + 5 - row >= cut
                {
                    self.rect(col, row, 1, 1, INK);
                }
            }
        }
        self.chamfer(x.clone(), y.clone(), cut, INK);
        self.chamfer(
            x.start + border..x.end - border,
            y.start + border..y.end - border,
            cut - border / 2,
            PAPER,
        );
    }

    fn cells(&mut self, x: Range<i32>, y: Range<i32>, percent: u8) {
        let span = x.end - x.start - 5;
        let height = y.end - y.start;
        for cell in 0..10 {
            let left = x.start + cell * (span + 3) / 10;
            let right = x.start + (cell + 1) * (span + 3) / 10 - 3;
            for row in 0..height {
                let slant = 5 * (height - 1 - row) / (height - 1);
                if cell < i32::from(percent.div_ceil(10)) || row == 0 || row == height - 1 {
                    self.rect(left + slant, y.start + row, (right - left) as u32, 1, INK);
                } else {
                    self.rect(left + slant, y.start + row, 1, 1, INK);
                    self.rect(right - 1 + slant, y.start + row, 1, 1, INK);
                }
            }
        }
    }

    fn badge(&mut self, x: Range<i32>, y: i32, label: &str) -> Result<(), String> {
        self.chamfer(x.clone(), y..y + 17, 2, INK);
        self.chamfer(x.start + 1..x.end - 1, y + 1..y + 16, 2, PAPER);
        self.label(&BODY, label, x.start + 2..x.end - 2, y + 13, Center, INK)
    }

    fn dial(&mut self, source: &Source<CodexQuota>, now: i64) -> Result<(), String> {
        self.armor(10..198, 32..230, 14, 3);
        self.chamfer(18..122, 39..56, 5, INK);
        self.label(&LABEL, "CODEX // WK", 22..118, 52, Center, PAPER)?;
        let center = Point::new(104, 124);
        Arc::with_center(center, 101, 135.0.deg(), 270.0.deg())
            .into_styled(PrimitiveStyle::with_stroke(INK, 2))
            .draw(self)
            .unwrap();
        for i in 0..20 {
            let angle = (135.0 + 270.0 * i as f64 / 19.0).to_radians();
            let point = |r: f64| {
                center
                    + Point::new(
                        (angle.cos() * r).round() as i32,
                        (angle.sin() * r).round() as i32,
                    )
            };
            let major = i % 5 == 0;
            Line::new(point(54.), point(if major { 61. } else { 57. }))
                .into_styled(PrimitiveStyle::with_stroke(INK, if major { 2 } else { 1 }))
                .draw(self)
                .unwrap();
        }
        if let Some(quota) = source.value() {
            let w = &quota.weekly;
            if w.kind != WindowKind::Weekly || w.remaining_percent > 100 {
                return Err("invalid Codex weekly window".into());
            }
            if source.is_stale() {
                self.badge(125..144, 39, "旧")?;
            }
            if w.remaining_percent <= 25 {
                self.chamfer(145..190, 39..56, 3, INK);
                self.label(
                    &FontRenderer::new::<u8g2_font_6x13B_tf>(),
                    "[!LOW]",
                    147..188,
                    52,
                    Center,
                    PAPER,
                )?;
            } else if let Some(n) = quota.reset_credits.filter(|n| *n > 0) {
                let (region, y) = if n > 9 {
                    (48..160, 171)
                } else {
                    (145..190, 39)
                };
                self.badge(region, y, &format!("CR×{n}"))?;
            }
            if w.remaining_percent > 0 {
                Arc::with_center(
                    center,
                    89,
                    135.0.deg(),
                    (2.7 * w.remaining_percent as f32).deg(),
                )
                .into_styled(PrimitiveStyle::with_stroke(INK, 11))
                .draw(self)
                .unwrap();
            }
            self.label(
                if w.remaining_percent == 100 {
                    &THREE_DIGITS
                } else {
                    &HERO
                },
                &w.remaining_percent.to_string(),
                64..145,
                136,
                Center,
                INK,
            )?;
            self.label(&BODY, "%", 87..121, 152, Center, INK)?;
            self.cells(20..188, 190..203, w.remaining_percent);
            self.label(
                &LABEL,
                &format!(
                    "RST // {}",
                    countdown(now, w.resets_at.ok_or("missing Codex reset")?)
                ),
                18..190,
                220,
                Center,
                INK,
            )?;
        } else {
            self.label(&LABEL, "暂不可用", 67..141, 124, Center, INK)?;
            self.label(&BODY, "等待 Codex", 64..144, 145, Center, INK)?;
        }
        Ok(())
    }
    fn card(
        &mut self,
        y: i32,
        name: &str,
        reading: Option<&crate::model::WindowReading>,
        now: i64,
    ) -> Result<(), String> {
        self.armor(210..384, y..y + 94, 10, 2);
        self.chamfer(216..310, y + 6..y + 22, 4, INK);
        self.label(&BODY, name, 222..306, y + 19, Left, PAPER)?;
        let Some(reading) = reading else {
            return self.label(&LABEL, "暂不可用", 218..376, y + 57, Center, INK);
        };
        if reading.stale {
            self.badge(344..376, y + 6, "旧")?;
        }
        let w = &reading.window;
        if w.remaining_percent > 100 {
            return Err("invalid Antigravity percentage".into());
        }
        self.label(
            &NUMBER,
            &format!("{}%", w.remaining_percent),
            218..301,
            y + 56,
            Left,
            INK,
        )?;
        self.label(
            &BODY,
            &w.resets_at
                .map(|t| countdown(now, t))
                .unwrap_or_else(|| "--".into()),
            302..377,
            y + 56,
            Right,
            INK,
        )?;
        self.cells(218..376, y + 66..y + 82, w.remaining_percent);
        Ok(())
    }
    fn telemetry(&mut self, dashboard: &Dashboard) -> Result<(), String> {
        let now = dashboard.now;
        let weekday = ["一", "二", "三", "四", "五", "六", "日"]
            [now.weekday().num_days_from_monday() as usize];
        let clock = dashboard
            .screen_updated_at
            .map(|t| t.with_timezone(now.offset()).format("%H:%M").to_string())
            .unwrap_or_else(|| "--:--".into());
        self.rect(0, 0, 400, 24, INK);
        self.label(
            &BODY,
            "AI-QUOTA // 01-CODEX 02-GEMINI",
            10..241,
            16,
            Left,
            PAPER,
        )?;
        self.label(
            &BODY,
            &format!("{} 周{weekday} {clock}", now.format("%m/%d")),
            241..391,
            16,
            Right,
            PAPER,
        )?;
        let quota = dashboard.antigravity.value();
        self.card(
            32,
            "GEMINI · 5H",
            quota.and_then(|q| q.five_hour.as_ref()),
            now.timestamp(),
        )?;
        self.card(
            136,
            "GEMINI · WK",
            quota.and_then(|q| q.weekly.as_ref()),
            now.timestamp(),
        )?;
        Ok(())
    }
    fn radar(
        &mut self,
        x: i32,
        provider: &str,
        source: &Source<Radar>,
        d: &Dashboard,
    ) -> Result<(), String> {
        let state = source.value();
        let active = matches!(
            state,
            Some(Radar::Scheduled { .. } | Radar::Confirmed { .. })
        );
        let (title, detail) = match state {
            Some(Radar::Scheduled {
                starts_at,
                ends_at,
                scope,
            }) => {
                let pending = *starts_at > d.now.timestamp();
                let time = d
                    .now
                    .offset()
                    .timestamp_opt(if pending { *starts_at } else { *ends_at }, 0)
                    .single()
                    .ok_or("invalid schedule time")?;
                let date = if time.date_naive() == d.now.date_naive() {
                    "今日".into()
                } else {
                    time.format("%m/%d ").to_string()
                };
                (
                    format!("{provider} // 重置排期"),
                    format!(
                        "{}{date}{}{}",
                        if provider == "GEMINI" {
                            format!("{} ", scope.label())
                        } else {
                            String::new()
                        },
                        time.format("%H:%M"),
                        if pending { "" } else { "截止" }
                    ),
                )
            }
            Some(Radar::Confirmed {
                kind,
                announced_at,
                scope,
            }) => {
                let status = match kind {
                    ResetKind::Global => "全局已重置",
                    ResetKind::Banked => "重置额度已发",
                };
                (
                    format!("{provider} // {status}"),
                    format!(
                        "{} · {}",
                        if provider == "GEMINI" {
                            scope.label()
                        } else {
                            match kind {
                                ResetKind::Banked => "Banked",
                                ResetKind::Global => "Global",
                            }
                        },
                        elapsed_label(*announced_at, d.now.timestamp())
                    ),
                )
            }
            Some(Radar::NoSchedule) => (format!("{provider} RADAR"), "暂无排期".into()),
            None => (format!("{provider} RADAR"), "信号丢失".into()),
        };
        let title = if source.is_stale() {
            format!("{title}(旧)")
        } else {
            title
        };
        let color = if active {
            for row in 0..46 {
                for col in 0..186 {
                    if (col + row) % 6 < 2 {
                        self.rect(x + col, 246 + row, 1, 1, INK);
                    }
                }
            }
            self.rect(x + 12, 250, 162, 38, INK);
            PAPER
        } else {
            self.chamfer(x..x + 186, 246..292, 6, INK);
            self.chamfer(x + 2..x + 184, 248..290, 5, PAPER);
            INK
        };
        self.label(
            if source.is_stale() { &COMPACT } else { &BODY },
            &title,
            x + 13..x + 173,
            265,
            Left,
            color,
        )?;
        self.label(&BODY, &detail, x + 16..x + 171, 282, Left, color)
    }
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Self::All => "全模型",
            Self::Gemini => "Gemini",
            Self::ClaudeGpt => "CLAUDE/GPT",
            Self::Named => "指定模型",
            Self::Unknown => "范围未知",
        }
    }
}
fn elapsed_label(from: i64, to: i64) -> String {
    let minutes = to.saturating_sub(from).max(0) / 60;
    if minutes >= 60 {
        format!("{}h前", minutes / 60)
    } else if minutes >= 10 {
        format!("{}m前", (minutes / 10) * 10)
    } else {
        "刚刚".into()
    }
}
fn countdown(from: i64, to: i64) -> String {
    let minutes = to.saturating_sub(from).max(0) / 60;
    let days = minutes / 1440;
    let hours = (minutes / 60) % 24;
    if days > 0 {
        format!("{days}天{hours}h")
    } else if hours > 0 {
        let mins = ((minutes % 60) / 15) * 15;
        if mins == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h{mins}m")
        }
    } else if minutes >= 15 {
        let mins = (minutes / 5) * 5;
        format!("{mins}m")
    } else {
        format!("{minutes}m")
    }
}
pub fn render(dashboard: &Dashboard) -> Result<Frame, String> {
    let mut frame = Frame {
        packed: vec![255; WIDTH * HEIGHT / 8],
        visible_text: Vec::new(),
    };
    frame.dial(&dashboard.codex, dashboard.now.timestamp())?;
    frame.telemetry(dashboard)?;
    frame.radar(10, "CODEX", &dashboard.codex_radar, dashboard)?;
    frame.radar(204, "GEMINI", &dashboard.antigravity_radar, dashboard)?;
    Ok(frame)
}
