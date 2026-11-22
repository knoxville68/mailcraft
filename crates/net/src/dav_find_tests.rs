// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Calendar Server form's search against the strict stand-in of `mailcraft-dav`, one server
//! for each host a test makes (nothing here is a real account's).
use super::*;
use mailcraft_dav::href::Origin;
use mailcraft_dav::http::{HttpRequest, HttpResponse, TransportError};
use mailcraft_dav::standin::StandIn;
use std::sync::Mutex;

const USER: &str = "owner@example.org";
const PASSWORD: &str = "test-only-not-a-secret";

/// Hosts that answer as stand-ins do; every request is noted by origin and whether it signed in.
struct World {
    servers: Vec<StandIn>,
    log: Mutex<Vec<(String, bool)>>,
}

impl World {
    fn new(origins: &[&str]) -> World {
        let servers = origins
            .iter()
            .map(|origin| {
                let s = StandIn::new(origin, USER, PASSWORD).unwrap();
                s.add_calendar("home", "Home", false);
                s
            })
            .collect();
        World { servers, log: Mutex::new(Vec::new()) }
    }

    fn origins(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().map(|(o, _)| o.clone()).collect()
    }
}

impl HttpExchange for World {
    fn exchange(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let origin = Origin::parse(request.url()).unwrap();
        self.log.lock().unwrap().push((origin.base(), request.field("authorization").is_some()));
        // A server whose tree starts below its root sends the root there, as iCloud's does.
        if request.url().ends_with(".com/") || request.url().ends_with(".net/") {
            return Ok(HttpResponse::new(301, Vec::new()).with_field("Location", "/dav/"));
        }
        match self.servers.iter().find(|s| *s.origin() == origin) {
            Some(server) => Ok(server.handle(request)),
            None => Err(TransportError::Unreachable),
        }
    }
}

#[test]
fn icloud_is_looked_for_at_apples_server_only_whatever_the_accounts_address() {
    let world = World::new(&["https://caldav.icloud.com"]);
    // An Apple ID can be an address at any domain, even Google's: the provider decides where.
    let found = find_with(&world, "someone@gmail.example", USER, PASSWORD, Place::Provider("icloud"));
    let Found::Homes(homes) = found else { panic!("{found:?}") };
    assert!(homes.iter().all(|h| h.starts_with("https://caldav.icloud.com/")), "{homes:?}");
    assert!(world.origins().iter().all(|o| o == "https://caldav.icloud.com"), "{:?}", world.origins());
}

#[test]
fn a_typed_server_is_the_only_place_asked() {
    let world = World::new(&["https://dav.example.net", "https://example.org"]);
    let found = find_with(&world, USER, USER, PASSWORD, Place::Typed("https://dav.example.net/"));
    assert!(matches!(found, Found::Homes(ref homes) if !homes.is_empty()), "{found:?}");
    assert!(world.origins().iter().all(|o| o == "https://dav.example.net"), "{:?}", world.origins());
}

#[test]
fn a_wrong_password_says_so_and_asks_nothing_more() {
    let world = World::new(&["https://dav.example.net"]);
    let found = find_with(&world, USER, USER, "not-the-test-password", Place::Typed("https://dav.example.net/"));
    let Found::Failed(why) = found else { panic!("{found:?}") };
    assert!(why.contains("user name and password"), "{why}");
    assert!(world.log.lock().unwrap().iter().filter(|(_, signed)| *signed).count() <= 1);
}

#[test]
fn nothing_is_sent_for_an_unknown_provider_a_bad_address_or_a_colon_in_the_name() {
    let world = World::new(&["https://dav.example.net"]);
    for (user, place) in
        [(USER, Place::Provider("someone")), (USER, Place::Typed("ftp://dav.example.net/")), ("a:b", Place::Typed("https://dav.example.net/"))]
    {
        assert!(matches!(find_with(&world, USER, user, PASSWORD, place), Found::Failed(_)));
    }
    assert!(world.origins().is_empty());
    // And the Debug of what was found shows no address.
    assert_eq!(format!("{:?}", Found::Homes(vec!["https://dav.example.net/secret/".into()])), "Homes(1)");
}
