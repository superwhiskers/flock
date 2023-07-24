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
use rand::seq::SliceRandom;
use sqlx::{pool::PoolConnection, Sqlite};
use std::collections::{HashMap, HashSet};
use tracing::{debug, trace};

use crate::{
    configuration::Algorithm as AlgorithmConfiguration,
    model,
    rand::pcg_thread_rng,
    util::{self, ScaledRatingData, ScaledRatingWrapper},
};

//TODO(superwhiskers): this is all pretty suboptimal. pass over it and optimize it
pub async fn generate_feed<'a>(
    algorithm_configuration: &'a AlgorithmConfiguration,
    mut connection: PoolConnection<Sqlite>,
    account_id: &'a str,
) -> Result<Vec<(String, ScaledRatingData)>, (StatusCode, &'static str)> {
    trace!("generating feed for account {}", account_id);

    let mut candidates: HashSet<String> = HashSet::new();

    for tag in sqlx::query_scalar!(
        r#"SELECT tag_id as "tag_id!" FROM scores WHERE id = ?"#,
        account_id
    )
    .fetch_all(&mut *connection)
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
                             WHERE scores.tag_id = ?
                               AND scores.id = links.link_id
                          )"#,
                account_id,
                tag
            )
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to query candidate links from the db",
                )
            })?,
        );
    }

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    //TODO(superwhiskers): it would likely be more beneficial memory-wise to not use
    //                     fetch_all and instead use fetch, if possible
    //TODO(superwhiskers): consider parallelizing this
    let mut tags = sqlx::query!(
        r#"SELECT tag_id as "tag_id!", score as "score!" FROM scores WHERE id = ?"#,
        account_id
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to query the account's tags from the db",
        )
    })?
    .into_iter()
    .map(|tag| {
        rmp_serde::from_slice(&tag.score)
            .map(|score| (tag.tag_id, score))
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to deserialize the score data for a tag",
                )
            })
    })
    .collect::<Result<Vec<(String, model::Score)>, _>>()?;

    let mut tag_sum = ScaledRatingData {
        rating: 0.0,
        deviation: 0.0,
        volatility: 0.0,
    };

    for (ref tag, ref mut score) in &mut tags {
        if util::decay_score(algorithm_configuration, score, 1)? {
            let score = rmp_serde::to_vec(&*score).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to convert data to messagepack",
                )
            })?;

            sqlx::query!(
                r"UPDATE scores SET score = ? WHERE id = ? AND tag_id = ?",
                score,
                account_id,
                tag
            )
            .execute(&mut *connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to update a score",
                )
            })?;
        }

        tag_sum += ScaledRatingWrapper(score.score).abs();
    }

    let mut tag_importance = HashMap::with_capacity(tags.len());
    for (tag, score) in tags {
        let importance = (ScaledRatingWrapper(score.score).abs() / tag_sum).prune_nan();
        debug!(
            "inserting tag importance for {} with score {:?}: {:?}",
            tag, score, importance
        );
        tag_importance.insert(tag, importance);
    }

    let mut candidate_scores = Vec::new();
    for candidate in candidates {
        //TODO(superwhiskers): perhaps we should factor this out into a function, given that
        //                     we do this twice (and will probably do it more times)
        //TODO(superwhiskers): the same note about using fetch over fetch_all applies here
        //                     as well
        let candidate_tags = sqlx::query!(
            r#"SELECT tag_id as "tag_id!", score as "score!" FROM scores WHERE id = ?"#,
            candidate,
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to query a candidate link's tags from the db",
            )
        })?
        .into_iter()
        .map(|tag| {
            rmp_serde::from_slice(&tag.score)
                .map(|score| (tag.tag_id, score))
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to deserialize the score data for a tag",
                    )
                })
        })
        .collect::<Result<Vec<(String, model::Score)>, _>>()?;

        let mut scaled_avg = ScaledRatingData {
            rating: 0.0,
            deviation: 0.0,
            volatility: 0.0,
        };
        let mut overlap = 0.0; // there will always be overlap.
        for (tag, mut score) in candidate_tags {
            if let Some(percentage) = tag_importance.get(&tag) {
                if util::decay_score(algorithm_configuration, &mut score, 12)? {
                    let score = rmp_serde::to_vec(&score).map_err(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "unable to convert data to messagepack",
                        )
                    })?;

                    sqlx::query!(
                        r"UPDATE scores SET score = ? WHERE id = ? AND tag_id = ?",
                        score,
                        candidate,
                        tag
                    )
                    .execute(&mut *connection)
                    .await
                    .map_err(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "unable to update a score",
                        )
                    })?;
                }

                scaled_avg += *percentage * ScaledRatingWrapper(score.score);
                overlap += 1.0;
            }
        }

        scaled_avg /= overlap;
        if scaled_avg.rating.is_nan()
            || scaled_avg.deviation.is_nan()
            || scaled_avg.volatility.is_nan()
        {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "invariant violation lmao (nan)",
            ));
        }
        candidate_scores.push((candidate, scaled_avg));
    }

    candidate_scores.sort_unstable_by(|(_, ref left), (_, ref right)| {
        left.partial_cmp(right)
            .expect("invariant violation lmao (nan)")
    });

    let mut rng = pcg_thread_rng();
    let mut feed = Vec::with_capacity(10);
    let segment_length = candidate_scores.len().div_ceil(4);
    for (i, segment) in candidate_scores.rchunks_mut(segment_length).enumerate() {
        feed.extend(
            segment
                .choose_multiple(&mut rng, 4 - i)
                .map(|(id, overall_score)| (id.to_string(), *overall_score)),
        );
    }

    feed.shuffle(&mut rng);

    Ok(feed)
}
