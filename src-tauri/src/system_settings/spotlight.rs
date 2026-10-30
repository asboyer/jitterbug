//! Spotlight Search Privacy folder exclusions have no public capture/restore API.
//! Volume indexing (`mdutil`) is a different setting and is not substituted here.

pub(super) fn annotate(items: &mut [super::Capability]) {
    if let Some(item) = items.iter_mut().find(|v| v.id == "spotlight_exclusions") {
        item.status = super::CapabilityStatus::Manual;
        item.verification = "Search Privacy folder exclusions are user-managed. No supported exclusion-list getter/setter is implemented; mdutil volume indexing is not this list. No index rebuild or deletion is offered.".into();
    }
}
