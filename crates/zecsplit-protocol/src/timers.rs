//! Safety timers (PROTOCOL_SPEC §4.4, §5.3; T-035).

/// §4.4-R1 (margin rule): Alice may broadcast `tx_redeem` only while `tip + redeem_margin <
/// h_lock + t1`.
pub fn may_redeem(tip: u32, h_lock: u32, t1: u16, redeem_margin: u16) -> bool {
    u64::from(tip) + u64::from(redeem_margin) < u64::from(h_lock) + u64::from(t1)
}

/// `tx_cancel` (relative lock `t1` on the lock output) can be mined in the next block.
pub fn cancel_valid(tip: u32, h_lock: u32, t1: u16) -> bool {
    u64::from(tip) + 1 >= u64::from(h_lock) + u64::from(t1)
}

/// `tx_punish` (relative lock `t2` on the cancel output) can be mined in the next block.
pub fn punish_valid(tip: u32, h_cancel: u32, t2: u16) -> bool {
    u64::from(tip) + 1 >= u64::from(h_cancel) + u64::from(t2)
}

/// §4.4-R3 (and §5.3-R5 for the refund): fee-bump with CPFP once a broadcast transaction has
/// stayed unconfirmed for `blocks` blocks.
pub fn needs_cpfp(tip: u32, broadcast_at_tip: u32, blocks: u32) -> bool {
    u64::from(tip) >= u64::from(broadcast_at_tip) + u64::from(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T1: u16 = 20;
    const T2: u16 = 20;
    const MARGIN: u16 = 5;

    #[test]
    fn spec_4_4_r1_refuses_redeem_inside_margin() {
        let h_lock = 100;
        assert!(may_redeem(114, h_lock, T1, MARGIN));
        assert!(!may_redeem(115, h_lock, T1, MARGIN));
        assert!(!may_redeem(119, h_lock, T1, MARGIN));
        assert!(!may_redeem(500, h_lock, T1, MARGIN));
    }

    #[test]
    fn spec_4_4_r1_margin_never_overlaps_cancel_validity() {
        for tip in 0..300 {
            if may_redeem(tip, 100, T1, MARGIN) {
                assert!(!cancel_valid(tip, 100, T1), "tip {tip}");
            }
        }
    }

    #[test]
    fn spec_5_3_r1_cancel_becomes_valid_at_t1() {
        assert!(!cancel_valid(118, 100, T1));
        assert!(cancel_valid(119, 100, T1));
        assert!(
            cancel_valid(121, 100, T1),
            "t1 + 1 and later: offline catch-up"
        );
    }

    #[test]
    fn spec_5_3_r4_punish_after_t2() {
        assert!(!punish_valid(138, 120, T2));
        assert!(punish_valid(139, 120, T2));
    }

    #[test]
    fn spec_4_4_r3_cpfp_after_three_blocks() {
        assert!(!needs_cpfp(102, 100, 3));
        assert!(needs_cpfp(103, 100, 3));
    }

    #[test]
    fn heights_near_u32_max_do_not_overflow() {
        assert!(!may_redeem(u32::MAX, u32::MAX, u16::MAX, u16::MAX));
        assert!(cancel_valid(u32::MAX, u32::MAX, 1));
        assert!(!cancel_valid(u32::MAX - 2, u32::MAX, 2));
        assert!(!punish_valid(u32::MAX - 2, u32::MAX, 2));
        assert!(needs_cpfp(u32::MAX, u32::MAX - 3, 3));
    }
}
