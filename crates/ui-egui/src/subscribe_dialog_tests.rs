// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Subscribe to Internet Calendar: the address has the keyboard when the dialog opens, and the
//! name can be typed into once it is clicked (the address used to take the keyboard back at once).

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::dialogs::Dialog;
use crate::people_view::testing;

#[test]
fn the_name_can_be_typed_into_after_the_address() {
    let mut h = testing::window_on(mailcraft_engine::sample::mailbox(testing::now(), 0), "view.calendar");
    h.state_mut().run("ui.dialog", json!({"name": "subscribe"})).unwrap();
    h.run_steps(3);
    h.event(Event::Text("webcal://calendar.example/f1.ics".into()));
    h.run_steps(3);
    let name = h.query_all_by_role(Role::TextInput).nth(1).expect("the name field").rect().center();
    h.event(Event::PointerMoved(name));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: name, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    h.event(Event::Text("F1".into()));
    h.run_steps(3);
    match &h.state().dialog {
        Some(Dialog::Subscribe { address, name, .. }) => {
            assert_eq!(address, "webcal://calendar.example/f1.ics");
            assert_eq!(name, "F1", "the name took what was typed");
        }
        _ => panic!("the dialog is open"),
    }
}
