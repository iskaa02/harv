//! Font files shipped inside the binary so the built-in themes render in their own typefaces
//! without anything installed. All are SIL OFL 1.1 (licences in `fonts/`). GUI-free: the app
//! writes them to a cache dir and registers them with fontconfig (see `apps/harv/src/fonts.rs`).
//!
//! Inter is deliberately not bundled: the blur theme relies on the system copy.

/// `(family name, font file bytes)`. One upright variable TTF per family.
pub const BUNDLED: &[(&str, &[u8])] = &[
    ("Figtree", include_bytes!("../fonts/Figtree-Variable.ttf")),
    ("Red Hat Display", include_bytes!("../fonts/RedHatDisplay-Variable.ttf")),
    ("Red Hat Mono", include_bytes!("../fonts/RedHatMono-Variable.ttf")),
];

/// True when `family` is one of the bundled families (case-insensitive), whether or not the system has it too.
pub fn is_bundled(family: &str) -> bool {
    BUNDLED.iter().any(|(name, _)| name.eq_ignore_ascii_case(family.trim()))
}
