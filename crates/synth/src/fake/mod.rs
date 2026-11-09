//! 가짜 PII·번호 생성기 (PLAN.md 3단계).
//!
//! - 모든 생성기는 호출자가 넘긴 RNG만 쓴다 → 같은 시드면 같은 값 ([`seeded`])
//! - 번호는 자릿수 묶음([`NumberText`])으로 만들고, 표기(하이픈·공백·점·붙여 쓰기·전각·가림)는 [`NumberStyle`]로 따로 고른다
//! - 섞는 비율은 `assets/synth/ratios.toml`에만 둔다 (코드에 숫자로 박지 않음). 목록에서 고르는 값은 균등 선택
//!
//! 구성: 날짜, 식별번호(주민·외국인등록번호, 사업자·법인등록번호, 하드네거티브 번호), 전화번호,
//! 사람(한국인 이름·외국인 이름·이메일), 주소(국내·본국), 계좌·여권·운전면허.
//! 자료 표(성씨 빈도·이름 음절·도로명·국가별 이름·은행 형식)는 `assets/synth/*.toml` ([`data::FakeData`])

pub mod address;
pub mod data;
pub mod date;
pub mod finance;
pub mod ids;
pub mod person;
pub mod phone;
pub mod text;

pub use date::Ymd;
pub use text::{NumberStyle, NumberText, Separator};

use rand::{Rng, RngExt};
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::SeedableRng;

use crate::ratios::RatioConfig;

/// 합성에 쓰는 RNG (시드 고정, 플랫폼과 무관하게 같은 수열)
pub type SynthRng = ChaCha8Rng;

/// 시드로 RNG를 만든다
pub fn seeded(seed: u64) -> SynthRng {
    ChaCha8Rng::seed_from_u64(seed)
}

#[derive(Debug, thiserror::Error)]
pub enum FakeError {
    #[error("비율 설정에 표 `{0}`가 없습니다")]
    MissingRatio(String),
    #[error("비율 표 `{table}`에 키 `{key}`가 없습니다 (생성기가 기대하는 키)")]
    MissingRatioKey { table: String, key: String },
}

/// 비율 표에서 생성기가 기대하는 키들의 가중치를 순서대로 꺼낸다
pub(crate) fn ratio_weights<const N: usize>(
    config: &RatioConfig,
    table: &str,
    keys: [&str; N],
) -> Result<[f64; N], FakeError> {
    let t = config
        .ratio
        .get(table)
        .ok_or_else(|| FakeError::MissingRatio(table.to_string()))?;
    let mut out = [0.0; N];
    for (slot, key) in out.iter_mut().zip(keys) {
        *slot = *t
            .weights
            .get(key)
            .ok_or_else(|| FakeError::MissingRatioKey {
                table: table.to_string(),
                key: key.to_string(),
            })?;
    }
    Ok(out)
}

/// 가중치로 하나를 고른다 (가중치 합으로 정규화). 가중치가 모두 0이면 첫 번째
pub(crate) fn pick_weighted<R: Rng + ?Sized>(rng: &mut R, weights: &[f64]) -> usize {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return 0;
    }
    let mut x = rng.random_range(0.0..total);
    for (i, w) in weights.iter().enumerate() {
        if x < *w {
            return i;
        }
        x -= w;
    }
    weights.len().saturating_sub(1)
}

/// 목록에서 균등하게 하나 고른다 (빈 목록이면 `None`)
pub(crate) fn pick<'a, T, R: Rng + ?Sized>(rng: &mut R, items: &'a [T]) -> Option<&'a T> {
    if items.is_empty() {
        None
    } else {
        items.get(rng.random_range(0..items.len()))
    }
}

/// 0~9 숫자 n개를 문자열로 (첫 자리 범위 지정 가능)
pub(crate) fn digits<R: Rng + ?Sized>(
    rng: &mut R,
    n: usize,
    first: std::ops::RangeInclusive<u8>,
) -> String {
    let mut s = String::with_capacity(n);
    for i in 0..n {
        let d = if i == 0 {
            rng.random_range(first.clone())
        } else {
            rng.random_range(0..=9u8)
        };
        s.push(char::from(b'0' + d));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let a: Vec<String> = {
            let mut r = seeded(42);
            (0..20).map(|_| digits(&mut r, 8, 0..=9)).collect()
        };
        let b: Vec<String> = {
            let mut r = seeded(42);
            (0..20).map(|_| digits(&mut r, 8, 0..=9)).collect()
        };
        let c: Vec<String> = {
            let mut r = seeded(43);
            (0..20).map(|_| digits(&mut r, 8, 0..=9)).collect()
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn weighted_pick_follows_weights() {
        let mut r = seeded(1);
        let n = 20_000;
        let hits = (0..n)
            .filter(|_| pick_weighted(&mut r, &[0.7, 0.3]) == 0)
            .count();
        let p = hits as f64 / n as f64;
        assert!((p - 0.7).abs() < 0.02, "{p}");
        assert_eq!(pick_weighted(&mut r, &[0.0, 0.0]), 0);
    }
}
