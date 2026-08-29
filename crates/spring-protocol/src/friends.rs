//! Friends, and the requests that become them.
//!
//! Every command here names its target as `userName=<name>`, which teiserver
//! parses by splitting on `=` and taking the second field
//! (`spring_in.ex:527`) — so the key is decorative and the name is what
//! matters, but sending it the documented way costs nothing.

use crate::policy::{Area, Envelope};

fn command(verb: &str, user: &str) -> Envelope {
    Envelope::queue(Area::ChannelChat, format!("{verb} userName={user}"))
}

/// Asks for the friend list, answered between `FRIENDLISTBEGIN` and `FRIENDLISTEND`.
pub fn list() -> Envelope {
    Envelope::queue(Area::ChannelChat, "FRIENDLIST")
}

/// Asks for incoming friend requests.
pub fn list_requests() -> Envelope {
    Envelope::queue(Area::ChannelChat, "FRIENDREQUESTLIST")
}

pub fn request(user: &str) -> Envelope {
    command("FRIENDREQUEST", user)
}

pub fn accept(user: &str) -> Envelope {
    command("ACCEPTFRIENDREQUEST", user)
}

pub fn decline(user: &str) -> Envelope {
    command("DECLINEFRIENDREQUEST", user)
}

pub fn remove(user: &str) -> Envelope {
    command("UNFRIEND", user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_names_its_target_the_same_way() {
        assert_eq!(request("alice").line, "FRIENDREQUEST userName=alice");
        assert_eq!(accept("alice").line, "ACCEPTFRIENDREQUEST userName=alice");
        assert_eq!(decline("bob").line, "DECLINEFRIENDREQUEST userName=bob");
        assert_eq!(remove("bob").line, "UNFRIEND userName=bob");
    }

    #[test]
    fn the_two_listings_are_asked_for_without_arguments() {
        assert_eq!(list().line, "FRIENDLIST");
        assert_eq!(list_requests().line, "FRIENDREQUESTLIST");
    }
}
