//! The match engine: rolling per-speaker windows, Level-1 card selection
//! with hysteresis, Level-2 bullet coverage as independent booleans.
//!
//! Pure state machine — no DB, no embedding, no Tauri. The ticker feeds it
//! ranked candidates and bullet similarities; it answers with decisions.
//! This keeps the thresholds testable (they ARE the product).
//!
//! Scoring: candidates carry the hybrid RRF fused score normalized to
//! [0, 1] (1.0 = rank 1 in both legs). Bullet coverage uses raw cosine.

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

// ── Rolling windows ─────────────────────────────────────────────────

pub const WINDOW_MS: u64 = 10_000;

#[derive(Default)]
pub struct RollingWindow {
    entries: VecDeque<(u64, String)>, // (ts_ms since session start, text)
}

impl RollingWindow {
    pub fn push(&mut self, ts_ms: u64, text: String) {
        self.entries.push_back((ts_ms, text));
        self.trim(ts_ms);
    }
    fn trim(&mut self, now_ms: u64) {
        while let Some((ts, _)) = self.entries.front() {
            if now_ms.saturating_sub(*ts) > WINDOW_MS {
                self.entries.pop_front();
            } else {
                break;
            }
        }
    }
    pub fn text(&self) -> String {
        self.entries
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

// ── Thresholds (live-tunable; the debug panel owns these) ───────────

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize)]
pub struct Thresholds {
    /// Minimum normalized fused score for any card to be shown at all.
    pub theta_card: f64,
    /// A challenger must beat the current card's score by this much.
    pub theta_margin: f64,
    /// …for this many consecutive ticks…
    pub stable_ticks: u32,
    /// …and no jump may happen within this cooldown after the last one.
    pub cooldown_ms: u64,
    /// Bullet cosine above this marks the bullet covered.
    pub theta_bullet: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            theta_card: 0.30,
            theta_margin: 0.15,
            stable_ticks: 2,
            cooldown_ms: 1500,
            // 0.45 produced a false "covered" on an unspoken bullet in the
            // Phase-3 transcript run; 0.55 was clean on the same material.
            theta_bullet: 0.55,
        }
    }
}

// ── Level 1: card selection with hysteresis ─────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub card_id: String,
    /// Normalized fused score in [0, 1].
    pub score: f64,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Stay,
    Jump { card_id: String },
    /// Best candidate below θ_card — Mode 2 territory (Phase 5).
    NoConfidence,
}

pub struct MatchEngine {
    pub thresholds: Thresholds,
    current: Option<String>,
    challenger: Option<(String, u32)>,
    last_jump: Option<Instant>,
    /// card_id → covered flags, sticky for the session run.
    coverage: HashMap<String, Vec<bool>>,
}

impl MatchEngine {
    pub fn new(thresholds: Thresholds) -> Self {
        Self {
            thresholds,
            current: None,
            challenger: None,
            last_jump: None,
            coverage: HashMap::new(),
        }
    }

    pub fn current_card(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn challenger_streak(&self) -> Option<(&str, u32)> {
        self.challenger.as_ref().map(|(id, n)| (id.as_str(), *n))
    }

    /// True while a challenger is accumulating its streak — the ticker must
    /// keep evaluating even without new transcript, or a jump pending at the
    /// end of an utterance freezes forever (measured in Phase-3 tuning).
    pub fn has_pending_challenger(&self) -> bool {
        self.challenger.is_some()
    }

    pub fn reset(&mut self) {
        self.current = None;
        self.challenger = None;
        self.last_jump = None;
        self.coverage.clear();
    }

    pub fn decide(&mut self, ranked: &[Candidate], now: Instant) -> Decision {
        let Some(best) = ranked.first() else {
            self.challenger = None;
            return Decision::Stay;
        };
        if best.score < self.thresholds.theta_card {
            self.challenger = None;
            return Decision::NoConfidence;
        }

        // No card yet: jump immediately — there is nothing to protect.
        let Some(current_id) = self.current.clone() else {
            self.jump(best.card_id.clone(), now);
            return Decision::Jump {
                card_id: best.card_id.clone(),
            };
        };

        if best.card_id == current_id {
            self.challenger = None;
            return Decision::Stay;
        }

        // Challenger logic: must lead the CURRENT card's present score by
        // the margin, hold for N ticks, and respect the cooldown.
        let current_score = ranked
            .iter()
            .find(|c| c.card_id == current_id)
            .map(|c| c.score)
            .unwrap_or(0.0);
        let leads = best.score - current_score >= self.thresholds.theta_margin;

        if !leads {
            self.challenger = None;
            return Decision::Stay;
        }

        let streak = match &self.challenger {
            Some((id, n)) if *id == best.card_id => n + 1,
            _ => 1,
        };
        self.challenger = Some((best.card_id.clone(), streak));

        let cooled = self
            .last_jump
            .map(|t| now.duration_since(t) >= Duration::from_millis(self.thresholds.cooldown_ms))
            .unwrap_or(true);

        if streak >= self.thresholds.stable_ticks && cooled {
            self.jump(best.card_id.clone(), now);
            return Decision::Jump {
                card_id: best.card_id.clone(),
            };
        }
        Decision::Stay
    }

    fn jump(&mut self, card_id: String, now: Instant) {
        self.current = Some(card_id);
        self.challenger = None;
        self.last_jump = Some(now);
    }

    // ── Level 2: bullet coverage — matching, not sequencing ─────────

    /// `bullet_sims`: cosine of the ME window against each bullet of the
    /// active card, in display order. Returns the updated flags if anything
    /// newly covered.
    pub fn update_coverage(
        &mut self,
        card_id: &str,
        bullet_sims: &[f64],
    ) -> Option<Vec<bool>> {
        let flags = self
            .coverage
            .entry(card_id.to_string())
            .or_insert_with(|| vec![false; bullet_sims.len()]);
        if flags.len() != bullet_sims.len() {
            *flags = vec![false; bullet_sims.len()];
        }
        let mut changed = false;
        for (flag, sim) in flags.iter_mut().zip(bullet_sims) {
            if !*flag && *sim >= self.thresholds.theta_bullet {
                *flag = true;
                changed = true;
            }
        }
        if changed {
            Some(flags.clone())
        } else {
            None
        }
    }

    pub fn coverage_of(&self, card_id: &str) -> Option<&Vec<bool>> {
        self.coverage.get(card_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, score: f64) -> Candidate {
        Candidate {
            card_id: id.into(),
            score,
        }
    }

    #[test]
    fn first_confident_candidate_jumps_immediately() {
        let mut e = MatchEngine::new(Thresholds::default());
        let now = Instant::now();
        assert_eq!(
            e.decide(&[c("a", 0.6)], now),
            Decision::Jump { card_id: "a".into() }
        );
        assert_eq!(e.current_card(), Some("a"));
    }

    #[test]
    fn below_theta_card_is_no_confidence() {
        let mut e = MatchEngine::new(Thresholds::default());
        assert_eq!(e.decide(&[c("a", 0.1)], Instant::now()), Decision::NoConfidence);
        assert_eq!(e.current_card(), None);
    }

    #[test]
    fn challenger_needs_margin_streak_and_cooldown() {
        let t = Thresholds {
            cooldown_ms: 0,
            ..Default::default()
        };
        let mut e = MatchEngine::new(t);
        let now = Instant::now();
        e.decide(&[c("a", 0.8)], now);

        // Margin too small → no streak accrues.
        assert_eq!(
            e.decide(&[c("b", 0.85), c("a", 0.8)], now),
            Decision::Stay
        );
        // Leads by margin: tick 1 of 2 → still stay.
        assert_eq!(e.decide(&[c("b", 0.99), c("a", 0.5)], now), Decision::Stay);
        // Tick 2 → jump.
        assert_eq!(
            e.decide(&[c("b", 0.99), c("a", 0.5)], now),
            Decision::Jump { card_id: "b".into() }
        );
    }

    #[test]
    fn cooldown_blocks_pingpong() {
        let mut e = MatchEngine::new(Thresholds {
            stable_ticks: 1,
            cooldown_ms: 60_000,
            ..Default::default()
        });
        let now = Instant::now();
        e.decide(&[c("a", 0.8)], now);
        // Challenger leads massively but cooldown has not elapsed.
        assert_eq!(
            e.decide(&[c("b", 0.99), c("a", 0.4)], now),
            Decision::Stay
        );
    }

    #[test]
    fn streak_resets_when_challenger_changes() {
        let mut e = MatchEngine::new(Thresholds {
            cooldown_ms: 0,
            stable_ticks: 2,
            ..Default::default()
        });
        let now = Instant::now();
        e.decide(&[c("a", 0.8)], now);
        e.decide(&[c("b", 0.99), c("a", 0.5)], now); // b streak 1
        e.decide(&[c("x", 0.99), c("a", 0.5)], now); // x streak 1 (b reset)
        assert_eq!(
            e.decide(&[c("b", 0.99), c("a", 0.5)], now),
            Decision::Stay,
            "b must restart its streak"
        );
    }

    #[test]
    fn coverage_is_sticky_and_out_of_order() {
        let mut e = MatchEngine::new(Thresholds::default());
        // Bullet 3 covered first (out of order).
        let f = e.update_coverage("a", &[0.1, 0.1, 0.9, 0.1]).unwrap();
        assert_eq!(f, vec![false, false, true, false]);
        // Bullet 1 next; bullet 3 stays covered though its sim dropped.
        let f = e.update_coverage("a", &[0.9, 0.1, 0.0, 0.1]).unwrap();
        assert_eq!(f, vec![true, false, true, false]);
        // Nothing new → None.
        assert!(e.update_coverage("a", &[0.9, 0.1, 0.0, 0.1]).is_none());
    }

    #[test]
    fn rolling_window_trims_by_time() {
        let mut w = RollingWindow::default();
        w.push(0, "old".into());
        w.push(5_000, "mid".into());
        w.push(16_000, "new".into()); // 0 and 5000 now out of the 10 s window
        assert_eq!(w.text(), "new");
    }
}
