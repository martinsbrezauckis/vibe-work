//! The Chrome Control model: three cumulative levels, raising, and the tool table.
use super::*;

#[test]
fn basic_is_the_default_and_each_level_includes_the_ones_below() {
    let basic = ChromeSettings::default();
    assert_eq!(basic.level, Level::Browse);
    assert!(basic.allows(Group::Read) && basic.allows(Group::Navigate));
    assert!(!basic.allows(Group::Forms) && !basic.allows(Group::Send));
    let advanced = ChromeSettings { level: Level::Fill };
    assert!(advanced.allows(Group::Forms) && !advanced.allows(Group::Send));
    let full = ChromeSettings { level: Level::Act };
    for group in [Group::Send, Group::SignIn, Group::SiteData, Group::Scripts] {
        assert!(full.allows(group), "{group:?}");
    }
    // No level includes Bookmarks and History: they ask at every level.
    for level in Level::ALL {
        let at = ChromeSettings { level };
        assert!(
            !at.allows(Group::Bookmarks) && !at.allows(Group::History),
            "{level:?}"
        );
    }
}

#[test]
fn raising_lists_what_the_new_level_adds_and_lowering_adds_nothing() {
    let basic = ChromeSettings::default();
    let full = ChromeSettings { level: Level::Act };
    assert!(basic.raises(full) && !full.raises(basic));
    assert_eq!(
        basic.added(full),
        vec![
            Group::Forms,
            Group::Send,
            Group::SignIn,
            Group::SiteData,
            Group::Scripts
        ]
    );
    assert_eq!(full.added(basic), Vec::<Group>::new());
}

#[test]
fn every_group_has_its_own_ask_kind_and_wire_id() {
    let mut kinds: Vec<_> = Group::ALL.iter().map(|g| g.ask_kind()).collect();
    kinds.dedup();
    assert_eq!(kinds.len(), Group::ALL.len());
    for group in Group::ALL {
        assert_eq!(Group::from_id(group.id()), Some(group));
        assert_eq!(
            serde_json::to_value(group).unwrap(),
            serde_json::json!(group.id())
        );
    }
    assert_eq!(
        serde_json::to_value(ChromeSettings::default()).unwrap(),
        serde_json::json!({"level": "browse"})
    );
}
