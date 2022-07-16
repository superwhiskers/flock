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

use serde::{Deserialize, Serialize};
use instant_glicko_2::{ScaledRating, algorithm::ScaledPlayerResult};

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostLogin {
    pub account_id: String,
}


#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostSignup {
    pub tags: String,
}

#[derive(Serialize, Deserialize)]
pub struct Feed {
    /// The duration since the feed was last refreshed expressed in seconds since unix epoch
    pub refreshed: u64,

    // An array of link ids selected to be in the feed
    pub links: [String; 10],
}

#[derive(Serialize, Deserialize)]
pub struct Score {
    /// The Glicko-2 score associated with the (id, tag)
    pub score: ScaledRating,

    /// The number of ratings that have been made since the last rating period closure
    pub ratings_since_last_period: u8,

    /// The time of the last rating period closure expressed in seconds since unix epoch
    pub last_period: u64,

    /// The queue of results that haven't been incorporated into the score
    pub result_queue: Vec<ScaledPlayerResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostPost {
    pub link: String,
    pub description: String,
    pub tags: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostEditLink {
    pub description: String,
    pub tags: String,
}
