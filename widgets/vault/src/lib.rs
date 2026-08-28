//! A credential that outlives the process holding it.
//!
//! The reference widget for `secrets`, and the shape almost every widget that
//! signs in to something ends up with: it has a token or it does not, it says
//! which, and it can forget one. What a real widget would do differently is
//! *where the token comes from* — a sign-in flow, a paste, an OAuth redirect —
//! and that is the only part this leaves out, because it needs a service to
//! sign in to.
//!
//! Three things here are worth copying, and one is worth not.
//!
//! * **Absence is not failure.** The first run has no token, and that is the
//!   normal case. It reaches the widget as `Ok(None)` rather than as an error
//!   whose string has to be read — see the ABI 4 → 5 note in `wit/`.
//! * **The value is loaded in `start`, not in `view`.** `view` runs on a
//!   sixteen-millisecond deadline and may run sixty times a second; `start`
//!   runs once. A widget that reads its credential every frame is a widget
//!   spending its whole budget on the host's storage.
//! * **The token is never shown, never logged, and never put in the view.**
//!   The view is composed by the host, hit-tested, and handed to an
//!   accessibility tree — a secret in it is a secret being read aloud by a
//!   screen reader. What is shown is whether there *is* one.
//! * **What not to copy: the token this stores is a constant.** A widget with
//!   a fixed credential in its own binary has not kept a secret, it has
//!   published one. This one is a demonstration of the API, and says so on
//!   screen.

use uniwidged_guest::prelude::*;

/// The key this widget keeps its credential under.
///
/// One key, named after what it is. The namespace is the host's — this widget
/// cannot name another's, and does not have to name its own.
const KEY: &str = "api.token";

/// Stands in for a token a real widget would be given by a sign-in.
const DEMONSTRATION_TOKEN: &[u8] = b"a token a real widget would have been given";

#[derive(Default)]
pub struct Vault {
    /// Whether a credential is stored — never the credential itself. A widget
    /// that keeps the bytes in memory for the sake of its view has widened
    /// what a bug in it can leak.
    stored: bool,
    /// What the host said, when it said something other than yes or no.
    /// Shown, because a widget that silently does nothing when the keychain is
    /// unavailable is one nobody can debug.
    trouble: Option<String>,
}

impl Widget for Vault {
    fn start(&mut self, cx: &Cx) -> Result<(), String> {
        self.reload(cx);
        // Not an error even when the keychain is unavailable: a widget that
        // refuses to start because it could not reach storage is one the user
        // sees as broken, where what is true is that it has nothing signed in.
        Ok(())
    }

    fn view(&mut self, _cx: &Cx) -> View {
        let (label, accent) = match (&self.trouble, self.stored) {
            (Some(_), _) => ("unavailable", Accent::Warning),
            (None, true) => ("signed in", Accent::Positive),
            (None, false) => ("not signed in", Accent::Neutral),
        };
        panel(NodeId::named("vault"))
            .gap(2)
            .child(text("Vault").role(Role::Section))
            .child(
                text(label)
                    .role(Role::Title)
                    .accent(accent)
                    .id(NodeId::named("state")),
            )
            .children(self.trouble.as_ref().map(|why| {
                text(why.clone())
                    .role(Role::Caption)
                    .accent(Accent::Warning)
                    .id(NodeId::named("trouble"))
            }))
            .child(
                row()
                    .gap(1)
                    .child(if self.stored {
                        chip("Sign out")
                            .on_click(NodeId::named("forget"))
                            .label("Forget the stored credential")
                    } else {
                        chip("Sign in")
                            .accent(Accent::Brand)
                            .on_click(NodeId::named("store"))
                            .label("Store a credential")
                    })
                    .child(spacer()),
            )
            .child(
                text("the token here is a demonstration, not a secret")
                    .role(Role::Caption)
                    .muted()
                    .id(NodeId::named("disclaimer")),
            )
            .view()
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {
        let Event::Clicked(id) = event else {
            return false;
        };
        if *id == NodeId::named("store") {
            match cx.set_secret(KEY, DEMONSTRATION_TOKEN) {
                Ok(()) => {
                    self.stored = true;
                    self.trouble = None;
                }
                Err(denied) => self.trouble = Some(explain(&denied)),
            }
        } else if *id == NodeId::named("forget") {
            match cx.remove_secret(KEY) {
                // `false` means there was nothing there — somebody removed it
                // from outside, or two clicks arrived. Signing out twice is
                // not a state to complain about.
                Ok(_) => {
                    self.stored = false;
                    self.trouble = None;
                }
                Err(denied) => self.trouble = Some(explain(&denied)),
            }
        } else {
            return false;
        }
        true
    }

    /// Nothing scheduled. A credential changes when somebody changes it, and
    /// asking to be woken anyway is what makes a desktop full of widgets cost
    /// something.
    fn wake(&self) -> Wake {
        Wake::Never
    }
}

impl Vault {
    /// Asks the keychain once whether there is anything there.
    ///
    /// The answer is a boolean, deliberately: what comes back is dropped at
    /// the end of this function rather than kept, because this widget has no
    /// use for the bytes and every copy of a credential is somewhere it can
    /// leak from.
    fn reload(&mut self, cx: &Cx) {
        match cx.get_secret(KEY) {
            Ok(found) => {
                self.stored = found.is_some();
                self.trouble = None;
            }
            Err(denied) => {
                self.stored = false;
                self.trouble = Some(explain(&denied));
            }
        }
    }
}

/// What to put on screen when the keychain said no.
///
/// The three answers mean genuinely different things, and a widget that shows
/// one message for all of them sends its user — or its author — to the wrong
/// place. `NotGranted` is the user's own decision and is fixed in the settings;
/// `Unavailable` is this desktop and is fixed by nobody; `Malformed` is this
/// widget's bug and is fixed by its author.
fn explain(denied: &Denied) -> String {
    match denied {
        Denied::NotGranted => "not allowed to use the keychain".into(),
        Denied::Unavailable(why) => format!("keychain unavailable: {why}"),
        Denied::Malformed(why) => format!("this widget asked wrongly: {why}"),
    }
}

uniwidged_guest::export!(Vault);

#[cfg(test)]
mod tests {
    use super::*;

    /// The off-target `Cx` refuses everything, which is exactly the state a
    /// user who declined `secrets` puts this widget in. So these run natively,
    /// with no WASM toolchain — which is the point of the stub existing.

    #[test]
    fn a_widget_that_cannot_reach_the_keychain_still_starts() {
        let mut widget = Vault::default();
        assert!(widget.start(&Cx::new()).is_ok());
        assert!(!widget.stored);
        assert!(widget.trouble.is_some(), "and it says why");
    }

    #[test]
    fn a_refused_store_leaves_the_widget_saying_so_rather_than_signed_in() {
        // The failure that matters: a widget that shows "signed in" because it
        // did not look at the answer would send its user to wonder why nothing
        // works.
        let mut widget = Vault::default();
        assert!(widget.on_event(&Event::Clicked(NodeId::named("store")), &Cx::new()));
        assert!(!widget.stored);
        assert!(widget.trouble.is_some());
    }

    #[test]
    fn a_click_on_something_else_changes_nothing_and_says_so() {
        let mut widget = Vault::default();
        assert!(!widget.on_event(&Event::Clicked(NodeId::named("elsewhere")), &Cx::new()));
        assert!(!widget.on_event(&Event::Configured, &Cx::new()));
    }

    #[test]
    fn the_three_denials_read_differently() {
        // One message for all three would send a user to their settings for a
        // bug, or an author to their manifest for a decision the user made.
        let not_granted = explain(&Denied::NotGranted);
        let unavailable = explain(&Denied::Unavailable("no keychain".into()));
        let malformed = explain(&Denied::Malformed("bad key".into()));
        assert_ne!(not_granted, unavailable);
        assert_ne!(unavailable, malformed);
        assert!(malformed.contains("this widget"), "{malformed}");
    }

    #[test]
    fn the_credential_is_never_in_the_view() {
        // The view is composed, hit-tested and handed to an accessibility
        // tree. A secret in it is a secret a screen reader will read out.
        let mut widget = Vault {
            stored: true,
            trouble: None,
        };
        let view = widget.view(&Cx::new());
        let rendered = format!("{view:?}");
        let token = std::str::from_utf8(DEMONSTRATION_TOKEN).expect("utf-8");
        assert!(!rendered.contains(token), "the token reached the view");
    }
}
