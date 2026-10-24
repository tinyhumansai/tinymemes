//! Pure turn predicates hosts share: remix gating and A/B bucketing.

/// Whether a finished reply is worth remixing at all: not the host's
/// `placeholder` (for example an out-of-budget notice) and not blank.
pub fn should_remix(reply: &str, placeholder: &str) -> bool {
    reply != placeholder && !reply.trim().is_empty()
}

/// Stable 0..100 bucket for a thread (FNV-1a, so it does not change between
/// builds or platforms the way `DefaultHasher` may). Changing it would move
/// live threads between experiment arms.
pub fn bucket(thread_id: &str) -> u8 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in thread_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash % 100) as u8
}

#[cfg(test)]
#[path = "ab_tests.rs"]
mod tests;
