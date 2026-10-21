//! The keyboard shortcuts sheet (F1 or ?). Keep in step with the key handlers in `window.rs`.

use crate::ctx::Ctx;
use adw::prelude::*;
use std::rc::Rc;

const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    ("Playback", &[
        ("Play / pause", "Space"),
        ("Previous / next song", "Ctrl + ← / →"),
        ("Seek 5 seconds", "Shift + ← / →"),
        ("Volume", "Ctrl + ↑ / ↓"),
        ("Like the current song", "Ctrl + L"),
    ]),
    ("Navigation", &[
        ("Command palette: go anywhere, run anything", "Ctrl + K"),
        ("Search", "Ctrl + F"),
        ("Preferences", "Ctrl + ,"),
        ("Close now playing / full screen", "Esc"),
        ("Full screen (now playing)", "F / F11"),
    ]),
    ("Lists", &[
        ("Open the menu for a song or card", "Right-click · Menu · Shift + F10"),
        ("Select several songs", "Ctrl-click, Shift-click"),
        ("Reorder queue and playlists", "Drag a row"),
    ]),
    ("Help", &[("This sheet", "F1 or ?")]),
];

pub fn show(ctx: &Rc<Ctx>) {
    let dialog = adw::Dialog::builder().title("Keyboard shortcuts").content_width(460).content_height(560).build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let page = adw::PreferencesPage::new();
    for (title, rows) in SECTIONS {
        let group = adw::PreferencesGroup::builder().title(*title).build();
        for (what, keys) in *rows {
            let row = adw::ActionRow::builder().title(*what).build();
            row.add_suffix(&gtk::Label::builder().label(*keys).css_classes(["dim", "numeric"]).valign(gtk::Align::Center).build());
            group.add(&row);
        }
        page.add(&group);
    }
    tv.set_content(Some(&page));
    dialog.set_child(Some(&tv));
    dialog.present(ctx.toasts.root().as_ref());
}
