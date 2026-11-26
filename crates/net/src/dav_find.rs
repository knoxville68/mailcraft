// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding where an account's calendars are (#209): the search of `mailcraft_dav::discovery`,
//! run over the real wire for the window's Calendar Server form. The rules of where the password
//! may go are the machine's; this only performs its requests.
//!
//! What the owner picks: a provider (iCloud, Fastmail), whose own servers are then the only place
//! asked, or the address of a server they typed, which is then the only place asked; with
//! neither, the places the account's address leads to.

use mailcraft_dav::discovery::{Discovery, Outcome};
use mailcraft_dav::http::{HttpExchange, Secret};
use mailcraft_dav::request::Kind;

use crate::dav::Wire;

/// Where to look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place<'a> {
    /// A known provider's own servers: `icloud` or `fastmail`.
    Provider(&'a str),
    /// The `https://` address the owner typed.
    Typed(&'a str),
    /// The places the account's address leads to.
    Address,
}

/// What a search came to: the calendar homes found (absolute `https://` addresses, at least
/// one), or a sentence for the owner saying why none was. Its `Debug` shows no address.
#[derive(Clone, PartialEq, Eq)]
pub enum Found {
    Homes(Vec<String>),
    Failed(String),
}

impl std::fmt::Debug for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Found::Homes(homes) => write!(f, "Homes({})", homes.len()),
            Found::Failed(why) => write!(f, "Failed({why})"),
        }
    }
}

/// [`find_with`] over the real wire, trusting the certificates the system does.
pub fn find(email: &str, user: &str, password: &str, place: Place<'_>) -> Found {
    find_with(&Wire::new(""), email, user, password, place)
}

/// Searches for the calendar homes of the account `email`, signing in as `user` with `password`
/// only where the search allows. A host the search would need the owner's yes for is declined
/// (the owner can type the server's address instead).
pub fn find_with<E: HttpExchange>(exchange: &E, email: &str, user: &str, password: &str, place: Place<'_>) -> Found {
    let Ok(secret) = Secret::basic(user.trim(), password) else {
        return Found::Failed("A user name with a colon in it can't sign in. Check the user name.".into());
    };
    // A provider's servers are found by its domain: the account's own address may be at another
    // (an Apple ID can be any address), so the search is told the provider's. Only the domain is
    // used, and with a provider no other place is asked.
    let searched = match place {
        Place::Provider("icloud") => "owner@icloud.com".to_string(),
        Place::Provider("fastmail") => "owner@fastmail.com".to_string(),
        Place::Provider(_) => return Found::Failed("That provider isn't one this program knows. Type the server's address.".into()),
        Place::Typed(_) | Place::Address => email.trim().to_string(),
    };
    let Ok(mut search) = Discovery::new(&searched, secret, true, false) else {
        return Found::Failed("That is not an e-mail address a server can be looked for from; type the server's address instead.".into());
    };
    // The provider the owner picked is asked at its own address as if they had typed it: its
    // server (iCloud's) asks for the password without saying first that it keeps calendars, which
    // only a server the owner named may do.
    let typed = match place {
        Place::Provider(_) => mailcraft_model::provider::dav_by_domain(searched.rsplit('@').next().unwrap_or_default()).map(|p| p.caldav),
        Place::Typed(url) => Some(url),
        Place::Address => None,
    };
    if let Some(url) = typed {
        search = match search.typed(Kind::Calendar, url) {
            Ok(search) => search,
            Err(_) => return Found::Failed("That isn't the https:// address of a server.".into()),
        };
    }
    // shortcut: no SRV look-up and every host outside the owner's is declined; enough for a
    // provider or a typed address, upgrade when the search by address alone has to find more.
    match search.run(exchange, |_| None, |_| false) {
        Outcome::Found(found) => match found.calendars() {
            Some(located) if !located.homes().is_empty() => Found::Homes(located.homes().to_vec()),
            _ => Found::Failed("The server did not say where your calendars are. Type the server's address.".into()),
        },
        Outcome::Microsoft => Found::Failed("This is a Microsoft account: its calendars come with the account itself.".into()),
        Outcome::Google => Found::Failed("This is a Google account: its calendars aren't read this way yet.".into()),
        Outcome::Failed(why) => Found::Failed(why.to_string()),
    }
}

#[cfg(test)]
#[path = "dav_find_tests.rs"]
mod tests;
