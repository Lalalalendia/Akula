//! Resource ownership is inside the guest, authority remains inside the host.
use std::cell::RefCell;

wit_bindgen::generate!({
    world: "resource-lab",
    path: "wit",
});

use akula::plugin_resource::notes::{self, Note};

thread_local! {
    static HELD: RefCell<Option<Note>> = const { RefCell::new(None) };
}

struct GuestImpl;

impl Guest for GuestImpl {
    fn acquire() -> bool {
        let note = notes::open_active();
        HELD.with(|slot| {
            *slot.borrow_mut() = note;
            slot.borrow().is_some()
        })
    }

    fn read_held() -> i32 {
        HELD.with(|slot| match slot.borrow().as_ref() {
            Some(note) => note.read_first(),
            None => -1,
        })
    }

    fn release() {
        HELD.with(|slot| {
            // Dropping an owned Component Resource calls the host destructor.
            slot.borrow_mut().take();
        });
    }
}

export!(GuestImpl);
