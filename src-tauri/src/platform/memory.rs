// Memory accounting adapted from Stats, Modules/RAM/readers.swift at
// 21f866bb3e8ba606c9c01769f558accbcf1fa27c. Copyright (c) 2019 Serhiy Mytrovtsiy.
// MIT license: see THIRD_PARTY_NOTICES.md. Kernel access is implemented in macos.rs.
use crate::types::MemoryBreakdown;

#[derive(Default)]
pub(super) struct VmPages {
    pub active: u64,
    pub inactive: u64,
    pub speculative: u64,
    pub wired: u64,
    pub compressed: u64,
    pub purgeable: u64,
    pub external: u64,
    pub swap_in: u64,
    pub swap_out: u64,
}

pub(super) fn account(p: VmPages, page_size: u64) -> Option<MemoryBreakdown> {
    if page_size == 0 {
        return None;
    }
    let cached = p.purgeable.checked_add(p.external)?;
    let app = p
        .active
        .checked_add(p.inactive)?
        .checked_add(p.speculative)?
        .checked_sub(cached)?;
    let used = app.checked_add(p.wired)?.checked_add(p.compressed)?;
    Some(MemoryBreakdown {
        app_bytes: app.checked_mul(page_size)?,
        wired_bytes: p.wired.checked_mul(page_size)?,
        compressed_bytes: p.compressed.checked_mul(page_size)?,
        cached_bytes: cached.checked_mul(page_size)?,
        used_bytes: used.checked_mul(page_size)?,
        swap_in_total_bytes: p.swap_in.checked_mul(page_size)?,
        swap_out_total_bytes: p.swap_out.checked_mul(page_size)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_for_reclaimable_pages_and_actual_page_size() {
        for size in [4096, 16384] {
            let value = account(
                VmPages {
                    active: 100,
                    inactive: 30,
                    speculative: 10,
                    wired: 20,
                    compressed: 15,
                    purgeable: 5,
                    external: 25,
                    swap_in: 7,
                    swap_out: 9,
                },
                size,
            )
            .unwrap();
            assert_eq!(value.app_bytes, 110 * size);
            assert_eq!(value.used_bytes, 145 * size);
            assert_eq!(
                value.used_bytes,
                value.app_bytes + value.wired_bytes + value.compressed_bytes
            );
            assert_eq!(value.cached_bytes, 30 * size);
            assert_eq!(value.swap_out_total_bytes, 9 * size);
        }
    }

    #[test]
    fn unavailable_or_inconsistent_counters_are_not_reported_as_zero_memory() {
        assert!(account(VmPages::default(), 0).is_none());
        assert!(account(
            VmPages {
                external: 1,
                ..Default::default()
            },
            4096
        )
        .is_none());
        assert!(account(
            VmPages {
                swap_in: u64::MAX,
                ..Default::default()
            },
            4096
        )
        .is_none());
    }
}
