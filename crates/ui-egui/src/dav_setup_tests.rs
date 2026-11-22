// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Calendar Server form (#209): what the host's search found becomes the account's settings,
//! and the password typed is held for those servers alone and remembered; a search that found
//! nothing changes nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mailcraft_engine::Session;
use mailcraft_host::SecretError;
use mailcraft_model::Id;
use serde_json::json;

use crate::dav_setup::{DavFindDone, DavFindJob, DavForm, DavPlace};
use crate::{MailApp, SecretStore, Services};

const NOW: i64 = 1_791_993_600;
const PASSWORD: &str = "test-only-app-password";

#[derive(Default)]
struct Store(Mutex<HashMap<String, String>>);

impl SecretStore for Store {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().ok()?.get(key).cloned()
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        self.0.lock().map_err(|_| SecretError::Refused)?.insert(key.into(), secret.into());
        Ok(())
    }
    fn delete(&self, key: &str) {
        if let Ok(mut held) = self.0.lock() {
            held.remove(key);
        }
    }
}

/// A window whose host's search answers `found`, noting the place each job asked for.
fn app(found: Result<Vec<String>, String>) -> (MailApp, Id, Arc<Store>, Arc<Mutex<Vec<DavPlace>>>) {
    let store = Arc::new(Store::default());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let noted = asked.clone();
    let services = Services {
        secrets: Some(store.clone()),
        dav_find: Some(Box::new(move |job: DavFindJob| {
            noted.lock().unwrap().push(job.place);
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(DavFindDone { found: found.clone() }).unwrap();
            rx
        })),
        ..Default::default()
    };
    let mut a = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), services);
    a.autosave = false;
    a.session.dav_enabled = true;
    a.ui.remember_sign_in = true;
    let id = a.execute("account.add", json!({"email": "ada@icloud.com", "incoming": {"host": "imap.mail.me.com"}})).unwrap()["id"].as_u64().unwrap();
    (a, id, store, asked)
}

fn form(a: &MailApp, id: Id) -> DavForm {
    let mut f = DavForm::new(a.session.mailbox.account(id).unwrap());
    f.password = PASSWORD.into();
    f
}

#[test]
fn what_was_found_becomes_the_accounts_server_and_its_password_is_kept_for_it_alone() {
    let homes = vec!["https://p42-caldav.icloud.com/1234/calendars/".to_string()];
    let (mut a, id, store, asked) = app(Ok(homes.clone()));
    let f = form(&a, id);
    assert_eq!(f.place, DavPlace::ICloud, "an iCloud mail account starts on iCloud");
    a.begin_dav_find(&f).unwrap();
    a.dav_find_tick();
    assert_eq!(a.dav_found, Some(Ok(())));
    assert_eq!(*asked.lock().unwrap(), [DavPlace::ICloud]);
    let server = a.session.mailbox.account(id).unwrap().dav.clone().expect("the account has a calendar server");
    assert_eq!(server.home_origins(), ["https://p42-caldav.icloud.com"]);
    assert_eq!(a.dav_password(id, "https://p42-caldav.icloud.com").as_deref(), Some(PASSWORD));
    // Remembered under that server's name, and under no other.
    let held = store.0.lock().unwrap().clone();
    assert_eq!(held.values().filter(|v| v.as_str() == PASSWORD).count(), 1, "{:?}", held.keys());
    assert!(a.dav_password(id, "https://dav.example.org").is_none());
}

#[test]
fn a_search_that_found_nothing_changes_nothing_and_says_why() {
    let (mut a, id, store, _) = app(Err("The server did not accept the user name and password.".into()));
    a.begin_dav_find(&form(&a, id)).unwrap();
    a.dav_find_tick();
    assert!(matches!(a.dav_found, Some(Err(ref why)) if why.contains("user name and password")));
    assert!(a.session.mailbox.account(id).unwrap().dav.is_none());
    assert!(store.0.lock().unwrap().is_empty());
    assert!(a.dav_finding.is_none());
}

#[test]
fn the_form_is_offered_only_when_switched_on_and_shows_no_password() {
    let (mut a, id, _, _) = app(Ok(Vec::new()));
    let other =
        a.execute("account.add", json!({"email": "bo@example.org", "incoming": {"host": "imap.example.org"}})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(DavForm::new(a.session.mailbox.account(other).unwrap()).place, DavPlace::Typed);
    let job =
        DavFindJob { email: "ada@icloud.com".into(), user: "ada".into(), password: PASSWORD.into(), place: DavPlace::ICloud, server: String::new() };
    assert!(!format!("{job:?}").contains(PASSWORD));
    a.session.dav_enabled = false;
    let _ = a.run("ui.dialog", json!({"name": "davServer", "account": id}));
    assert!(a.dialog.is_none());
    a.session.dav_enabled = true;
    let _ = a.run("ui.dialog", json!({"name": "davServer", "account": id}));
    assert_eq!(a.dialog.as_ref().map(|d| d.name()), Some("davServer"));
}
