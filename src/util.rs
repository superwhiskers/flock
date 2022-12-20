//
//  flock - baa (with twenty instances of the letter "a")
//  Copyright (C) superwhiskers <whiskerdev@protonmail.com> 2022
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Affero General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Affero General Public License for more details.
//
//  You should have received a copy of the GNU Affero General Public License
//  along with this program.  If not, see <https://www.gnu.org/licenses/>.
//

use axum::http::StatusCode;
use instant_glicko_2::{
    algorithm as glicko_2, constants as glicko_2_constants, Parameters, ScaledRating,
};
use sqlx::SqlitePool;
use std::{
    borrow::Borrow,
    cmp::{self, Ordering},
    fmt::Debug,
    ops::{self, RangeInclusive},
    time::{Duration, SystemTime},
};
use tokio::signal;
use tracing::info;

use crate::model;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaledRatingData {
    pub rating: f64,
    pub deviation: f64,
    pub volatility: f64,
}

impl ScaledRatingData {
    pub fn prune_nan(self) -> Self {
        Self {
            rating: self.rating.is_nan().then_some(0.0).unwrap_or(self.rating),
            deviation: self
                .deviation
                .is_nan()
                .then_some(0.0)
                .unwrap_or(self.deviation),
            volatility: self
                .volatility
                .is_nan()
                .then_some(0.0)
                .unwrap_or(self.volatility),
        }
    }

    pub fn to_range(&self) -> RangeInclusive<f64> {
        RangeInclusive::new(self.rating - self.deviation, self.rating + self.deviation)
    }

    pub fn cmp_volatility(&self, other: &Self) -> Option<cmp::Ordering> {
        self.deviation
            .partial_cmp(&other.deviation)
            .and_then(|ord| {
                if ord == Ordering::Equal {
                    self.volatility
                        .partial_cmp(&other.volatility)
                        .map(|ord| match ord {
                            Ordering::Greater => Ordering::Less,
                            Ordering::Less => Ordering::Greater,
                            Ordering::Equal => Ordering::Equal,
                        })
                } else if ord == Ordering::Less {
                    Some(Ordering::Greater)
                } else {
                    Some(Ordering::Less)
                }
            })
    }
}

impl From<ScaledRatingWrapper> for ScaledRatingData {
    fn from(value: ScaledRatingWrapper) -> Self {
        ScaledRatingData {
            rating: value.0.rating(),
            deviation: value.0.deviation(),
            volatility: value.0.volatility(),
        }
    }
}

impl cmp::PartialOrd for ScaledRatingData {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        self.rating.partial_cmp(&other.rating).and_then(|ord| {
            if ord == Ordering::Equal {
                self.deviation
                    .partial_cmp(&other.deviation)
                    .and_then(|ord| {
                        if ord == Ordering::Equal {
                            self.volatility
                                .partial_cmp(&other.volatility)
                                .map(|ord| match ord {
                                    Ordering::Greater => Ordering::Less,
                                    Ordering::Less => Ordering::Greater,
                                    Ordering::Equal => Ordering::Equal,
                                })
                        } else if ord == Ordering::Less {
                            Some(Ordering::Greater)
                        } else {
                            Some(Ordering::Less)
                        }
                    })
            } else {
                Some(ord)
            }
        })
    }
}

impl ops::AddAssign<ScaledRatingWrapper> for ScaledRatingData {
    fn add_assign(&mut self, rhs: ScaledRatingWrapper) {
        self.rating += rhs.0.rating();
        self.deviation += rhs.0.deviation();
        self.volatility += rhs.0.volatility();
    }
}

impl ops::AddAssign for ScaledRatingData {
    fn add_assign(&mut self, rhs: ScaledRatingData) {
        self.rating += rhs.rating;
        self.deviation += rhs.deviation;
        self.volatility += rhs.volatility;
    }
}

impl ops::Add<ScaledRatingWrapper> for ScaledRatingData {
    type Output = ScaledRatingData;

    fn add(mut self, rhs: ScaledRatingWrapper) -> Self::Output {
        self += rhs;
        self
    }
}

impl ops::Add for ScaledRatingData {
    type Output = ScaledRatingData;

    fn add(mut self, rhs: ScaledRatingData) -> Self::Output {
        self += rhs;
        self
    }
}

impl<RW> ops::MulAssign<RW> for ScaledRatingData
where
    RW: Borrow<ScaledRatingWrapper>,
{
    fn mul_assign(&mut self, rhs: RW) {
        let rhs = rhs.borrow();
        self.rating *= rhs.0.rating();
        self.deviation *= rhs.0.deviation();
        self.volatility *= rhs.0.volatility();
    }
}

impl<RW> ops::Mul<RW> for ScaledRatingData
where
    RW: Borrow<ScaledRatingWrapper>,
{
    type Output = ScaledRatingData;

    fn mul(mut self, rhs: RW) -> Self::Output {
        self *= rhs.borrow();
        self
    }
}

impl ops::DivAssign<f64> for ScaledRatingData {
    fn div_assign(&mut self, rhs: f64) {
        self.rating /= rhs;
        self.deviation /= rhs;
        self.volatility /= rhs;
    }
}

impl ops::Div<f64> for ScaledRatingData {
    type Output = ScaledRatingData;

    fn div(mut self, rhs: f64) -> Self::Output {
        self /= rhs;
        self
    }
}

impl<RD> ops::Div<RD> for ScaledRatingData
where
    RD: Borrow<ScaledRatingData>,
{
    type Output = ScaledRatingData;

    fn div(self, rhs: RD) -> Self::Output {
        let rhs = rhs.borrow();
        ScaledRatingData {
            rating: self.rating / rhs.rating,
            deviation: self.deviation / rhs.deviation,
            volatility: self.volatility / rhs.volatility,
        }
    }
}

#[derive(Debug)]
pub struct ScaledRatingWrapper(pub ScaledRating);

impl ScaledRatingWrapper {
    pub fn abs(self) -> ScaledRatingData {
        ScaledRatingData {
            rating: self.0.rating().abs(),
            deviation: self.0.deviation().abs(),
            volatility: self.0.volatility().abs(),
        }
    }
}

impl<RD> ops::Div<RD> for ScaledRatingWrapper
where
    RD: Borrow<ScaledRatingData>,
{
    type Output = ScaledRatingData;

    fn div(self, rhs: RD) -> Self::Output {
        let rhs = rhs.borrow();
        ScaledRatingData {
            rating: self.0.rating() / rhs.rating,
            deviation: self.0.deviation() / rhs.deviation,
            volatility: self.0.volatility() / rhs.volatility,
        }
    }
}

pub fn rating_overlap(a: ScaledRatingData, b: ScaledRatingData) -> f64 {
    let (s1, e1) = (a.rating - a.deviation, a.rating + a.deviation);
    let (s2, e2) = (b.rating - b.deviation, b.rating + b.deviation);

    f64::min(e1, e2) - f64::max(s1, s2)
}

pub fn decay_score(
    score: &mut model::Score,
    period: u64,
) -> Result<bool, (StatusCode, &'static str)> {
    let period_as_seconds = 60 * 60 * 24 * 30 * period;

    let periods =
        (SystemTime::UNIX_EPOCH + Duration::from_secs(score.last_period))
            .elapsed()
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to calculate the amount of time that has passed since the previous rating interval for a tag",
                )
            })?
            .as_secs()
                / period_as_seconds;

    Ok(if periods != 0 {
        for i in 0..periods {
            glicko_2::close_player_rating_period_scaled(
                &mut score.score,
                if i == 0 {
                    score.result_queue.as_slice()
                } else {
                    &[]
                },
                //TODO(superwhiskers): should we make these configurable?
                Parameters::new(
                    glicko_2_constants::DEFAULT_START_RATING,
                    0.6,
                    glicko_2_constants::DEFAULT_CONVERGENCE_TOLERANCE,
                ),
            )
        }

        score.result_queue.clear();

        score.last_period += period_as_seconds * periods;

        true
    } else if score.result_queue.len() >= 10 {
        //TODO(superwhiskers): ditto
        glicko_2::close_player_rating_period_scaled(
            &mut score.score,
            score.result_queue.as_slice(),
            Parameters::new(
                glicko_2_constants::DEFAULT_START_RATING,
                0.6,
                glicko_2_constants::DEFAULT_CONVERGENCE_TOLERANCE,
            ),
        );

        true
    } else {
        false
    })
}

#[cfg(unix)]
pub async fn signal_handler(sqlite: SqlitePool) {
    let sigterm = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install a sigterm handler")
            .recv()
            .await;
    };

    tokio::select! {
        _ = signal::ctrl_c() => {},
        _ = sigterm => {},
    }

    info!("stopping the server");

    sqlite.close().await
}

#[cfg(windows)]
pub async fn signal_handler() {
    signal::ctrl_c().await;

    info!("stopping the server");
}
