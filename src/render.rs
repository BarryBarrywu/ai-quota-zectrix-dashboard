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

use crate::model::{
    AntigravityQuota, CodexQuota, QuotaWindow, Radar, ResetKind, Scope, Source, WindowKind,
};
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
    fn dial(&mut self, source: &Source<CodexQuota>, now: i64) -> Result<(), String> {
        self.rect(0, 0, 214, 246, INK);
        self.label(&LABEL, "CODEX", 14..94, 20, Left, PAPER)?;
        if source.is_stale() {
            self.label(&BODY, "[旧]", 64..110, 20, Left, PAPER)?;
        }
        self.label(&BODY, "WEEKLY", 130..201, 20, Right, PAPER)?;
        let center = Point::new(107, 128);
        Arc::with_center(center, 137, 135.0.deg(), 270.0.deg())
            .into_styled(PrimitiveStyle::with_stroke(PAPER, 2))
            .draw(self)
            .unwrap();
        for i in 0..=20 {
            let angle = (135.0 + 13.5 * i as f64).to_radians();
            let point = |r: f64| {
                center
                    + Point::new(
                        (angle.cos() * r).round() as i32,
                        (angle.sin() * r).round() as i32,
                    )
            };
            let major = i % 5 == 0;
            Line::new(point(75.), point(if major { 83. } else { 79. }))
                .into_styled(PrimitiveStyle::with_stroke(
                    PAPER,
                    if major { 2 } else { 1 },
                ))
                .draw(self)
                .unwrap();
        }
        if let Some(quota) = source.value() {
            let w = &quota.weekly;
            if w.kind != WindowKind::Weekly || w.remaining_percent > 100 {
                return Err("invalid Codex weekly window".into());
            }
            if w.remaining_percent > 0 {
                Arc::with_center(
                    center,
                    125,
                    135.0.deg(),
                    (2.7 * w.remaining_percent as f32).deg(),
                )
                .into_styled(PrimitiveStyle::with_stroke(PAPER, 14))
                .draw(self)
                .unwrap();
            }
            self.label(
                &HERO,
                &w.remaining_percent.to_string(),
                56..158,
                140,
                Center,
                PAPER,
            )?;
            self.label(&BODY, "% 剩余", 67..147, 160, Center, PAPER)?;
            if let Some(n) = quota.reset_credits.filter(|n| *n > 0) {
                let width = if n > 999 { 154 } else { 90 };
                self.rect(107 - width / 2, 199, width as u32, 17, PAPER);
                self.label(
                    &BODY,
                    &format!("重置额度 ×{n}"),
                    107 - width / 2..107 + width / 2,
                    212,
                    Center,
                    INK,
                )?;
            }
            self.label(
                &LABEL,
                &format!(
                    "重置 {}",
                    countdown(now, w.resets_at.ok_or("missing Codex reset")?, false)
                ),
                8..206,
                229,
                Center,
                PAPER,
            )?;
        } else {
            self.label(&LABEL, "暂不可用", 50..164, 131, Center, PAPER)?;
            self.label(&BODY, "等待 Codex", 54..160, 153, Center, PAPER)?;
        }
        Ok(())
    }
    fn card(&mut self, y: i32, name: &str, w: &QuotaWindow, now: i64) -> Result<(), String> {
        if w.remaining_percent > 100 {
            return Err("invalid Antigravity percentage".into());
        }
        self.rect(226, y, 162, 82, INK);
        self.rect(228, y + 18, 158, 62, PAPER);
        self.label(&LABEL, name, 233..332, y + 14, Left, PAPER)?;
        let tag = match w.kind {
            WindowKind::Weekly => "周窗口",
            WindowKind::FiveHour => "5h窗口",
            WindowKind::Tied => "双窗口",
            WindowKind::Unspecified => "窗口",
        };
        self.label(&BODY, tag, 330..382, y + 14, Right, PAPER)?;
        self.label(
            &NUMBER,
            &format!("{}%", w.remaining_percent),
            234..319,
            y + 56,
            Left,
            INK,
        )?;
        self.label(&BODY, "重置", 318..380, y + 35, Right, INK)?;
        self.label(
            &BODY,
            &w.resets_at
                .map(|t| countdown(now, t, true))
                .unwrap_or_else(|| "--".into()),
            313..380,
            y + 55,
            Right,
            INK,
        )?;
        self.rect(234, y + 65, 146, 8, INK);
        self.rect(235, y + 66, 144, 6, PAPER);
        self.rect(
            235,
            y + 66,
            144 * u32::from(w.remaining_percent) / 100,
            6,
            INK,
        );
        Ok(())
    }
    fn telemetry(
        &mut self,
        dashboard: &Dashboard,
        source: &Source<AntigravityQuota>,
    ) -> Result<(), String> {
        let now = dashboard.now;
        let weekday = ["一", "二", "三", "四", "五", "六", "日"]
            [now.weekday().num_days_from_monday() as usize];
        self.label(
            &LABEL,
            &format!("{} 周{weekday}", now.format("%m/%d")),
            226..333,
            20,
            Left,
            INK,
        )?;
        let clock = dashboard
            .screen_updated_at
            .map(|t| t.with_timezone(now.offset()).format("%H:%M").to_string())
            .unwrap_or_else(|| "--:--".into());
        self.label(&LABEL, &clock, 340..389, 20, Right, INK)?;
        self.rect(226, 28, 162, 1, INK);
        self.label(&LABEL, "ANTIGRAVITY", 226..345, 47, Left, INK)?;
        if source.is_stale() {
            self.rect(321, 33, 20, 17, INK);
            self.rect(322, 34, 18, 15, PAPER);
            self.label(&BODY, "旧", 322..340, 46, Center, INK)?;
        }
        if let Some(q) = source.value() {
            self.card(58, "GEMINI", &q.gemini, now.timestamp())?;
            self.card(150, "CLAUDE/GPT", &q.claude_gpt, now.timestamp())?;
        } else {
            for y in (56..234).step_by(5) {
                for x in (226..388).step_by(5) {
                    self.rect(x, y, 1, 1, INK);
                }
            }
            self.rect(246, 119, 122, 52, INK);
            self.rect(248, 121, 118, 48, PAPER);
            self.label(&LABEL, "额度不可用", 250..364, 141, Center, INK)?;
            self.label(&BODY, "等待读取额度", 250..364, 160, Center, INK)?;
        }
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
                    format!("{provider} · 重置排期"),
                    format!(
                        "{}{date}{}{}",
                        if provider == "AGY" {
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
                    format!("{provider} · {status}"),
                    format!(
                        "{} · {}前",
                        if provider == "AGY" {
                            scope.label()
                        } else {
                            match kind {
                                ResetKind::Banked => "Banked",
                                ResetKind::Global => "Global",
                            }
                        },
                        countdown(*announced_at, d.now.timestamp(), true)
                    ),
                )
            }
            Some(Radar::NoSchedule) => (format!("{provider} RADAR"), "暂无排期".into()),
            None => (format!("{provider} RADAR"), "信号丢失".into()),
        };
        self.rect(x, 252, 188, 42, INK);
        if !active {
            self.rect(x + 1, 253, 186, 40, PAPER);
        }
        let color = if active { PAPER } else { INK };
        let end = if source.is_stale() {
            self.label(&BODY, "(旧)", x + 151..x + 184, 268, Right, color)?;
            x + 149
        } else {
            x + 181
        };
        self.label(&LABEL, &title, x + 7..end, 269, Left, color)?;
        self.label(&BODY, &detail, x + 7..x + 181, 287, Left, color)
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
fn countdown(from: i64, to: i64, compact: bool) -> String {
    let minutes = to.saturating_sub(from).max(0) / 60;
    let days = minutes / 1440;
    let hours = (minutes / 60) % 24;
    if days > 0 {
        format!("{days}天{hours}{}", if compact { "h" } else { "小时" })
    } else if hours > 0 {
        format!(
            "{hours}{}{}{}",
            if compact { "h" } else { "小时" },
            minutes % 60,
            if compact { "m" } else { "分" }
        )
    } else {
        format!("{minutes}{}", if compact { "m" } else { "分" })
    }
}
pub fn render(dashboard: &Dashboard) -> Result<Frame, String> {
    let mut frame = Frame {
        packed: vec![255; WIDTH * HEIGHT / 8],
        visible_text: Vec::new(),
    };
    frame.dial(&dashboard.codex, dashboard.now.timestamp())?;
    frame.telemetry(dashboard, &dashboard.antigravity)?;
    frame.rect(0, 246, 400, 2, INK);
    frame.radar(8, "CODEX", &dashboard.codex_radar, dashboard)?;
    frame.radar(204, "AGY", &dashboard.antigravity_radar, dashboard)?;
    Ok(frame)
}
