//! Tray icon near the clock (like HDT): the panels have no frame nor taskbar button, so the menu is
//! here: show the window, my stats, win chance at the top, panel width, what's new, Hearthstone folder,
//! data folder, update, feedback, sharing, quit. Windows only.

use super::App;

#[cfg(windows)]
mod imp {
    use super::App;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    pub struct Tray {
        _icon: TrayIcon,
        show: MenuItem,
        stats: MenuItem,
        odds: CheckMenuItem,
        boards: CheckMenuItem,
        narrower: MenuItem,
        wider: MenuItem,
        news: MenuItem,
        hs_dir: MenuItem,
        folder: MenuItem,
        update: MenuItem,
        feedback: MenuItem,
        sharing: CheckMenuItem,
        sent: MenuItem,
        erase: MenuItem,
        quit: MenuItem,
        shown_update: Option<String>,
    }

    impl Tray {
        pub fn new(app: &App) -> Option<Tray> {
            let share = owlingstone::share::Share::default();
            let show = MenuItem::new("Show window (turns off auto-hide)", true, None);
            let stats = MenuItem::new("My stats", true, None);
            let odds = CheckMenuItem::new("Win chance at the top", true, app.top_odds(), None);
            let boards = CheckMenuItem::new("Opponent board on portrait hover", true, app.settings.board_on_hover(), None);
            let narrower = MenuItem::new("Side panels narrower", true, None);
            let wider = MenuItem::new("Side panels wider", true, None);
            let news = MenuItem::new("What's new", true, None);
            let hs_dir = MenuItem::new("Choose Hearthstone folder…", true, None);
            let folder = MenuItem::new("Open my data folder", true, None);
            let update = MenuItem::new("No update ready", false, None);
            let feedback = MenuItem::new("Send feedback…", true, None);
            let sharing = CheckMenuItem::new("Share my games (anonymous)", true, share.enabled(), None);
            let sent = MenuItem::new("Show what is sent", true, None);
            let erase = MenuItem::new("Delete my shared games", true, None);
            let quit = MenuItem::new("Quit Owlingstone", true, None);
            let menu = Menu::new();
            let sep = PredefinedMenuItem::separator;
            menu.append_items(&[
                &show, &stats, &odds, &boards, &narrower, &wider, &news, &hs_dir, &folder, &update, &sep(), &feedback, &sharing, &sent,
                &erase, &sep(), &quit,
            ])
            .ok()?;
            let data = super::super::icon();
            let mut b = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip(format!("Owlingstone v{}", owlingstone::VERSION));
            if let Ok(i) = Icon::from_rgba(data.rgba, data.width, data.height) {
                b = b.with_icon(i);
            }
            let icon = b.build().ok()?;
            Some(Tray {
                _icon: icon,
                show,
                stats,
                odds,
                boards,
                narrower,
                wider,
                news,
                hs_dir,
                folder,
                update,
                feedback,
                sharing,
                sent,
                erase,
                quit,
                shown_update: None,
            })
        }

        pub fn poll(&mut self, app: &mut App) {
            if app.update_ready != self.shown_update {
                self.shown_update = app.update_ready.clone();
                if let Some(v) = &self.shown_update {
                    self.update.set_text(format!("Update to v{v}"));
                    self.update.set_enabled(true);
                }
            }
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                let id = ev.id;
                if id == *self.show.id() {
                    app.settings.set_flag("hide_when_unfocused", false);
                    app.save_settings();
                } else if id == *self.stats.id() {
                    app.dialogs.stats = Some(String::new());
                } else if id == *self.odds.id() {
                    app.settings.set_flag("top_odds", self.odds.is_checked());
                    app.save_settings();
                } else if id == *self.boards.id() {
                    app.settings.set_flag("board_on_hover", self.boards.is_checked());
                    app.save_settings();
                } else if id == *self.narrower.id() {
                    app.resize_panels(-super::super::PANEL_STEP);
                } else if id == *self.wider.id() {
                    app.resize_panels(super::super::PANEL_STEP);
                } else if id == *self.news.id() {
                    app.show_whats_new();
                } else if id == *self.hs_dir.id() {
                    super::super::choose_hearthstone_dir(app);
                } else if id == *self.folder.id() {
                    super::super::open_data_folder();
                } else if id == *self.update.id() {
                    app.run_update();
                } else if id == *self.feedback.id() {
                    app.dialogs.feedback = Some(super::super::dialogs::new_feedback());
                } else if id == *self.sharing.id() {
                    let on = self.sharing.is_checked();
                    if let Err(e) = owlingstone::share::Share::default().set_enabled(on) {
                        owlingstone::errlog::note("sharing.json", &e.to_string());
                    }
                } else if id == *self.sent.id() {
                    app.dialogs.sent = true;
                } else if id == *self.erase.id() {
                    app.dialogs.confirm_delete = true;
                } else if id == *self.quit.id() {
                    app.quit();
                }
            }
            // Keep the ticks in step with changes made elsewhere (sharing question, deletion).
            let sharing = owlingstone::share::Share::default().enabled();
            if self.sharing.is_checked() != sharing {
                self.sharing.set_checked(sharing);
            }
            if self.odds.is_checked() != app.top_odds() {
                self.odds.set_checked(app.top_odds());
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::App;
    pub struct Tray;
    impl Tray {
        pub fn new(_app: &App) -> Option<Tray> {
            None
        }
        pub fn poll(&mut self, _app: &mut App) {}
    }
}

pub use imp::Tray;
