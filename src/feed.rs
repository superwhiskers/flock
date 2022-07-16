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
use instant_glicko_2::{algorithm as glicko_2, constants as glicko_2_constants, Parameters};
use sqlx::{pool::PoolConnection, Sqlite};
use std::{
    collections::HashSet,
    time::{Duration, SystemTime},
};

use crate::model;

//TODO(superwhiskers): put feed stuff here
pub async fn generate_feed(
    mut connection: PoolConnection<Sqlite>,
    account_id: String,
) -> Result<[String; 10], (StatusCode, &'static str)> {
    let mut candidates: HashSet<String> = HashSet::new();

    for tag in sqlx::query_scalar!(
        r#"SELECT tag as "tag!" FROM scores WHERE id = ?"#,
        account_id
    )
    .fetch_all(&mut connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to query the account's tags from the db",
        )
    })? {
        candidates.extend(
            sqlx::query_scalar!(
                r#"SELECT links.link_id as "link_id!"
                     FROM links
         WHERE NOT EXISTS (
                            SELECT 1
                              FROM seen
                             WHERE seen.account_id = ?
                               AND seen.link_id = links.link_id
                          )
               AND EXISTS (
                            SELECT 1
                              FROM scores
                             WHERE scores.tag = ?
                               AND scores.id = links.link_id
                          )"#,
                account_id,
                tag
            )
            .fetch_all(&mut connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to query candidate links from the db",
                )
            })?
            .into_iter(),
        );
    }

    //TODO(superwhiskers): finish your implementation
    let tags = sqlx::query!(
        r#"SELECT tag as "tag!", score as "score!" FROM scores WHERE id = ?"#,
        account_id
    )
    .fetch_all(&mut connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to query the account's tags from the db",
        )
    })?;

    //TODO(superwhiskers): pull this out into a function
    let mut tags = tags
        .into_iter()
        .map(|tag| {
            rmp_serde::from_slice(&tag.score)
                .map(|score| (tag.tag, score))
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to deserialize the score data for a tag",
                    )
                })
        })
        .collect::<Result<Vec<(String, model::Score)>, _>>()?;

    //TODO(superwhiskers): something something use scaledratingdata
    let mut tag_rating_sum = 0;
    let mut tag_deviation_sum = 0;
    let mut tag_volatility_sum = 0;

    for (ref tag, ref mut score) in &mut tags {
        let months =
            (SystemTime::UNIX_EPOCH + Duration::from_secs(score.last_period))
                .elapsed()
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to calculate the amount of time that has passed since the previous rating interval for a tag",
                    )
                })?
                .as_secs()
                    / (60 * 60 * 24 * 30);

        if months != 0 {
            for _ in 0..months {
                glicko_2::close_player_rating_period_scaled(
                    &mut score.score,
                    &[],
                    Parameters::new(
                        glicko_2_constants::DEFAULT_START_RATING,
                        0.6,
                        glicko_2_constants::DEFAULT_CONVERGENCE_TOLERANCE,
                    ),
                )
            }

            let score = rmp_serde::to_vec(&*score).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to convert data to messagepack",
                )
            })?;

            sqlx::query!(
                r"UPDATE scores SET score = ? WHERE id = ? AND tag = ?",
                score,
                account_id,
                tag
            )
            .execute(&mut connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to update a score",
                )
            })?;
        }

        tag_score_sum += score.score.
    }

    //TODO(superwhiskers): iterate over the candidate links and perform the operations as laid out in the notes document
    ,

    Ok([
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
    ])
}
