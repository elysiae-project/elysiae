use crate::{
    core::game::Game,
    util::{
        cache::{self, ResolvedMedia},
        runtime,
        settings::{display_locale, get_option},
    },
};
use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};
    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/background.ui")]
    pub struct Background {
        #[template_child]
        pub background: TemplateChild<gtk::Picture>,
        #[template_child]
        pub video: TemplateChild<gtk::Video>,
        #[template_child]
        pub overlay: TemplateChild<gtk::Picture>,
        #[template_child]
        pub fallback: TemplateChild<gtk::Box>,
        pub request: Cell<u64>,
        pub game: Cell<Option<Game>>,
        pub key: RefCell<Option<(std::path::PathBuf, bool, Option<std::path::PathBuf>)>>,
        pub video_generation: Cell<u64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Background {
        const NAME: &'static str = "ElysiaeBackground";
        type Type = super::Background;
        type ParentType = gtk::Box;
        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }
    impl ObjectImpl for Background {
        fn dispose(&self) {
            self.video.set_autoplay(false);
            self.video.set_filename(None::<&std::path::Path>);
            self.dispose_template();
        }
    }
    impl WidgetImpl for Background {}
    impl BoxImpl for Background {}
}

glib::wrapper! {
    pub struct Background(ObjectSubclass<imp::Background>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Background {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn set_media(&self, game: Game) {
        let imp = self.imp();
        let changed_game = imp.game.replace(Some(game)) != Some(game);
        if changed_game {
            self.clear_media();
        }
        let request = imp.request.get().wrapping_add(1);
        imp.request.set(request);
        let locale = display_locale().unwrap_or_else(|_| "en-us".into());
        let use_video = get_option("use-video-background")
            .and_then(bool::try_from)
            .unwrap_or(false);
        let (sender, receiver) = async_channel::bounded(1);
        runtime::spawn(async move {
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let media = cache::resolve_media(game, &locale, use_video)?;
                let poster = if media.is_video {
                    cache::resolve_media(game, &locale, false)?
                } else {
                    ResolvedMedia::default()
                };
                Ok((media, poster))
            })
            .await;
            let _ = sender.send(result).await;
        });
        let weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = receiver.recv().await else {
                return;
            };
            let Some(widget) = weak.upgrade() else {
                return;
            };
            if widget.imp().request.get() != request {
                return;
            }
            match result {
                Ok(Ok((media, poster))) => {
                    if media.background.is_none() && changed_game {
                        let _ = widget.activate_action("app.refresh-assets", None);
                    }
                    widget.apply_media(media, poster);
                }
                // Keep usable artwork on refresh failure. A new game already
                // has the immediate themed fallback, never a cleared black layer.
                Ok(Err(error)) => log::debug!("Artwork unavailable: {error:#}"),
                Err(error) => log::warn!("Artwork lookup failed: {error}"),
            }
        });
    }

    fn apply_media(&self, media: ResolvedMedia, poster: ResolvedMedia) {
        let Some(path) = media.background.clone() else {
            return;
        };
        let key = (path.clone(), media.is_video, media.overlay.clone());
        if self.imp().key.borrow().as_ref() == Some(&key) {
            return;
        }
        let picture_path = if media.is_video {
            poster.background.as_ref()
        } else {
            Some(&path)
        };
        let texture = picture_path.and_then(|path| gtk::gdk::Texture::from_filename(path).ok());
        if !media.is_video && texture.is_none() {
            log::warn!("Background image could not be decoded: {}", path.display());
            return;
        }
        let paths: Vec<_> = media
            .background
            .iter()
            .chain(media.overlay.iter())
            .chain(poster.background.iter())
            .chain(poster.overlay.iter())
            .cloned()
            .collect();
        let imp = self.imp();
        imp.video.set_autoplay(false);
        imp.video.set_filename(None::<&std::path::Path>);
        imp.video.set_opacity(0.0);
        if let Err(error) = cache::set_active_assets(&paths) {
            log::warn!("Could not pin background media: {error:#}");
            return;
        }
        imp.background.set_paintable(texture.as_ref());
        imp.background.set_visible(texture.is_some());
        imp.fallback.set_visible(texture.is_none());
        let initial_overlay = if media.is_video {
            poster.overlay.as_ref()
        } else {
            media.overlay.as_ref()
        };
        imp.overlay.set_filename(initial_overlay);
        imp.overlay.set_visible(initial_overlay.is_some());
        *imp.key.borrow_mut() = Some(key);
        let generation = imp.video_generation.get().wrapping_add(1);
        imp.video_generation.set(generation);
        if !media.is_video {
            return;
        }
        // Keep the poster underneath a mapped, initially transparent player.
        // Hiding GtkVideo prevents autoplay; reveal only after playback advances.
        imp.video.set_filename(Some(&path));
        if let Some(stream) = imp.video.media_stream() {
            stream.set_muted(true);
            let weak = self.downgrade();
            let overlay = media.overlay.clone();
            stream.connect_timestamp_notify(move |stream| {
                if stream.timestamp() <= 0 {
                    return;
                }
                let Some(widget) = weak.upgrade() else {
                    return;
                };
                if widget.imp().video.opacity() >= 1.0 {
                    return;
                }
                if widget.imp().video_generation.get() != generation || stream.error().is_some() {
                    return;
                }
                widget.imp().video.set_opacity(1.0);
                widget.imp().overlay.set_filename(overlay.as_ref());
                widget.imp().overlay.set_visible(overlay.is_some());
            });
            let weak = self.downgrade();
            stream.connect_error_notify(move |stream| {
                if stream.error().is_some()
                    && let Some(widget) = weak.upgrade()
                    && widget.imp().video_generation.get() == generation
                {
                    widget.imp().video.set_opacity(0.0);
                    log::warn!("Video background unavailable; retaining static fallback");
                }
            });
        }
        imp.video.set_autoplay(true);
    }

    fn clear_media(&self) {
        let imp = self.imp();
        imp.video_generation
            .set(imp.video_generation.get().wrapping_add(1));
        imp.video.set_autoplay(false);
        imp.video.set_filename(None::<&std::path::Path>);
        imp.video.set_opacity(0.0);
        imp.background.set_paintable(None::<&gtk::gdk::Texture>);
        imp.background.set_visible(false);
        imp.overlay.set_visible(false);
        imp.overlay.set_filename(None::<&std::path::Path>);
        imp.fallback.set_visible(true);
        *imp.key.borrow_mut() = None;
        if let Err(error) = cache::set_active_assets(&[]) {
            log::debug!("Could not clear media pins: {error:#}");
        }
    }
}
impl Default for Background {
    fn default() -> Self {
        Self::new()
    }
}
