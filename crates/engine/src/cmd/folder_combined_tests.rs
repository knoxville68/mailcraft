// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Favorites' "All Inboxes" and the like: one kind's folder of every account shown as one list
//! (`folder.select` with `combined`, `folder.favoriteCombined`).

use mailcraft_model::{Account, FolderKind, Id, Message};
use serde_json::json;

use crate::{Session, sample};

const NOW: i64 = 1_791_993_600;

/// The sample mailbox with a second account, one message in its Inbox; both Inboxes.
fn two_accounts() -> (Session, Id, Id, Id) {
    let mut s = Session::at(sample::mailbox(NOW, 0), NOW, 0);
    let first = s.mailbox.default_account().unwrap().id;
    let second = s.mailbox.add_account(Account { name: "Second".into(), email: "second@example.org".into(), ..Default::default() });
    let (a, b) = (s.mailbox.folder_of_kind(first, FolderKind::Inbox).unwrap(), s.mailbox.folder_of_kind(second, FolderKind::Inbox).unwrap());
    let m = s.mailbox.add_message(Message { folder: b, account: second, subject: "to the second".into(), ..Default::default() }).unwrap();
    (s, a, b, m)
}

fn listed_folders(s: &Session) -> std::collections::BTreeSet<Id> {
    crate::list::rows(s).iter().filter_map(|r| s.mailbox.message(r.id)).map(|m| m.folder).collect()
}

#[test]
fn all_inboxes_lists_every_accounts_inbox() {
    let (mut s, a, b, m) = two_accounts();
    let r = s.run("folder.select", &json!({"combined": "inbox"})).unwrap();
    assert_eq!(r["combined"], "inbox");
    assert_eq!(r["folders"], json!([a, b]));
    assert_eq!(s.combined(), Some(FolderKind::Inbox));
    let folders = listed_folders(&s);
    assert!(folders.contains(&a) && folders.contains(&b), "{folders:?}");
    assert!(crate::list::rows(&s).iter().any(|r| r.id == m));
}

#[test]
fn picking_one_folder_leaves_the_combined_list() {
    let (mut s, a, b, _) = two_accounts();
    s.run("folder.select", &json!({"combined": "inbox"})).unwrap();
    s.run("folder.select", &json!({"id": b})).unwrap();
    assert_eq!(s.combined(), None);
    assert_eq!(listed_folders(&s).into_iter().collect::<Vec<_>>(), vec![b]);
    // Whatever else changes the folder in view ends it too.
    s.run("folder.select", &json!({"combined": "inbox"})).unwrap();
    s.view.folder = s.mailbox.folder_of_kind(s.mailbox.folder(a).unwrap().account, FolderKind::Sent).unwrap();
    assert_eq!(s.combined(), None);
}

#[test]
fn favorite_combined_toggles_and_keeps_order() {
    let (mut s, ..) = two_accounts();
    s.run("folder.favoriteCombined", &json!({"kind": "inbox"})).unwrap();
    let r = s.run("folder.favoriteCombined", &json!({"kind": "drafts", "value": true})).unwrap();
    assert_eq!(r["combined"], json!(["inbox", "drafts"]));
    let r = s.run("folder.favoriteCombined", &json!({"kind": "inbox"})).unwrap();
    assert_eq!((r["shown"].clone(), r["combined"].clone()), (json!(false), json!(["drafts"])));
    assert_eq!(s.prefs.combined_favorites, vec![FolderKind::Drafts]);
}

#[test]
fn junk_params_are_refused() {
    let (mut s, ..) = two_accounts();
    for bad in [json!("custom"), json!("nope"), json!(7), json!([]), json!({})] {
        assert!(s.run("folder.select", &json!({"combined": bad})).is_err(), "{bad}");
        assert!(s.run("folder.favoriteCombined", &json!({"kind": bad})).is_err(), "{bad}");
    }
    assert!(s.run("folder.favoriteCombined", &json!({})).is_err());
    assert_eq!(s.combined(), None);
}

#[test]
fn mark_all_read_in_all_inboxes_reaches_every_account() {
    let (mut s, a, b, m) = two_accounts();
    s.run("folder.select", &json!({"combined": "inbox"})).unwrap();
    let r = s.run("mail.markAllRead", &json!({})).unwrap();
    assert_eq!(r["folders"], json!([a, b]));
    assert!(s.mailbox.message(m).unwrap().read);
    assert!(s.mailbox.messages_in(a).all(|m| m.read));
    // A folder named outright is still that folder alone.
    s.mailbox.message_mut(m).unwrap().read = false;
    s.run("mail.markAllRead", &json!({"folder": a})).unwrap();
    assert!(!s.mailbox.message(m).unwrap().read);
}

#[test]
fn folder_commands_wait_for_one_folder_in_all_inboxes() {
    let (mut s, a, _, _) = two_accounts();
    s.run("folder.select", &json!({"combined": "inbox"})).unwrap();
    let before = s.mailbox.count(a);
    assert!(s.run("folder.empty", &json!({})).is_err());
    assert!(s.run("folder.rename", &json!({"name": "X"})).is_err());
    assert!(s.run("folder.delete", &json!({})).is_err());
    assert_eq!(s.mailbox.count(a), before);
}

#[test]
fn a_mailbox_shared_with_the_owner_is_not_in_all_inboxes() {
    let (mut s, a, b, _) = two_accounts();
    let first = s.mailbox.default_account().unwrap().id;
    let shared =
        s.mailbox.add_account(Account { name: "Team".into(), email: "team@example.org".into(), shared_of: Some(first), ..Default::default() });
    let team = s.mailbox.folder_of_kind(shared, FolderKind::Inbox).unwrap();
    let r = s.run("folder.select", &json!({"combined": "inbox"})).unwrap();
    assert_eq!(r["folders"], json!([a, b]));
    assert!(!listed_folders(&s).contains(&team));
}

#[test]
fn running_rules_or_exporting_waits_for_one_folder_in_all_inboxes() {
    let (mut s, a, _, _) = two_accounts();
    s.run("folder.select", &json!({"combined": "inbox"})).unwrap();
    for (cmd, named) in [("rules.run", json!({"folder": a})), ("file.exportFolder", json!({"id": a}))] {
        assert!(s.run(cmd, &json!({})).is_err(), "{cmd} with no folder named");
        // (Run Rules Now names its folder as `folder`, which a greyed-out button wouldn't let
        // through: it says why when it is clicked instead.)
        if cmd == "file.exportFolder" {
            assert!(s.registry.get(cmd).is_some_and(|c| (c.enabled)(&s).is_some()), "{cmd} greyed out");
        }
        // A folder named outright is still that folder.
        let r = s.run(cmd, &named);
        assert!(!format!("{r:?}").contains("every account"), "{cmd}: {r:?}");
    }
}

#[test]
fn the_list_kept_for_one_inbox_is_not_shown_for_all_inboxes() {
    let (mut s, a, _, m) = two_accounts();
    s.run("folder.select", &json!({"id": a})).unwrap();
    assert!(!s.rows().iter().any(|r| r.id == m));
    // The same folder stays in view (it is the anchor): the rows must still be built anew.
    s.run("folder.select", &json!({"combined": "inbox"})).unwrap();
    assert!(s.rows().iter().any(|r| r.id == m));
}
