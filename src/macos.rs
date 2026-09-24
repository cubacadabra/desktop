use std::cell::{Cell, OnceCell};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenu};
use objc2_foundation::{NSObject, ns_string};

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(showAbout:))]
        fn show_about(&self, _sender: Option<&AnyObject>) {
            ABOUT_REQUESTED.with(|requested| requested.set(true));
        }
    }
);

thread_local! {
    static MENU_TARGET: OnceCell<Retained<MenuTarget>> = const { OnceCell::new() };
    static ABOUT_REQUESTED: Cell<bool> = const { Cell::new(false) };
}

impl MenuTarget {
    fn new(main_thread: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(main_thread).set_ivars(());
        // SAFETY: NSObject's initializer is valid for this ivar-free subclass.
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) fn take_about_requested() -> bool {
    ABOUT_REQUESTED.with(|requested| requested.replace(false))
}

pub(crate) fn install_native_menu() {
    let main_thread =
        MainThreadMarker::new().expect("Desktop's event loop must run on the main thread");
    let application = NSApplication::sharedApplication(main_thread);
    let Some(main_menu) = application.mainMenu() else {
        log::warn!("macOS menu installation skipped: AppKit has no main menu yet");
        return;
    };
    let Some(application_menu_item) = main_menu.itemAtIndex(0) else {
        log::warn!("macOS menu installation skipped: AppKit has no application menu item yet");
        return;
    };
    let Some(application_menu) = application_menu_item.submenu() else {
        log::warn!("macOS menu installation skipped: application menu has no submenu yet");
        return;
    };

    application_menu_item.setTitle(ns_string!("Cubacadabra Desktop"));
    application_menu.setTitle(ns_string!("Cubacadabra Desktop"));

    MENU_TARGET.with(|target| {
        let target = target.get_or_init(|| MenuTarget::new(main_thread));
        configure_application_menu(&application_menu, target);
    });
}

fn configure_application_menu(menu: &NSMenu, target: &AnyObject) {
    let Some(about_item) = menu.itemAtIndex(0) else {
        log::warn!("macOS menu installation skipped: application menu has no About item");
        return;
    };
    about_item.setTitle(ns_string!("About Cubacadabra Desktop"));
    // SAFETY: `showAbout:` is registered on MenuTarget with the standard
    // one-argument menu action signature. MENU_TARGET retains the target.
    unsafe {
        about_item.setTarget(Some(target));
        about_item.setAction(Some(sel!(showAbout:)));
    }

    if let Some(hide_item) = menu.itemAtIndex(3) {
        hide_item.setTitle(ns_string!("Hide Cubacadabra Desktop"));
    }
    if let Some(quit_item) = menu.itemAtIndex(7) {
        quit_item.setTitle(ns_string!("Quit Cubacadabra Desktop"));
    }
}
