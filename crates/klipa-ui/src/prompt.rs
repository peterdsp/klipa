//! The one modal klipa ever shows: "how long?" for a custom keep-awake
//! session.
//!
//! klipa has no windows, so there is nothing to hang a form off. On macOS
//! `NSAlert` gives a native, sandbox-safe modal with an accessory text
//! field and no window of our own, which is exactly the size of dialog
//! this needs. Everywhere else there is no equivalent without pulling in
//! a GUI toolkit, so [`CUSTOM_DURATION_SUPPORTED`] is false and the menu
//! simply does not offer the item rather than offering a dead one.

/// Whether [`custom_minutes`] can actually ask the user anything here.
/// The tray hides the "Custom..." entry when this is false.
pub const CUSTOM_DURATION_SUPPORTED: bool = cfg!(target_os = "macos");

/// Ask for a session length in minutes.
///
/// `initial` pre-fills the field with the last value the user chose.
/// Returns `None` if they cancelled, left it blank, or typed something
/// that isn't a positive number of minutes. Blocks the main thread for as
/// long as the modal is up, which is fine: it is only ever called from a
/// menu click, and the tray menu has already closed by then.
#[cfg(target_os = "macos")]
pub fn custom_minutes(initial: Option<u64>) -> Option<u64> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSAlert, NSAlertStyle, NSApplication, NSTextField};
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

    // NSAlert is AppKit UI: main thread only. Every caller is a menu
    // handler on the event loop, so this holds; bail rather than risk UB
    // if that ever stops being true.
    let mtm = MainThreadMarker::new()?;

    // SAFETY: plain AppKit object construction and property setting, all
    // on the main thread, with objects kept alive by `Retained` for the
    // duration of the modal.
    unsafe {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Keep awake for how long?"));
        alert.setInformativeText(&NSString::from_str(
            "Enter a number of minutes. For no time limit, pick \"Indefinitely\" instead.",
        ));
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.addButtonWithTitle(&NSString::from_str("Start"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));

        let field: Retained<NSTextField> = NSTextField::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(220.0, 24.0)),
        );
        field.setStringValue(&NSString::from_str(
            &initial.map(|m| m.to_string()).unwrap_or_default(),
        ));
        alert.setAccessoryView(Some(&field));

        // klipa is a menubar accessory app and a menu click does not make
        // it active, so without this the alert can open behind the
        // frontmost app with no keyboard focus. `activate` would be the
        // modern call but needs macOS 14.
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);

        // NSAlertFirstButtonReturn: the "Start" button we added first.
        const FIRST_BUTTON: isize = 1000;
        if alert.runModal() != FIRST_BUTTON {
            return None;
        }
        parse_minutes(&field.stringValue().to_string())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn custom_minutes(_initial: Option<u64>) -> Option<u64> {
    None
}

/// Read a positive whole number of minutes out of what the user typed.
///
/// Deliberately strict and capped: a custom session is a *timed* one, and
/// the way to ask for no limit is the "Indefinitely" item, not a
/// preposterous number here. A year is far past any real use and keeps
/// the deadline arithmetic nowhere near overflowing.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_minutes(input: &str) -> Option<u64> {
    const MAX_MINUTES: u64 = 365 * 24 * 60;
    let minutes: u64 = input.trim().parse().ok()?;
    (1..=MAX_MINUTES).contains(&minutes).then_some(minutes)
}

#[cfg(test)]
mod tests {
    use super::parse_minutes;

    #[test]
    fn accepts_a_plain_positive_number() {
        assert_eq!(parse_minutes("45"), Some(45));
        assert_eq!(parse_minutes("  90 "), Some(90));
    }

    #[test]
    fn rejects_nonsense_and_zero() {
        assert_eq!(parse_minutes(""), None);
        assert_eq!(parse_minutes("0"), None);
        assert_eq!(parse_minutes("-5"), None);
        assert_eq!(parse_minutes("soon"), None);
        assert_eq!(parse_minutes("1.5"), None);
    }

    #[test]
    fn rejects_absurd_lengths_rather_than_faking_forever() {
        // "Indefinitely" is its own mode; a custom length must not be a
        // back door to a pretend-infinite timer.
        assert_eq!(parse_minutes(&u64::MAX.to_string()), None);
        assert_eq!(parse_minutes("525601"), None);
        assert_eq!(parse_minutes("525600"), Some(525600));
    }
}
