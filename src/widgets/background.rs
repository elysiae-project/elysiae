use gtk::{glib, prelude::*, subclass::prelude::ObjectSubclassIsExt};

use crate::{
    core::game::Game,
    util::{
        cache::{self, ResolvedMedia},
        runtime,
        settings::{display_locale, get_option},
    },
};

mod imp {
    use gtk::{CompositeTemplate, glib, prelude::*, subclass::prelude::*};
    use std::cell::{Cell, OnceCell};

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/app/elysiae/Elysiae/ui/background.ui")]
    pub struct Background {
        pub background: OnceCell<gtk::Picture>,
        pub video: OnceCell<gtk::Video>,
        pub overlay: OnceCell<gtk::Picture>,
        pub request: Cell<u64>,
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

    impl Background {
        pub fn initialize_layers(&self) {
            if self.background.get().is_some() {
                return;
            }
            // Reparent only after GtkBuilder finishes construction; doing this
            // in constructed() destroys children with queued notify signals.
            let obj = self.obj();
            // Accept both the original picture-only template and the user's
            // newer video template without changing resource files. Optional
            // layers must not be required template bindings in older installs.
            let root = obj
                .first_child()
                .and_downcast::<gtk::Overlay>()
                .unwrap_or_default();
            let background = super::named_child(&root, "background")
                .and_downcast::<gtk::Picture>()
                .unwrap_or_else(|| {
                    gtk::Picture::builder()
                        .can_shrink(true)
                        .content_fit(gtk::ContentFit::Cover)
                        .build()
                });
            let video = super::named_child(&root, "video")
                .and_downcast::<gtk::Video>()
                .unwrap_or_default();
            let overlay = super::named_child(&root, "overlay")
                .and_downcast::<gtk::Picture>()
                .unwrap_or_default();
            root.set_child(Some(&background));
            if video.parent().is_some() {
                root.remove_overlay(&video);
            }
            if overlay.parent().is_some() {
                root.remove_overlay(&overlay);
            }
            root.add_overlay(&video);
            root.add_overlay(&overlay);
            for widget in [
                root.upcast_ref::<gtk::Widget>(),
                background.upcast_ref(),
                video.upcast_ref(),
                overlay.upcast_ref(),
            ] {
                widget.set_hexpand(true);
                widget.set_vexpand(true);
            }
            video.set_loop(true);
            if root.parent().is_none() {
                obj.append(&root);
            }
            self.background
                .set(background)
                .expect("Picture initialized once");
            self.video.set(video).expect("Video initialized once");
            self.overlay.set(overlay).expect("Overlay initialized once");
            obj.clear_media();
        }
    }
    impl ObjectImpl for Background {}
    impl WidgetImpl for Background {}
    impl BoxImpl for Background {}
}

fn named_child(parent: &impl IsA<gtk::Widget>, id: &str) -> Option<gtk::Widget> {
    let mut child = parent.first_child();
    while let Some(widget) = child {
        if widget.buildable_id().as_deref() == Some(id) {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

glib::wrapper! {
    pub struct Background(ObjectSubclass<imp::Background>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Background {
    pub fn new() -> Self {
        let widget: Self = glib::Object::builder().build();
        widget.imp().initialize_layers();
        widget
    }

    pub fn set_media(&self, game: Game) {
        self.imp().initialize_layers();
        let request = self.imp().request.get().wrapping_add(1);
        self.imp().request.set(request);
        let locale = display_locale().unwrap_or_else(|error| {
            log::warn!("Invalid display locale: {error:#}");
            "en-us".into()
        });
        let use_video = get_option("use-video-background")
            .and_then(bool::try_from)
            .unwrap_or(false);
        let (sender, receiver) = async_channel::bounded(1);
        runtime::spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || cache::resolve_media(game, &locale, use_video))
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
                Ok(Ok(media)) => widget.apply_media(media),
                Ok(Err(error)) => {
                    log::debug!("No cached media for {}: {error:#}", game.code());
                    widget.clear_media();
                }
                Err(error) => {
                    log::error!("Media lookup failed: {error}");
                    widget.clear_media();
                }
            }
        });
    }

    fn apply_media(&self, media: ResolvedMedia) {
        let paths: Vec<_> = media
            .background
            .iter()
            .chain(media.overlay.iter())
            .cloned()
            .collect();
        if let Err(error) = cache::set_active_assets(&paths) {
            log::warn!("Cached media changed before display: {error:#}");
            return;
        }
        let video = self.imp().video.get().expect("constructed video");
        let overlay = self.imp().overlay.get().expect("constructed overlay");
        // Stop the old stream even when switching from one video to another.
        video.set_autoplay(false);
        video.set_filename(None::<&std::path::Path>);
        let background = self.imp().background.get().expect("constructed picture");
        background.set_filename(None::<&std::path::Path>);
        background.set_visible(!media.is_video && media.background.is_some());
        video.set_visible(media.is_video && media.background.is_some());
        if media.is_video {
            video.set_filename(media.background.as_deref());
            if let Some(stream) = video.media_stream() {
                stream.set_muted(true);
            }
            video.set_autoplay(true);
        } else {
            background.set_filename(media.background.as_deref());
        }
        overlay.set_filename(media.overlay.as_deref());
        overlay.set_visible(media.overlay.is_some());
    }

    fn clear_media(&self) {
        if let Some(background) = self.imp().background.get() {
            background.set_visible(false);
            background.set_filename(None::<&std::path::Path>);
        }
        if let Some(video) = self.imp().video.get() {
            video.set_autoplay(false);
            video.set_filename(None::<&std::path::Path>);
            video.set_visible(false);
        }
        if let Some(overlay) = self.imp().overlay.get() {
            overlay.set_filename(None::<&std::path::Path>);
            overlay.set_visible(false);
        }
        if let Err(error) = cache::set_active_assets(&[]) {
            log::warn!("Could not release media pins: {error:#}");
        }
    }
}

impl Default for Background {
    fn default() -> Self {
        Self::new()
    }
}
