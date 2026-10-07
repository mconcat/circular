use engine::peer_bridges::transcript::TRANSCRIPT_LINE_BODY;

use engine::peer_bridges::transcript::TRANSCRIPT_LINE_PATH;

use engine::peer_bridges::transcript::TRANSCRIPT_LINE_OFFSET;

pub const ALL: &[(&str, &str)] = &[
    ("subscription target", "actor.events"),
    ("subscription target", "authoring-commits"),
    ("subscription target", "display.frames"),
    ("subscription target", "edge.depths"),
    ("subscription target", "records"),
    ("transcript line field", TRANSCRIPT_LINE_BODY),
    ("transcript line field", TRANSCRIPT_LINE_PATH),
    ("transcript line field", TRANSCRIPT_LINE_OFFSET),
];

#[cfg(test)]
mod tests {
    #[test]
    fn subscription_dispatch_matches_every_listed_subscription_target() {
        let mut listed = super::ALL
            .iter()
            .filter(|(vocabulary, _)| *vocabulary == "subscription target")
            .map(|(_, name)| *name)
            .collect::<Vec<_>>();
        let mut registered = crate::daemon::subscription_catalog::descriptors()
            .iter()
            .map(|descriptor| descriptor.name)
            .collect::<Vec<_>>();
        listed.sort_unstable();
        registered.sort_unstable();
        assert_eq!(
            listed, registered,
            "the full list of subscription registrations differs from the full list of subscription target rows in unregistered"
        );
        for name in listed {
            assert!(
                crate::daemon::subscription_catalog::resolve(name).is_some(),
                "subscription target {name:?} from the full list does not resolve"
            );
        }
        assert_eq!(
            crate::daemon::subscription_catalog::resolve("not-listed"),
            None
        );
    }

    #[test]
    fn no_two_names_collide() {
        let mut names: Vec<&str> = super::ALL.iter().map(|(_, name)| *name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "two built-in names overlap");
    }
}
