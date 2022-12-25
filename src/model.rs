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

use instant_glicko_2::{algorithm::ScaledPlayerResult, ScaledRating};
use serde::{Deserialize, Serialize, de};

use crate::util::ScaledRatingData;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostLogin {
    pub account_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostSignup {
    pub tags: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostProfile {
    #[serde(default = "default_checkbox", deserialize_with = "deserialize_checkbox")]
    pub refresh_account_id: bool,
    pub tags: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Feed {
    /// The duration since the feed was last refreshed expressed in seconds since unix epoch
    pub refreshed: u64,

    // A vector of link ids and their overall scores selected to be in the feed
    pub links: Vec<(String, ScaledRatingData)>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Score {
    /// The Glicko-2 score associated with the (id, tag)
    pub score: ScaledRating,

    /// The time of the last rating period closure expressed in seconds since unix epoch
    pub last_period: u64,

    /// The queue of results that haven't been incorporated into the score
    pub result_queue: Vec<ScaledPlayerResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostPost {
    pub link: String,
    pub description: String,
    pub tags: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PostEditLink {
    pub description: String,
    pub tags: String,
}

fn default_checkbox() -> bool {
    false
}

fn deserialize_checkbox<'de, D>(_: D) -> Result<bool, D::Error>
where
    D: de::Deserializer<'de>,
{
    Ok(true)
}
