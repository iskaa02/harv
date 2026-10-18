//! Shared screen layout (logical px) so both front-ends line up identically.

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// Narrow window: lyrics full-width under a small header. Otherwise: art left, lyrics right.
    pub compact: bool,
    pub art: [f32; 3], // x, y, size
    pub art_radius: f32,
    pub title_pos: [f32; 2],
    pub title_font: f32,
    pub artist_pos: [f32; 2],
    pub artist_font: f32,
    pub meta_w: f32,
    pub bar: [f32; 4], // x, y, w, h
    pub lyrics_x: f32,
    pub lyrics_w: f32,
    pub lyrics_font: f32,
    pub line_gap: f32,
    pub anchor_y: f32,
    pub viewport: [f32; 2],
    /// Vertical extent lyric lines are clipped / faded to.
    pub lyrics_top: f32,
    pub lyrics_bottom: f32,
}

/// `controls_h`: height of a transport-controls strip the host overlays below the progress bar.
pub fn compute(w: f32, h: f32, top_inset: f32) -> Layout { compute_with(w, h, top_inset, 0.0, crate::style::LayoutKind::Split) }

pub fn compute_with(w: f32, h: f32, top_inset: f32, controls_h: f32, kind: crate::style::LayoutKind) -> Layout {
    use crate::style::LayoutKind;
    let compact = w < h * 1.25 || w < 760.0;
    if !compact && kind != LayoutKind::Split {
        // Header row (small cover + title) on top, lyrics centred in a column, progress bar centred at the bottom.
        let m = (w * 0.05).clamp(28.0, 64.0);
        let art = if kind == LayoutKind::Centered { 64.0 } else { 0.0 };
        let top = top_inset + 10.0;
        let tx = if art > 0.0 { m + art + 16.0 } else { m };
        let font = (h * 0.06).clamp(30.0, 68.0);
        let (lw, bw) = ((w * 0.62).min(1000.0), (w * 0.5).min(680.0));
        let bar_y = h - 30.0 - controls_h;
        let (ltop, lbottom) = (top + art.max(40.0) + 10.0, bar_y - 18.0);
        return Layout {
            compact: false,
            art: [m, top, art],
            art_radius: 10.0,
            title_pos: [tx, top + if art > 0.0 { 8.0 } else { 0.0 }],
            title_font: 18.0,
            artist_pos: [tx, top + if art > 0.0 { 8.0 } else { 0.0 } + 26.0],
            artist_font: 14.0,
            meta_w: (w * 0.4).min(420.0),
            bar: [(w - bw) / 2.0, bar_y, bw, 5.0],
            lyrics_x: (w - lw) / 2.0,
            lyrics_w: lw,
            lyrics_font: font,
            line_gap: font * 0.5,
            anchor_y: ltop + (lbottom - ltop) * 0.34,
            viewport: [w, h],
            lyrics_top: ltop,
            lyrics_bottom: lbottom,
        };
    }
    if compact {
        let m = (w * 0.06).clamp(20.0, 40.0);
        let art = 56.0;
        let top = top_inset + 8.0;
        let font = (w * 0.07).clamp(26.0, 44.0);
        Layout {
            compact,
            art: [m, top, art],
            art_radius: 8.0,
            title_pos: [m + art + 14.0, top + 6.0],
            title_font: 17.0,
            artist_pos: [m + art + 14.0, top + 30.0],
            artist_font: 14.0,
            meta_w: w - m * 2.0 - art - 14.0,
            bar: [m, h - 26.0 - controls_h, w - m * 2.0, 4.0],
            lyrics_x: m,
            lyrics_w: w - m * 2.0,
            lyrics_font: font,
            line_gap: font * 0.5,
            anchor_y: top + art + (h - top - art) * 0.17,
            viewport: [w, h],
            lyrics_top: top + art + 4.0,
            lyrics_bottom: h - 26.0 - controls_h - 12.0,
        }
    } else {
        let size = (w * 0.34).min(h * 0.56);
        let x = w * 0.07;
        let y = (h - size) * 0.5 - h * 0.07;
        let font = (h * 0.058).clamp(28.0, 66.0);
        let lx = w * 0.53;
        Layout {
            compact,
            art: [x, y, size],
            art_radius: (size * 0.025).max(8.0),
            title_pos: [x, y + size + h * 0.035],
            title_font: (h * 0.028).clamp(18.0, 30.0),
            artist_pos: [x, y + size + h * 0.035 + (h * 0.028).clamp(18.0, 30.0) * 1.5],
            artist_font: (h * 0.022).clamp(15.0, 24.0),
            meta_w: size,
            bar: [x, y + size + h * 0.035 + (h * 0.028).clamp(18.0, 30.0) * 1.5 + (h * 0.022).clamp(15.0, 24.0) * 2.0, size, 5.0],
            lyrics_x: lx,
            lyrics_w: w - lx - w * 0.06,
            lyrics_font: font,
            line_gap: font * 0.55,
            anchor_y: h * 0.30,
            viewport: [w, h],
            lyrics_top: 0.0,
            lyrics_bottom: h,
        }
    }
}

/// Plain lyrics panel (see `LyricsStyle::plain`): full width minus side gutters, no header, art or transport.
/// The bar is parked far off-screen so nothing can hit it; `lyrics_font` is the active size lines are laid out at.
pub fn plain(w: f32, h: f32, style: &crate::style::LyricsStyle) -> Layout {
    let active = style.active_px();
    let pad = style.pad.unwrap_or((w * 0.05).clamp(16.0, 32.0)).min(w * 0.25);
    Layout {
        compact: false,
        art: [0.0, 0.0, 0.0],
        art_radius: 0.0,
        title_pos: [0.0, 0.0],
        title_font: 0.0,
        artist_pos: [0.0, 0.0],
        artist_font: 0.0,
        meta_w: w - pad * 2.0,
        bar: [0.0, -100_000.0, 0.0, 0.0],
        lyrics_x: pad,
        lyrics_w: (w - pad * 2.0).max(1.0),
        lyrics_font: active,
        line_gap: style.line_gap.unwrap_or(active * 0.5),
        anchor_y: h * 0.5 - active * 0.6,
        viewport: [w, h],
        lyrics_top: 0.0,
        lyrics_bottom: h,
    }
}
