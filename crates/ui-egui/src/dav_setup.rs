// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Calendar Server form (#209, behind `MAILCRAFT_DAV=1`): where an account's calendars are,
//! found by the host's search ([`Services::dav_find`](crate::Services::dav_find)) from a provider
//! the owner picks (iCloud, Fastmail) or a server address they type, signing in with the user name
//! and password they give (iCloud's app-specific password). What was found becomes the account's
//! settings through `account.davSet`; the password is held for the session by origin and, when
//! the owner lets MailCraft remember sign-ins, kept in the system's credential store. Neither is
//! ever in a setting, a log or an error.

use std::sync::mpsc::Receiver;

use mailcraft_model::Id;
use serde_json::json;

use crate::MailApp;
use crate::dialogs::{buttons, window};
use crate::theme::Tokens;

/// Where the host's search looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DavPlace {
    ICloud,
    Fastmail,
    /// The server address the owner typed.
    Typed,
}

/// What the host is to search for. Its `Debug` shows nothing of the password.
pub struct DavFindJob {
    pub email: String,
    pub user: String,
    pub password: String,
    pub place: DavPlace,
    pub server: String,
}

impl std::fmt::Debug for DavFindJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DavFindJob").field("place", &self.place).finish()
    }
}

/// What the search found: the calendar homes, or why not in a sentence for the owner.
pub struct DavFindDone {
    pub found: Result<Vec<String>, String>,
}

/// A search under way: the account, the user name and password it signs in with, and its answer.
pub(crate) struct Finding {
    account: Id,
    user: String,
    password: String,
    done: Receiver<DavFindDone>,
}

/// The form's fields.
#[derive(Clone, PartialEq)]
pub struct DavForm {
    pub account: Id,
    pub place: DavPlace,
    pub server: String,
    pub user: String,
    pub password: String,
    pub error: String,
    pub busy: bool,
}

impl DavForm {
    /// The form for `account`: iCloud picked for an iCloud mail account, Fastmail for Fastmail's,
    /// the account's address as the user name.
    pub(crate) fn new(account: &mailcraft_model::Account) -> DavForm {
        let domain = account.email.rsplit('@').next().unwrap_or_default().to_ascii_lowercase();
        let imap = account.incoming.as_ref().map(|s| s.host.to_ascii_lowercase()).unwrap_or_default();
        let place = if matches!(domain.as_str(), "icloud.com" | "me.com" | "mac.com") || imap == "imap.mail.me.com" {
            DavPlace::ICloud
        } else if matches!(domain.as_str(), "fastmail.com" | "fastmail.fm") || imap == "imap.fastmail.com" {
            DavPlace::Fastmail
        } else {
            DavPlace::Typed
        };
        DavForm {
            account: account.id,
            place,
            server: String::new(),
            user: account.email.clone(),
            password: String::new(),
            error: String::new(),
            busy: false,
        }
    }
}

impl MailApp {
    /// Starts the search the form asks for. Refused when another one is under way or the host
    /// can't search.
    pub(crate) fn begin_dav_find(&mut self, form: &DavForm) -> Result<(), String> {
        if self.dav_finding.is_some() {
            return Err(tl!("A search for a calendar server is already under way.").to_string());
        }
        let Some(start) = &self.services.dav_find else { return Err(tl!("This version can't look for calendar servers.").to_string()) };
        let email = self.session.mailbox.account(form.account).map(|a| a.email.clone()).unwrap_or_default();
        let job = DavFindJob {
            email,
            user: form.user.trim().to_string(),
            password: form.password.clone(),
            place: form.place,
            server: form.server.trim().to_string(),
        };
        let done = start(job);
        self.dav_finding = Some(Finding { account: form.account, user: form.user.trim().to_string(), password: form.password.clone(), done });
        Ok(())
    }

    /// Once a frame: takes in a search that came back. Found, the account's settings are set and
    /// the password is held (and remembered); the form is told how it went.
    pub(crate) fn dav_find_tick(&mut self) {
        let Some(finding) = &self.dav_finding else { return };
        let done = match finding.done.try_recv() {
            Ok(done) => done,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                DavFindDone { found: Err(tl!("The search stopped before it found anything.").to_string()) }
            }
        };
        let Some(Finding { account, user, password, .. }) = self.dav_finding.take() else { return };
        self.dav_found = Some(match done.found {
            Ok(homes) => self.take_dav_homes(account, &user, &password, homes),
            Err(why) => Err(why),
        });
    }

    fn take_dav_homes(&mut self, account: Id, user: &str, password: &str, homes: Vec<String>) -> Result<(), String> {
        let homes: Vec<String> = homes.into_iter().take(8).collect();
        self.execute("account.davSet", json!({"id": account, "calendarHomes": homes, "username": user, "separatePassword": true}))?;
        let mut origins: Vec<String> = homes.iter().filter_map(|h| mailcraft_model::origin_of(h)).collect();
        origins.dedup();
        for origin in origins {
            if self.hold_dav_password(account, &origin, password) {
                self.remember_dav(account, &origin, password);
            }
        }
        Ok(())
    }

    /// Keeps the password of `origin` in the system's credential store, as [`MailApp::remember`]
    /// does a mail password: only when the owner lets MailCraft remember sign-ins.
    fn remember_dav(&mut self, account: Id, origin: &str, password: &str) {
        if !self.ui.remember_sign_in || password.is_empty() || self.data_removed || self.unconfirmed {
            return;
        }
        let Some(key) = self.session.mailbox.account(account).and_then(|a| Self::dav_secret_key(a, origin)) else { return };
        if self.dav_forget_pending(&key) {
            return;
        }
        let Some(store) = &self.services.secrets else { return };
        match store.set(&key, password) {
            Ok(()) => {
                self.secret_cache.insert(key.clone(), password.to_string());
                self.index_secret_key(key);
            }
            Err(_) => {
                log::warn!("couldn't keep a calendar server's sign-in in the operating system's credential store");
                store.delete(&key);
                self.secret_cache.remove(&key);
            }
        }
    }
}

/// The form. Returns whether it is to close.
pub(crate) fn form(app: &mut MailApp, ctx: &egui::Context, enter: bool, f: &mut DavForm) -> bool {
    let t = Tokens::get(ctx);
    let mut close = false;
    if f.busy && app.dav_finding.is_none() {
        f.busy = false;
        match app.dav_found.take() {
            Some(Ok(())) => return true,
            Some(Err(why)) => f.error = why,
            None => {}
        }
    }
    let name = app.session.mailbox.account(f.account).map(crate::nav::account_label).unwrap_or_default();
    let ready = !f.busy && !f.user.trim().is_empty() && !f.password.is_empty() && (f.place != DavPlace::Typed || !f.server.trim().is_empty());
    let mut submit = false;
    let open = window(ctx, tl!("Calendar Server"), 440.0, |ui| {
        ui.label(crate::i18n::fmt(tl!("Where the calendars of {account} are"), &[("account", &name)]));
        ui.add_enabled_ui(!f.busy, |ui| {
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.place, DavPlace::ICloud, "iCloud");
                ui.radio_value(&mut f.place, DavPlace::Fastmail, "Fastmail");
                ui.radio_value(&mut f.place, DavPlace::Typed, tl!("Another server"));
            });
            if f.place == DavPlace::Typed {
                ui.label(tl!("The server's address"));
                crate::text_menu::field(
                    ui.add(egui::TextEdit::singleline(&mut f.server).hint_text("https://dav.example.org/").desired_width(f32::INFINITY)),
                );
            }
            ui.label(tl!("User name"));
            crate::text_menu::field(ui.add(egui::TextEdit::singleline(&mut f.user).hint_text("name@example.org").desired_width(f32::INFINITY)));
            ui.label(tl!("Password"));
            crate::text_menu::field(ui.add(egui::TextEdit::singleline(&mut f.password).password(true).desired_width(f32::INFINITY)));
        });
        let note = if f.place == DavPlace::ICloud {
            tl!(
                "Your Apple ID and an app-specific password, which you make at account.apple.com under Sign-In and Security. New appointments can be added here; other changes are made in iCloud."
            )
        } else {
            tl!(
                "The password goes only to that server, over an encrypted connection. New appointments can be added here; other changes are made on the server."
            )
        };
        ui.label(egui::RichText::new(note).color(t.text_dim).size(11.0));
        if f.busy {
            ui.label(egui::RichText::new(tl!("Looking for your calendars…")).color(t.text_dim));
        }
        if !f.error.is_empty() {
            ui.label(egui::RichText::new(f.error.as_str()).color(t.red));
        }
        let (ok, cancel) = buttons(ui, tl!("Connect"), ready);
        submit = ready && (ok || enter);
        close |= cancel;
    });
    close |= !open;
    if submit && !close {
        f.error.clear();
        match app.begin_dav_find(f) {
            Ok(()) => f.busy = true,
            Err(why) => f.error = why,
        }
    }
    close
}
