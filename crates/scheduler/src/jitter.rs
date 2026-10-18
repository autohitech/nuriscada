// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

//! 스캔 지터 계측: (실제 시작 시각 − 계획 시각)의 분포.

use std::time::Duration;

/// 보관하는 최근 표본 수.
pub const SAMPLE_CAPACITY: usize = 10_000;

/// 지터 통계. 최근 표본으로 백분위수를 계산하고 최댓값은 전체 기간으로 유지한다.
#[derive(Debug, Clone, Default)]
pub struct JitterStats {
    samples: Vec<u32>,
    next: usize,
    max_us: u64,
    total: u64,
}

impl JitterStats {
    /// 빈 통계.
    pub fn new() -> Self {
        Self::default()
    }

    /// 지터 하나를 기록한다.
    pub fn record(&mut self, jitter: Duration) {
        let us = u64::try_from(jitter.as_micros()).unwrap_or(u64::MAX);
        self.max_us = self.max_us.max(us);
        self.total += 1;
        let sample = u32::try_from(us).unwrap_or(u32::MAX);
        if self.samples.len() < SAMPLE_CAPACITY {
            self.samples.push(sample);
        } else {
            self.samples[self.next] = sample;
            self.next = (self.next + 1) % SAMPLE_CAPACITY;
        }
    }

    /// 기록한 표본의 총 수.
    pub fn count(&self) -> u64 {
        self.total
    }

    /// 전체 기간의 최댓값(마이크로초).
    pub fn max_us(&self) -> u64 {
        self.max_us
    }

    /// 최근 표본의 `p`번째 백분위수(0.0..=1.0, 마이크로초). 표본이 없으면 `None`.
    pub fn percentile_us(&self, p: f64) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        // 가장 가까운 순위(nearest-rank) 방식.
        let rank = ((p.clamp(0.0, 1.0) * sorted.len() as f64).ceil() as usize).max(1);
        Some(u64::from(sorted[rank - 1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_has_no_percentile() {
        assert_eq!(JitterStats::new().percentile_us(0.99), None);
    }

    #[test]
    fn nearest_rank_percentiles() {
        let mut s = JitterStats::new();
        for ms in 1..=100u64 {
            s.record(Duration::from_millis(ms));
        }
        assert_eq!(s.percentile_us(0.99), Some(99_000));
        assert_eq!(s.percentile_us(1.0), Some(100_000));
        assert_eq!(s.percentile_us(0.5), Some(50_000));
        assert_eq!(s.max_us(), 100_000);
        assert_eq!(s.count(), 100);
    }

    #[test]
    fn keeps_recent_samples_but_overall_max() {
        let mut s = JitterStats::new();
        s.record(Duration::from_secs(1));
        for _ in 0..SAMPLE_CAPACITY {
            s.record(Duration::from_micros(10));
        }
        assert_eq!(s.percentile_us(1.0), Some(10));
        assert_eq!(s.max_us(), 1_000_000);
        assert_eq!(s.count(), SAMPLE_CAPACITY as u64 + 1);
    }
}
