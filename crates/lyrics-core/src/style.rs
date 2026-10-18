//! How the lyrics screen looks. Pure data + the plain-mode mapping; the view turns it into drawing.

/// `Spotlight`: full-screen look (artwork backdrop, blur, glow). `Plain`: text only, coloured by the style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleKind {
    Spotlight,
    Plain,
}

/// Horizontal anchor of the lines. Only `Plain` honours it; `Spotlight` keeps its left / RTL-aware alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
}

/// What the full-screen view paints behind the lines. `Cover` is the animated artwork backdrop; the others are built from
/// [`LyricsStyle::palette`] (theme colours) and ignore the artwork. (No backdrop at all = `LyricsStyle::backdrop == false`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackdropKind {
    Cover,
    /// Slowly moving blobs of the accent over a theme-coloured gradient.
    Gradient,
    /// A flat theme colour with one soft accent glow; static, so the cheapest.
    Solid,
}

/// Arrangement of the full-screen view (wide windows; narrow ones always use the stacked compact layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutKind {
    /// Artwork and meta on the left, lyrics on the right.
    Split,
    /// A header row with a small cover, lyrics centred in a column below.
    Centered,
    /// Like `Centered` but without the cover.
    LyricsOnly,
}

/// RGBA (0..1) text colours used by `Plain`: the active line, the lines next to it, and far lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextColors {
    pub active: [f32; 4],
    pub near: [f32; 4],
    pub far: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct LyricsStyle {
    pub kind: StyleKind,
    /// Font px of far (inactive) lines. `None` = the layout's own size (Spotlight).
    pub font_px: Option<f32>,
    /// Font px of the active line. `None` = same as `font_px`.
    pub active_font_px: Option<f32>,
    /// Vertical gap between lines, px. `None` = layout default.
    pub line_gap: Option<f32>,
    /// Side gutter, px (Plain). `None` = layout default.
    pub pad: Option<f32>,
    pub align: Align,
    pub colors: TextColors,
    /// Multiplier on the Spotlight blur of lines away from the active one (0 = sharp).
    pub blur: f32,
    /// Multiplier on the Spotlight glow of the active line (0 = none).
    pub glow: f32,
    /// Draw the artwork-derived backdrop. Off = the view is transparent.
    pub backdrop: bool,
    /// Seconds each line below the active one waits in the cascade (default 0.05).
    pub cascade_delay: f32,
    /// Spring stiffness multiplier (1.0 = current feel).
    pub spring_stiffness: f32,
    /// Preferred font family tried before the built-in fallback stack (a theme's UI font). `None` = Inter first.
    pub font_family: Option<String>,
    /// Spotlight only: colour of the lines, title, artist and progress bar (white on the artwork backdrop).
    pub tint: [f32; 3],
    pub backdrop_kind: BackdropKind,
    /// `[background, accent]` for `Gradient` / `Solid`.
    pub palette: [[f32; 3]; 2],
    /// Colour of the dimming / vignette laid over a `Gradient` backdrop (the background colour: black would dirty a light theme).
    pub scrim: [f32; 3],
    pub layout: LayoutKind,
    /// Cover corner radius in px; `None` = the layout's own.
    pub cover_radius: Option<f32>,
    pub cover_shadow: bool,
}

impl Default for LyricsStyle {
    fn default() -> Self {
        Self::spotlight()
    }
}

impl LyricsStyle {
    /// The current full-screen look, unchanged.
    pub fn spotlight() -> Self {
        Self {
            kind: StyleKind::Spotlight,
            font_px: None,
            active_font_px: None,
            line_gap: None,
            pad: None,
            align: Align::Start,
            colors: TextColors { active: [1.0; 4], near: [1.0, 1.0, 1.0, 0.6], far: [1.0, 1.0, 1.0, 0.3] },
            blur: 1.0,
            glow: 1.0,
            backdrop: true,
            cascade_delay: 0.05,
            spring_stiffness: 1.0,
            font_family: None,
            tint: [1.0; 3],
            backdrop_kind: BackdropKind::Cover,
            palette: [[0.04, 0.04, 0.05], [1.0, 0.22, 0.37]],
            scrim: [0.04, 0.04, 0.05],
            layout: LayoutKind::Split,
            cover_radius: None,
            cover_shadow: true,
        }
    }

    /// A plain lyrics panel: no backdrop, blur or glow; size and colour fall off with distance from the active line.
    /// Colours default to light text for a dark page; pass `colors` for a light one.
    pub fn plain() -> Self {
        Self {
            kind: StyleKind::Plain,
            font_px: Some(20.0),
            active_font_px: Some(28.0),
            line_gap: Some(16.0),
            pad: Some(20.0),
            align: Align::Start,
            colors: TextColors { active: [1.0, 1.0, 1.0, 1.0], near: [1.0, 1.0, 1.0, 0.55], far: [1.0, 1.0, 1.0, 0.28] },
            blur: 0.0,
            glow: 0.0,
            backdrop: false,
            cascade_delay: 0.05,
            spring_stiffness: 1.0,
            font_family: None,
            tint: [1.0; 3],
            backdrop_kind: BackdropKind::Cover,
            palette: [[0.04, 0.04, 0.05], [1.0, 0.22, 0.37]],
            scrim: [0.04, 0.04, 0.05],
            layout: LayoutKind::Split,
            cover_radius: None,
            cover_shadow: true,
        }
    }

    pub fn is_plain(&self) -> bool {
        self.kind == StyleKind::Plain
    }

    /// Font px of the active line (the size lines are laid out at in Plain mode).
    pub fn active_px(&self) -> f32 {
        self.active_font_px.or(self.font_px).unwrap_or(28.0).max(1.0)
    }

    /// Far lines' size relative to the active line (0..1].
    pub fn far_ratio(&self) -> f32 {
        let far = self.font_px.unwrap_or(self.active_px());
        (far / self.active_px()).clamp(0.2, 1.0)
    }

    /// Plain mode, one line: (scale about its own box, colour). `dist` is the smoothed distance to the active line
    /// in lines, `focus` is 0..1 (1 = active). Lines are laid out at the active size, so scale 1.0 = active size.
    pub fn plain_look(&self, dist: f32, focus: f32) -> (f32, [f32; 4]) {
        let r = self.far_ratio();
        let f = focus.clamp(0.0, 1.0);
        // 0 for the active line's neighbours, 1 from three lines away.
        let far_t = ((dist - 1.0) / 2.0).clamp(0.0, 1.0);
        let rest = (1.0 + r) * 0.5 + (r - (1.0 + r) * 0.5) * far_t;
        let scale = rest + (1.0 - rest) * f;
        let c = &self.colors;
        let mix = |a: [f32; 4], b: [f32; 4], t: f32| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
        let col = mix(mix(c.near, c.far, far_t), c.active, f);
        (scale, col)
    }
}
