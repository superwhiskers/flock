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

use instant_glicko_2::ScaledRating;
use sqlx::SqlitePool;
use std::ops;
use tokio::signal;
use tracing::info;

#[derive(Clone)]
pub struct ScaledRatingData {
    pub rating: f64,
    pub deviation: f64,
    pub volatility: f64,
}

impl ops::AddAssign<ScaledRatingWrapper> for ScaledRatingData {
    fn add_assign(&mut self, rhs: ScaledRatingWrapper) {
        self.rating += rhs.0.rating();
        self.deviation += rhs.0.deviation();
        self.volatility += rhs.0.volatility();
    }
}

impl ops::Add<ScaledRatingWrapper> for ScaledRatingData {
    type Output = ScaledRatingData;

    fn add(self, rhs: ScaledRatingWrapper) -> Self::Output {
        let mut out = self.clone();
        out += rhs;
        out
    }
}

pub struct ScaledRatingWrapper(pub ScaledRating);

impl ops::Div<ScaledRatingData> for ScaledRatingWrapper {
    type Output = ScaledRatingData;

    fn div(self, rhs: ScaledRatingData) -> Self::Output {
        ScaledRatingData {
            rating: self.0.rating() / rhs.rating,
            deviation: self.0.deviation() / rhs.deviation,
            volatility: self.0.volatility() / rhs.volatility,
        }
    }
}

impl ops::Add for ScaledRatingWrapper {
    type Output = ScaledRatingData;

    fn add(self, rhs: Self) -> Self::Output {
        ScaledRatingData {
            rating: self.0.rating() + rhs.0.rating(),
            deviation: self.0.deviation() + rhs.0.deviation(),
            volatility: self.0.volatility() + rhs.0.volatility(),
        }
    }
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
