//! Honors the operating system's reduced-motion preference.
//!
//! Every Base transition, spring, presence and reveal consults
//! [`App::reduce_motion`], but GPUI never reads the platform's setting into
//! that flag: it stays `false` until something sets it. [`init`] reads the
//! setting when Base initializes and writes it into the flag, so an
//! application inherits the user's choice by calling `gpui_base::init` or
//! `gpui_component::init`.
//!
//! The setting is read on the platforms below; everywhere else the flag is
//! left alone.
//!
//! - macOS: `NSWorkspace.accessibilityDisplayShouldReduceMotion`, the
//!   "Reduce motion" switch under Accessibility → Display, followed live via
//!   `NSWorkspace.notificationCenter` accessibility display notifications.
//! - Windows: `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`, the
//!   "Animation effects" switch under Accessibility → Visual effects, whose
//!   off state asks for reduced motion.
//! - Linux: the `reduced-motion` key of the `org.freedesktop.appearance`
//!   namespace of the XDG desktop portal's Settings interface, which GNOME
//!   and KDE back with their own animation switches. The portal answers over
//!   D-Bus, so the reading lands a moment after `init` returns, and Base keeps
//!   following the portal's change signal for the life of the application.
//!
//! An application owns the flag once it sets it. Base writes the
//! flag only while it still holds what Base last wrote (or GPUI's initial
//! `false`), so an application that calls [`App::set_reduce_motion`] after
//! `init` is never overridden by a later reading, and one that wants to follow
//! the system again calls [`set_system_reduce_motion_override`] with `None`.
//! Explicit overrides use that API to avoid ambiguous value-based ownership.
//!
//! Under GPUI's test scheduler the platform is never consulted, whichever
//! crate's tests are running: a probe that answers from another thread would
//! break the scheduler's determinism, and a test wanting reduced motion sets
//! the flag itself.

use gpui::{App, Global};

/// What Base last wrote into [`App::set_reduce_motion`], and whether it is
/// already listening for the platform to change its mind.
#[derive(Default)]
struct SystemReduceMotion {
    applied: Option<bool>,
    following: bool,
    explicit: Option<bool>,
}

impl Global for SystemReduceMotion {}

/// The last OS reading, independent of an application's explicit motion choice.
/// Observe this global to follow live accessibility changes without platform I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemMotionPreference(pub bool);
impl Global for SystemMotionPreference {}

/// Select an explicit policy, or resume following the last OS reading.
/// Unlike value-based ownership detection this preserves an override even when
/// its value happens to equal the OS value at the time it is selected.
pub fn set_system_reduce_motion_override(reduced: Option<bool>, cx: &mut App) {
    cx.default_global::<SystemReduceMotion>().explicit = reduced;
    let effective = reduced.or_else(|| cx.try_global::<SystemMotionPreference>().map(|v| v.0));
    if let Some(effective) = effective {
        cx.set_reduce_motion(effective);
        cx.default_global::<SystemReduceMotion>().applied = Some(effective);
    }
}

/// Applies the system's reduced-motion preference during `gpui_base::init`.
pub(crate) fn init(cx: &mut App) {
    apply_system_reduce_motion(cx);
}

/// Reads the operating system's reduced-motion preference into
/// [`App::set_reduce_motion`].
///
/// `gpui_base::init` calls this once. Call it again to re-read the preference
/// on a platform Base does not follow live (currently Windows). macOS follows
/// the NSWorkspace accessibility display notification without requiring a window.
///
/// The application wins over the system: when the flag no longer holds what
/// Base last wrote, the application set it, and this call leaves it alone.
/// Where the platform cannot say (wasm, an unsupported desktop, a Linux
/// session without the portal) the flag is left as it is, and under GPUI's
/// test scheduler the platform is not asked at all.
pub fn apply_system_reduce_motion(cx: &mut App) {
    if is_test_scheduler(cx) {
        return;
    }
    apply_preference(platform::read(), cx);
    let state = cx.default_global::<SystemReduceMotion>();
    if !state.following {
        state.following = true;
        platform::follow(cx);
    }
}

/// Whether the application runs on GPUI's deterministic test scheduler,
/// which must not be woken by a platform answering from its own thread.
fn is_test_scheduler(cx: &App) -> bool {
    cx.background_executor()
        .scheduler_executor()
        .scheduler()
        .as_test()
        .is_some()
}

/// Writes one reading of the system preference into the flag, unless the
/// application has taken the flag over since the reading Base last applied.
fn apply_preference(preference: Option<bool>, cx: &mut App) {
    let Some(reduce) = preference else {
        return;
    };
    if cx.try_global::<SystemMotionPreference>().map(|v| v.0) != Some(reduce) {
        cx.set_global(SystemMotionPreference(reduce));
    }
    if let Some(explicit) = cx.default_global::<SystemReduceMotion>().explicit {
        cx.set_reduce_motion(explicit);
        cx.default_global::<SystemReduceMotion>().applied = Some(explicit);
        return;
    }
    let applied = cx
        .try_global::<SystemReduceMotion>()
        .and_then(|state| state.applied);
    if cx.reduce_motion() != applied.unwrap_or(false) {
        return;
    }
    cx.set_reduce_motion(reduce);
    cx.default_global::<SystemReduceMotion>().applied = Some(reduce);
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::Mutex;

    use futures::{StreamExt as _, channel::mpsc};
    use gpui::{App, Global};
    use objc2::{AnyThread as _, DefinedClass as _, define_class, msg_send, rc::Retained, sel};
    use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject};

    // Notification delivery may occur on a posting thread. Only queue a wakeup
    // here; AppKit reads and all GPUI mutations run in the foreground task below.
    define_class!(
        #[unsafe(super = NSObject)]
        #[ivars = Mutex<mpsc::Sender<()>>]
        struct MotionObserver;

        impl MotionObserver {
            #[unsafe(method(accessibilityDisplayOptionsChanged:))]
            fn display_options_changed(&self, _notification: &NSNotification) {
                if let Ok(mut sender) = self.ivars().lock() {
                    // A pending wakeup already causes a fresh OS read. Coalesce
                    // display changes instead of accumulating unbounded work.
                    let _ = sender.try_send(());
                }
            }
        }
    );

    struct Observation {
        center: Retained<NSNotificationCenter>,
        observer: Retained<MotionObserver>,
    }
    impl Global for Observation {}
    impl Drop for Observation {
        fn drop(&mut self) {
            // SAFETY: The observer is retained through removal and registered
            // only on this center. Removal precedes release of both objects.
            unsafe { self.center.removeObserver(&self.observer) };
        }
    }

    pub(super) fn read() -> Option<bool> {
        Some(NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
    }

    pub(super) fn follow(cx: &mut App) {
        let (sender, mut changes) = mpsc::channel(1);
        let observer = MotionObserver::alloc().set_ivars(Mutex::new(sender));
        // SAFETY: NSObject's init initializes the allocated NSObject subclass.
        let observer: Retained<MotionObserver> = unsafe { msg_send![super(observer), init] };
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: The selector accepts one NSNotification argument and the
        // observer's lifetime is retained in the application global. The AppKit
        // notification constant is immutable and valid for the process lifetime.
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(accessibilityDisplayOptionsChanged:),
                Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
            );
        }
        cx.set_global(Observation { center, observer });
        cx.spawn(async move |cx| {
            while changes.next().await.is_some() {
                cx.update(|cx| super::apply_preference(read(), cx));
            }
        })
        .detach();
        // Close the read/register race without polling or adding a timer.
        super::apply_preference(read(), cx);
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use gpui::App;
    use windows::Win32::{
        Foundation::BOOL,
        UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        },
    };

    pub(super) fn read() -> Option<bool> {
        let mut animations_enabled = BOOL(0);
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through `pvParam`
        // and ignores `uiParam`; the BOOL outlives the call.
        unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&mut animations_enabled as *mut BOOL).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .ok()?;
        Some(!animations_enabled.as_bool())
    }

    pub(super) fn follow(_cx: &mut App) {}
}

#[cfg(target_os = "linux")]
mod platform {
    use ashpd::desktop::settings::{ReducedMotion, Settings};
    use futures::StreamExt as _;
    use gpui::App;

    /// The portal answers asynchronously; [`follow`] applies the first reading.
    pub(super) fn read() -> Option<bool> {
        None
    }

    pub(super) fn follow(cx: &mut App) {
        cx.spawn(async move |cx| {
            let settings = Settings::new().await.ok()?;
            let current = settings.reduced_motion().await.ok()?;
            cx.update(|cx| super::apply_preference(Some(reduces(current)), cx));
            let mut changes = settings.receive_reduced_motion_changed().await.ok()?;
            while let Some(preference) = changes.next().await {
                cx.update(|cx| super::apply_preference(Some(reduces(preference)), cx));
            }
            Some(())
        })
        .detach();
    }

    fn reduces(preference: ReducedMotion) -> bool {
        matches!(preference, ReducedMotion::ReducedMotion)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform {
    use gpui::App;

    pub(super) fn read() -> Option<bool> {
        None
    }

    pub(super) fn follow(_cx: &mut App) {}
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::{
        SystemMotionPreference, apply_preference, apply_system_reduce_motion,
        set_system_reduce_motion_override,
    };

    #[gpui::test]
    fn a_system_preference_for_reduced_motion_sets_the_flag(cx: &mut TestAppContext) {
        cx.update(|cx| {
            apply_preference(Some(true), cx);
            assert!(cx.reduce_motion());
        });
    }

    #[gpui::test]
    fn an_unknown_system_preference_leaves_the_flag_alone(cx: &mut TestAppContext) {
        cx.update(|cx| {
            apply_preference(None, cx);
            assert!(!cx.reduce_motion());

            cx.set_reduce_motion(true);
            apply_preference(None, cx);
            assert!(cx.reduce_motion());
        });
    }

    #[gpui::test]
    fn the_system_drives_the_flag_until_the_application_sets_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            apply_preference(Some(true), cx);
            apply_preference(Some(false), cx);
            assert!(!cx.reduce_motion());
            apply_preference(Some(true), cx);
            assert!(cx.reduce_motion());

            cx.set_reduce_motion(false);
            apply_preference(Some(true), cx);
            assert!(!cx.reduce_motion());
        });
    }

    #[gpui::test]
    fn explicit_overrides_survive_equal_os_values_and_resume_latest_reading(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            apply_preference(Some(false), cx);
            set_system_reduce_motion_override(Some(false), cx);
            apply_preference(Some(true), cx);
            assert!(!cx.reduce_motion());
            assert!(cx.global::<SystemMotionPreference>().0);
            set_system_reduce_motion_override(None, cx);
            assert!(cx.reduce_motion());
            apply_preference(Some(false), cx);
            assert!(!cx.reduce_motion());

            set_system_reduce_motion_override(Some(true), cx);
            apply_preference(Some(true), cx);
            apply_preference(Some(false), cx);
            assert!(cx.reduce_motion());
            assert!(!cx.global::<SystemMotionPreference>().0);
            set_system_reduce_motion_override(None, cx);
            assert!(!cx.reduce_motion());
        });
    }

    #[gpui::test]
    fn unchanged_display_notifications_do_not_republish_the_os_preference(cx: &mut TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let notifications = Rc::new(Cell::new(0));
        let count = notifications.clone();
        cx.update(|cx| {
            cx.observe_global::<SystemMotionPreference>(move |_| count.set(count.get() + 1))
                .detach();
        });
        cx.run_until_parked();
        cx.update(|cx| apply_preference(Some(false), cx));
        cx.run_until_parked();
        assert_eq!(notifications.get(), 1);
        cx.update(|cx| {
            apply_preference(Some(false), cx);
            apply_preference(None, cx);
        });
        cx.run_until_parked();
        assert_eq!(notifications.get(), 1);
        cx.update(|cx| apply_preference(Some(true), cx));
        cx.run_until_parked();
        assert_eq!(notifications.get(), 2);
    }

    #[gpui::test]
    fn the_test_scheduler_is_never_asked_for_the_platform_preference(cx: &mut TestAppContext) {
        cx.update(|cx| {
            apply_system_reduce_motion(cx);
            assert!(!cx.reduce_motion());
        });
        cx.run_until_parked();
        cx.update(|cx| assert!(!cx.reduce_motion()));
    }

    #[gpui::test]
    fn a_flag_the_application_set_before_the_first_reading_is_kept(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            apply_preference(Some(false), cx);
            assert!(cx.reduce_motion());
        });
    }
}
