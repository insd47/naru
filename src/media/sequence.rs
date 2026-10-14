//! 16-bit counters the server wraps around: RTP sequence numbers and DONL.

/// Maps a 16-bit counter to the extended value closest to `reference`, the highest one seen so far.
pub fn extend(value: u16, reference: Option<u64>) -> u64 {
    let Some(reference) = reference else {
        return (1 << 32) + u64::from(value);
    };

    let candidate = (reference & !0xffff) | u64::from(value);

    [candidate - 0x1_0000, candidate, candidate + 0x1_0000]
        .into_iter()
        .min_by_key(|candidate| candidate.abs_diff(reference))
        .expect("there is a candidate")
}
