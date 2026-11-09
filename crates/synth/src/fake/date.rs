//! 날짜 (외부 크레이트 없이 그레고리력 유효성만 다룬다)

use rand::{Rng, RngExt};

/// 연·월·일
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ymd {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

pub fn is_leap(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// 그 달의 날 수 (월이 1~12가 아니면 0)
pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

impl Ymd {
    pub fn is_valid(&self) -> bool {
        self.day >= 1 && self.day <= days_in_month(self.year, self.month)
    }

    /// `YYMMDD`
    pub fn yymmdd(&self) -> String {
        format!("{:02}{:02}{:02}", self.year % 100, self.month, self.day)
    }

    /// 1970-01-01부터 지난 날 수 (그레고리력, Hinnant의 days_from_civil)
    pub fn to_days(&self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// [`Ymd::to_days`]의 역 (연도는 0~65535 범위로 자름)
    pub fn from_days(days: i64) -> Ymd {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Ymd {
            year: year.clamp(0, i64::from(u16::MAX)) as u16,
            month: month as u8,
            day: day as u8,
        }
    }

    pub fn add_days(&self, n: i64) -> Ymd {
        Ymd::from_days(self.to_days() + n)
    }

    /// n개월 뒤 같은 날 (그 달에 없는 날이면 말일로)
    pub fn add_months(&self, n: u32) -> Ymd {
        let total = u32::from(self.month) - 1 + n;
        let year = self.year + (total / 12) as u16;
        let month = (total % 12 + 1) as u8;
        Ymd {
            year,
            month,
            day: self.day.min(days_in_month(year, month)),
        }
    }

    /// 기준일(`at`)에 만 나이
    pub fn age_at(&self, at: Ymd) -> u16 {
        let had_birthday = (at.month, at.day) >= (self.month, self.day);
        at.year
            .saturating_sub(self.year)
            .saturating_sub(u16::from(!had_birthday))
    }
}

/// 기준일에 만 나이가 `min_age..=max_age`가 되는 생년월일을 균등하게 고른다
pub fn birth_date_for_age<R: Rng + ?Sized>(
    rng: &mut R,
    at: Ymd,
    min_age: u16,
    max_age: u16,
) -> Ymd {
    loop {
        let year =
            rng.random_range(at.year.saturating_sub(max_age + 1)..=at.year.saturating_sub(min_age));
        let month = rng.random_range(1..=12u8);
        let day = rng.random_range(1..=days_in_month(year, month));
        let d = Ymd { year, month, day };
        let age = d.age_at(at);
        if (min_age..=max_age).contains(&age) {
            return d;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::seeded;

    #[test]
    fn validity_and_leap_years() {
        assert!(
            Ymd {
                year: 2024,
                month: 2,
                day: 29
            }
            .is_valid()
        );
        assert!(
            !Ymd {
                year: 2023,
                month: 2,
                day: 29
            }
            .is_valid()
        );
        assert!(
            !Ymd {
                year: 1900,
                month: 2,
                day: 29
            }
            .is_valid()
        );
        assert!(
            Ymd {
                year: 2000,
                month: 2,
                day: 29
            }
            .is_valid()
        );
        assert!(
            !Ymd {
                year: 2026,
                month: 4,
                day: 31
            }
            .is_valid()
        );
        assert!(
            !Ymd {
                year: 2026,
                month: 13,
                day: 1
            }
            .is_valid()
        );
        assert_eq!(
            Ymd {
                year: 1990,
                month: 1,
                day: 5
            }
            .yymmdd(),
            "900105"
        );
    }

    #[test]
    fn day_arithmetic_roundtrips() {
        let epoch = Ymd {
            year: 1970,
            month: 1,
            day: 1,
        };
        assert_eq!(epoch.to_days(), 0);
        let d = Ymd {
            year: 2024,
            month: 2,
            day: 28,
        };
        assert_eq!(
            d.add_days(1),
            Ymd {
                year: 2024,
                month: 2,
                day: 29
            }
        );
        assert_eq!(
            d.add_days(366),
            Ymd {
                year: 2025,
                month: 2,
                day: 28
            }
        );
        for n in -50_000..50_000 {
            let x = Ymd::from_days(n);
            assert!(x.is_valid());
            assert_eq!(x.to_days(), n);
        }
    }

    #[test]
    fn ages_fall_in_range() {
        let at = Ymd {
            year: 2026,
            month: 11,
            day: 8,
        };
        let mut r = seeded(7);
        for _ in 0..2000 {
            let d = birth_date_for_age(&mut r, at, 15, 17);
            assert!(d.is_valid());
            assert!((15..=17).contains(&d.age_at(at)), "{d:?}");
        }
    }
}
