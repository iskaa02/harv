//! What a page hands to its layout. Pages load their data, wrap it in `PageData` and call `detail::compose`; the theme's
//! component tree (or the built-in composition) decides how it looks.

use crate::rows::RowExtras;
use player_core::QueueKind;
use std::rc::Rc;
use ytm_core::Track;

pub enum PageData {
    /// A song list with a header: album, playlist, local playlist, podcast, Favorites / Recent / Most played / Downloads.
    Detail(DetailData),
    /// A grid of cards: local playlists, followed artists.
    Grid(GridData),
}

pub struct DetailData {
    /// "Album", "Playlist", "Podcast" (shown upper-cased in the hero). Empty for library lists.
    pub kind_label: String,
    pub title: String,
    pub subtitle: String,
    pub thumb: Option<String>,
    /// Round cover (artists).
    pub round: bool,
    /// Built-in header without cover art (library lists): just the title and the buttons.
    pub simple: bool,
    /// Podcast description, shown under the header.
    pub description: Option<String>,
    pub tracks: Rc<Vec<Track>>,
    pub queue_kind: QueueKind,
    pub extras: RowExtras,
    /// Play / Shuffle / Download… (built fresh on every call; a widget can only have one parent).
    pub buttons: Box<dyn Fn() -> Vec<gtk::Widget>>,
    /// Shown instead of the list when there are no tracks: (icon, title, description).
    pub empty: Option<(String, String, String)>,
}

pub struct GridItem {
    pub thumb: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub round: bool,
    pub on_click: Rc<dyn Fn()>,
}

pub struct GridData {
    pub title: String,
    pub items: Vec<GridItem>,
    /// Buttons shown under the title ("New playlist").
    pub buttons: Box<dyn Fn() -> Vec<gtk::Widget>>,
    pub empty: Option<(String, String, String)>,
}
