use gtk::{glib, prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};

use crate::{
    core::game::Game,
    util::{
        cache::{self, AssetType},
        runtime,
        settings::display_locale,
    },
};

mod icon_imp {
    use super::*;

    #[derive(Default)]
    pub struct GameIcon {
        pub saturation: Cell<f32>,
        pub target: Cell<f32>,
        pub animating: Cell<bool>,
        pub picture: RefCell<Option<gtk::Picture>>,
        pub fallback: RefCell<Option<gtk::Image>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GameIcon {
        const NAME: &'static str = "ElysiaeGameIcon";
        type Type = super::GameIcon;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for GameIcon {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.add_css_class("game-icon");
            obj.set_overflow(gtk::Overflow::Hidden);
            let picture = gtk::Picture::builder()
                .can_shrink(true)
                .content_fit(gtk::ContentFit::Cover)
                .visible(false)
                .build();
            picture.add_css_class("game-icon-image");
            let fallback = gtk::Image::from_icon_name("applications-games-symbolic");
            fallback.add_css_class("game-icon-fallback");
            picture.set_parent(&*obj);
            fallback.set_parent(&*obj);
            *self.picture.borrow_mut() = Some(picture);
            *self.fallback.borrow_mut() = Some(fallback);
        }
        fn dispose(&self) {
            if let Some(picture) = self.picture.borrow_mut().take() {
                picture.unparent();
            }
            if let Some(fallback) = self.fallback.borrow_mut().take() {
                fallback.unparent();
            }
        }
    }
    impl WidgetImpl for GameIcon {
        // CSS controls the icon's size, not the downloaded texture's dimensions.
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(picture) = self.picture.borrow().as_ref()
                && picture.is_visible()
            {
                picture.allocate(width, height, baseline, None);
            }
            if let Some(fallback) = self.fallback.borrow().as_ref()
                && fallback.is_visible()
            {
                fallback.allocate(width, height, baseline, None);
            }
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            snapshot.push_color_matrix(
                &gtk::graphene::Matrix::from_float(super::saturation_matrix(self.saturation.get())),
                &gtk::graphene::Vec4::new(0.0, 0.0, 0.0, 0.0),
            );
            if let Some(picture) = self.picture.borrow().as_ref()
                && picture.is_visible()
            {
                self.obj().snapshot_child(picture, snapshot);
            }
            if let Some(fallback) = self.fallback.borrow().as_ref()
                && fallback.is_visible()
            {
                self.obj().snapshot_child(fallback, snapshot);
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct GameIcon(ObjectSubclass<icon_imp::GameIcon>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Color-matrix coefficients are luminance weights, not theme colors.
fn saturation_matrix(saturation: f32) -> [f32; 16] {
    let s = saturation.clamp(0.0, 1.0);
    let r = 0.2126 * (1.0 - s);
    let g = 0.7152 * (1.0 - s);
    let b = 0.0722 * (1.0 - s);
    [
        r + s,
        r,
        r,
        0.0,
        g,
        g + s,
        g,
        0.0,
        b,
        b,
        b + s,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

impl GameIcon {
    fn load(&self, path: &std::path::Path) {
        match gtk::gdk::Texture::from_filename(path) {
            Ok(texture) => {
                if let Some(picture) = self.imp().picture.borrow().as_ref() {
                    picture.set_paintable(Some(&texture));
                    picture.set_visible(true);
                }
                if let Some(fallback) = self.imp().fallback.borrow().as_ref() {
                    fallback.set_visible(false);
                }
            }
            Err(error) => log::warn!("Game icon unavailable: {error}"),
        }
    }

    fn set_saturation(&self, target: f64) {
        self.imp().target.set(target.clamp(0.0, 1.0) as f32);
        if !self.settings().is_gtk_enable_animations() {
            self.imp().saturation.set(self.imp().target.get());
            self.queue_draw();
            return;
        }
        if self.imp().animating.replace(true) {
            return;
        }
        self.add_tick_callback(|icon, _| {
            let imp = icon.imp();
            let delta = imp.target.get() - imp.saturation.get();
            if delta.abs() < 0.002 {
                imp.saturation.set(imp.target.get());
                imp.animating.set(false);
                icon.queue_draw();
                return glib::ControlFlow::Break;
            }
            imp.saturation.set(imp.saturation.get() + delta * 0.16);
            icon.queue_draw();
            glib::ControlFlow::Continue
        });
    }
}

type IconSource = (PathBuf, Option<std::time::SystemTime>);

struct GameEntry {
    game: Game,
    button: gtk::ToggleButton,
    icon: GameIcon,
    installed: Cell<bool>,
    progress: Cell<Option<f64>>,
    icon_request: Cell<u64>,
    icon_source: RefCell<Option<IconSource>>,
}

impl GameEntry {
    fn refresh(&self) {
        let progress = self.progress.get();
        self.icon.set_saturation(if self.installed.get() {
            1.0
        } else {
            progress.unwrap_or(0.0)
        });
        for (class, enabled) in [
            ("is-installed", self.installed.get()),
            ("is-downloading", progress.is_some()),
            ("is-uninstalled", !self.installed.get()),
        ] {
            if enabled {
                self.button.add_css_class(class);
            } else {
                self.button.remove_css_class(class);
            }
        }
        let description = match progress {
            Some(value) => format!(
                "{} — downloading {:.0}%",
                self.game.display_name(),
                value * 100.0
            ),
            None => format!(
                "{} — {}",
                self.game.display_name(),
                if self.installed.get() {
                    "installed"
                } else {
                    "not installed"
                }
            ),
        };
        self.button.set_tooltip_text(Some(&description));
        self.button.update_property(&[
            gtk::accessible::Property::Label(self.game.display_name()),
            gtk::accessible::Property::Description(&description),
        ]);
    }
}

mod imp {
    use super::*;
    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/sidebar.ui")]
    pub struct Sidebar {
        #[template_child]
        pub games: TemplateChild<gtk::Box>,
        pub(super) entries: RefCell<Vec<GameEntry>>,
        pub request: Cell<u64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "ElysiaeSidebar";
        type Type = super::Sidebar;
        type ParentType = gtk::Box;
        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }
    impl ObjectImpl for Sidebar {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("game-selected")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }
        fn constructed(&self) {
            self.parent_constructed();
            let mut entries = self.entries.borrow_mut();
            for game in Game::all() {
                let icon: GameIcon = glib::Object::builder().build();
                let button = gtk::ToggleButton::builder().child(&icon).build();
                button.add_css_class("game-button");
                if let Some(first) = entries.first() {
                    button.set_group(Some(&first.button));
                }
                let weak = self.obj().downgrade();
                button.connect_clicked(move |_| {
                    if let Some(sidebar) = weak.upgrade() {
                        sidebar.emit_by_name::<()>("game-selected", &[&game.code()]);
                    }
                });
                self.games.append(&button);
                let entry = GameEntry {
                    game,
                    button,
                    icon,
                    installed: Cell::new(false),
                    progress: Cell::new(None),
                    icon_request: Cell::new(0),
                    icon_source: RefCell::new(None),
                };
                entry.refresh();
                entries.push(entry);
            }
        }
    }
    impl WidgetImpl for Sidebar {}
    impl BoxImpl for Sidebar {}
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn connect_game_selected(&self, callback: impl Fn(Game) + 'static) {
        self.connect_local("game-selected", false, move |values| {
            if let Ok(code) = values[1].get::<String>()
                && let Ok(game) = Game::try_from(code.as_str())
            {
                callback(game);
            }
            None
        });
    }

    pub fn select(&self, game: Game) {
        for entry in self.imp().entries.borrow().iter() {
            let selected = entry.game == game;
            entry.button.set_active(selected);
            if selected {
                entry.button.add_css_class("is-selected");
            } else {
                entry.button.remove_css_class("is-selected");
            }
        }
    }

    pub fn set_installed(&self, game: Game, installed: bool) {
        if let Some(entry) = self.imp().entries.borrow().iter().find(|e| e.game == game) {
            entry.installed.set(installed);
            entry.refresh();
        }
    }

    pub fn set_progress(&self, game: Game, progress: Option<f64>) {
        if let Some(entry) = self.imp().entries.borrow().iter().find(|e| e.game == game) {
            entry.progress.set(progress.map(|v| v.clamp(0.0, 1.0)));
            entry.refresh();
        }
    }

    pub fn refresh_icons(&self) {
        self.refresh_icon_set(None);
    }

    pub fn refresh_icon(&self, game: Game) {
        self.refresh_icon_set(Some(game));
    }

    fn refresh_icon_set(&self, game: Option<Game>) {
        let locale = display_locale().unwrap_or_else(|_| "en-us".into());
        let request = self.imp().request.get().wrapping_add(1);
        self.imp().request.set(request);
        let games: Vec<_> = self
            .imp()
            .entries
            .borrow()
            .iter()
            .filter(|entry| game.is_none_or(|game| game == entry.game))
            .map(|entry| {
                entry.icon_request.set(request);
                entry.game
            })
            .collect();
        let (sender, receiver) = async_channel::bounded(1);
        runtime::spawn(async move {
            let paths = tokio::task::spawn_blocking(move || {
                games
                    .into_iter()
                    .map(|game| {
                        let path = cache::get_cached_asset_paths(game, &locale, AssetType::Icon)
                            .ok()
                            .and_then(|paths| paths.into_iter().next())
                            .and_then(|path| {
                                std::fs::metadata(&path)
                                    .ok()
                                    .map(|metadata| (path, metadata.modified().ok()))
                            });
                        (game, path)
                    })
                    .collect::<Vec<(Game, Option<IconSource>)>>()
            })
            .await;
            let _ = sender.send(paths).await;
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(Ok(paths)) = receiver.recv().await else {
                return;
            };
            let Some(sidebar) = weak.upgrade() else {
                return;
            };
            for (game, source) in paths {
                if let Some(source) = source
                    && let Some(entry) = sidebar
                        .imp()
                        .entries
                        .borrow()
                        .iter()
                        .find(|e| e.game == game)
                    && entry.icon_request.get() == request
                    && entry.icon_source.borrow().as_ref() != Some(&source)
                {
                    entry.icon.load(&source.0);
                    *entry.icon_source.borrow_mut() = Some(source);
                }
            }
        });
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saturation_preserves_alpha_and_full_color() {
        assert_eq!(
            saturation_matrix(1.0),
            [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0
            ]
        );
        let gray = saturation_matrix(0.0);
        for channel in 0..3 {
            assert_eq!(gray[channel], gray[0]);
            assert_eq!(gray[4 + channel], gray[4]);
            assert_eq!(gray[8 + channel], gray[8]);
        }
        assert_eq!(gray[15], 1.0);
        assert_eq!(saturation_matrix(-1.0), gray);
        assert_eq!(saturation_matrix(2.0), saturation_matrix(1.0));
    }
}
