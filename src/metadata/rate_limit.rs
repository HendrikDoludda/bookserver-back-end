//! One provider's pace: how far apart its requests go out, and the wait it asked for.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::time::{sleep_until, Instant};

use crate::scanner::lock;

/// A provider's `Retry-After` is followed up to this, so a wrong value can't silence it for days.
const LONGEST_DEFER: Duration = Duration::from_secs(60 * 60);

/// Hands out a provider's turns: each at least `interval` after the one before, and none before
/// the time the provider last asked to wait for. Clones share their turns.
#[derive(Debug, Clone)]
pub struct RateLimit {
    interval: Duration,
    next: Arc<Mutex<Instant>>,
}

impl RateLimit {
    /// The first turn is free at once.
    pub fn new(interval: Duration) -> Self {
        RateLimit {
            interval,
            next: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Takes the next turn and waits for it. When it is more than `max_wait` away, answers how far
    /// instead, and takes none; `None` waits as long as it takes.
    pub async fn take_turn(&self, max_wait: Option<Duration>) -> Result<(), Duration> {
        let turn = {
            let mut next = lock(&self.next);
            let now = Instant::now();
            let turn = (*next).max(now);
            let wait = turn - now;
            if max_wait.is_some_and(|max_wait| wait > max_wait) {
                return Err(wait);
            }
            *next = turn + self.interval;
            turn
        };
        sleep_until(turn).await;
        Ok(())
    }

    /// The provider asked to wait `wait`: no turn starts before then. Answers the wait followed,
    /// which is at most an hour.
    pub fn defer(&self, wait: Duration) -> Duration {
        let wait = wait.min(LONGEST_DEFER);
        let until = Instant::now() + wait;
        let mut next = lock(&self.next);
        *next = (*next).max(until);
        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_SECONDS: Duration = Duration::from_secs(2);

    #[tokio::test(start_paused = true)]
    async fn turns_go_out_an_interval_apart_and_the_first_at_once() {
        let rate = RateLimit::new(TWO_SECONDS);
        let start = Instant::now();

        let mut taken = Vec::new();
        for _ in 0..3 {
            rate.take_turn(None).await.unwrap();
            taken.push(start.elapsed());
        }

        assert_eq!(taken, [Duration::ZERO, TWO_SECONDS, Duration::from_secs(4)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_pause_longer_than_the_interval_leaves_the_next_turn_free() {
        let rate = RateLimit::new(TWO_SECONDS);
        rate.take_turn(None).await.unwrap();
        tokio::time::advance(Duration::from_secs(5)).await;
        let start = Instant::now();

        rate.take_turn(Some(Duration::ZERO)).await.unwrap();

        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_who_would_wait_too_long_is_told_how_long_and_takes_no_turn() {
        let rate = RateLimit::new(TWO_SECONDS);
        rate.defer(Duration::from_secs(30));

        assert_eq!(
            rate.take_turn(Some(Duration::from_secs(10))).await,
            Err(Duration::from_secs(30))
        );
        tokio::time::advance(Duration::from_secs(25)).await;
        assert_eq!(
            rate.take_turn(Some(Duration::from_secs(10))).await,
            Ok(()),
            "5 s left, and the refused caller took no turn"
        );
        assert_eq!(
            rate.take_turn(Some(Duration::ZERO)).await,
            Err(TWO_SECONDS),
            "the turn after that one"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_without_a_limit_waits_as_long_as_the_provider_asked() {
        let rate = RateLimit::new(TWO_SECONDS);
        rate.defer(Duration::from_secs(60));
        let start = Instant::now();

        rate.take_turn(None).await.unwrap();

        assert_eq!(start.elapsed(), Duration::from_secs(60));
    }

    #[tokio::test(start_paused = true)]
    async fn a_shorter_wait_never_brings_the_next_turn_forward_and_a_huge_one_is_capped() {
        let rate = RateLimit::new(TWO_SECONDS);
        rate.defer(Duration::from_secs(30));
        rate.defer(Duration::from_secs(5));
        assert_eq!(
            rate.take_turn(Some(Duration::ZERO)).await,
            Err(Duration::from_secs(30))
        );

        assert_eq!(rate.defer(Duration::MAX), LONGEST_DEFER);
        assert_eq!(
            rate.take_turn(Some(Duration::ZERO)).await,
            Err(LONGEST_DEFER)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn clones_share_their_turns() {
        let rate = RateLimit::new(TWO_SECONDS);
        let clone = rate.clone();

        rate.take_turn(None).await.unwrap();

        assert_eq!(
            clone.take_turn(Some(Duration::ZERO)).await,
            Err(TWO_SECONDS)
        );
    }
}
