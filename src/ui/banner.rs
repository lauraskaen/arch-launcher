//! Inline banner (undo / error / info) shown at the bottom of the card.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Info,
    Error,
}

/// Optional action button: label and callback.
pub type BannerAction<'a> = (&'a str, Rc<dyn Fn()>);

pub struct Banner {
    pub widget: gtk::Revealer,
    frame: gtk::Box,
    label: gtk::Label,
    action: gtk::Button,
    timer: RefCell<Option<glib::SourceId>>,
    callback: RefCell<Option<Rc<dyn Fn()>>>,
    generation: Cell<u64>,
}

impl Banner {
    pub fn new() -> Rc<Self> {
        let frame = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        frame.add_css_class("banner");
        let label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .selectable(false)
            .build();
        let action = gtk::Button::builder().css_classes(["accent"]).focus_on_click(false).visible(false).build();
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .css_classes(["flat", "icon-button"])
            .focus_on_click(false)
            .tooltip_text("Dispensar")
            .build();
        frame.append(&label);
        frame.append(&action);
        frame.append(&close);
        let widget = gtk::Revealer::builder()
            .child(&frame)
            .reveal_child(false)
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(180)
            .build();

        let this = Rc::new(Self {
            widget,
            frame,
            label,
            action: action.clone(),
            timer: RefCell::new(None),
            callback: RefCell::new(None),
            generation: Cell::new(0),
        });
        let weak = Rc::downgrade(&this);
        close.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(b) = weak.upgrade() {
                    b.hide();
                }
            }
        });
        action.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                let cb = b.callback.borrow_mut().take();
                b.hide();
                if let Some(cb) = cb {
                    cb();
                }
            }
        });
        this
    }

    pub fn set_animated(&self, animated: bool) {
        self.widget.set_transition_type(if animated {
            gtk::RevealerTransitionType::SlideUp
        } else {
            gtk::RevealerTransitionType::None
        });
    }

    /// Shows a message. `timeout` of `None` keeps it until dismissed.
    pub fn show(
        self: &Rc<Self>,
        text: &str,
        kind: BannerKind,
        action: Option<BannerAction<'_>>,
        timeout: Option<Duration>,
    ) {
        self.cancel_timer();
        self.generation.set(self.generation.get() + 1);
        self.label.set_text(text);
        match kind {
            BannerKind::Error => self.frame.add_css_class("error"),
            BannerKind::Info => self.frame.remove_css_class("error"),
        }
        match action {
            Some((label, cb)) => {
                self.action.set_label(label);
                self.action.set_visible(true);
                *self.callback.borrow_mut() = Some(cb);
            }
            None => {
                self.action.set_visible(false);
                *self.callback.borrow_mut() = None;
            }
        }
        self.widget.set_reveal_child(true);
        if let Some(t) = timeout {
            let weak = Rc::downgrade(self);
            let generation = self.generation.get();
            let id = glib::timeout_add_local_once(t, move || {
                if let Some(b) = weak.upgrade() {
                    // The source is already gone; forget its id.
                    b.timer.borrow_mut().take();
                    if b.generation.get() == generation {
                        b.hide();
                    }
                }
            });
            *self.timer.borrow_mut() = Some(id);
        }
    }

    pub fn info(self: &Rc<Self>, text: &str) {
        self.show(text, BannerKind::Info, None, Some(Duration::from_secs(4)));
    }

    pub fn error(self: &Rc<Self>, text: &str) {
        self.show(text, BannerKind::Error, None, Some(Duration::from_secs(10)));
    }

    fn cancel_timer(&self) {
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
    }

    pub fn hide(&self) {
        self.cancel_timer();
        self.callback.borrow_mut().take();
        self.widget.set_reveal_child(false);
    }
}
