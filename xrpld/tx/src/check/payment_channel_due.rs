//! Shared pure due-time rule for payment channels.

/// Narrow facts for deciding whether a payment channel is due.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaymentChannelDueFacts<Time> {
    pub cancel_after: Option<Time>,
    pub expiration: Option<Time>,
    pub close_time: Time,
}

/// Returns `true` when the channel is due under the shared reference rule.
pub fn is_payment_channel_due<Time>(facts: PaymentChannelDueFacts<Time>) -> bool
where
    Time: Copy + Ord,
{
    facts
        .cancel_after
        .is_some_and(|cancel_after| facts.close_time >= cancel_after)
        || facts
            .expiration
            .is_some_and(|expiration| facts.close_time >= expiration)
}
