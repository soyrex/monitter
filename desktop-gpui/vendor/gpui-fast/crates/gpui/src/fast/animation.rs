use crate::{AsyncWindowContext, EntityId};
use std::{cell::Cell, rc::Rc};

/// Publish a timed animation change inside the display callback. Marking the
/// view dirty when the timer expires makes intervening keys synchronously draw
/// the whole window before dispatch. Keep the pending flag until the frame so
/// unrelated renders cannot enqueue duplicate animation callbacks.
pub(crate) fn notify_on_frame(
    cx: &mut AsyncWindowContext,
    view: EntityId,
    pending: Rc<Cell<bool>>,
) {
    cx.on_next_frame(move |_, cx| {
        pending.set(false);
        cx.notify(view);
    });
}
