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

use axum::{
    body::Bytes,
    extract::{Form, Path},
    headers::Cookie,
    http::{header::SET_COOKIE, Response, StatusCode},
    response::{AppendHeaders, IntoResponse, Redirect},
    Extension, TypedHeader,
};
use http_body::combinators::UnsyncBoxBody;
use instant_glicko_2::{algorithm::ScaledPlayerResult, ScaledRating};
use regex::Regex;
use sqlx::SqlitePool;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::LazyLock,
    time::{Duration, SystemTime},
};
use tracing::{debug, trace};
use ulid::Ulid;

use crate::{
    configuration::Routes as RouteConfiguration,
    feed, model,
    rand::pcg_thread_rng,
    templates::{self, Link},
    util::{self, ScaledRatingData, ScaledRatingWrapper},
};

static TAG_DELIMITER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*,\s*").expect("unable to compile a regex"));

static TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[[:alnum:]\-]+\z").expect("unable to compile a regex"));

pub fn string_to_tags(tags: &mut str) -> Result<HashSet<&'_ str>, (StatusCode, &'static str)> {
    tags.make_ascii_lowercase();

    let tags = TAG_DELIMITER_REGEX.split(tags).collect::<HashSet<&str>>();

    if !tags.iter().all(|tag| TAG_REGEX.is_match(tag)) {
        return Err((StatusCode::BAD_REQUEST, "the provided tags are invalid"));
    }

    Ok(tags)
}

pub async fn index(
    Extension(sqlite): Extension<SqlitePool>,
    Extension(route_configuration): Extension<RouteConfiguration>,
    cookies: Option<TypedHeader<Cookie>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("index requested, cookies: {:?}", cookies);

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        trace!("preparing index for {}", account_id);

        let mut connection = sqlite.acquire().await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to acquire a db connection",
            )
        })?;

        if let Some(feed) = sqlx::query_scalar!(
            r#"SELECT feed as "feed!" FROM accounts WHERE account_id = ?"#,
            account_id
        )
        .fetch_optional(&mut connection)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?
        {
            let mut feed = rmp_serde::from_slice::<model::Feed>(&feed).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to deserialize the feed"))?;

            debug!("deserialized feed for {}: {:?}", account_id, feed);

            if (SystemTime::UNIX_EPOCH + Duration::from_secs(feed.refreshed))
                .elapsed()
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to calculate the amount of time that has passed since the last time the feed was refreshed"))?
                .as_secs()
                    > route_configuration.feed_refresh_period {
                trace!("generating new feed for {}", account_id);

                feed = model::Feed {
                    links: feed::generate_feed(sqlite.acquire().await.map_err(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "unable to acquire a db connection",
                        )
                    })?, account_id).await?,
                    refreshed: SystemTime::UNIX_EPOCH
                        .elapsed()
                        .map_err(|_| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "unable to calculate the amount of time that has passed since the unix epoch",
                            )
                        })?
                        .as_secs(),
                };

                debug!("new feed for {}: {:?}", account_id, feed);

                let serialized_feed = rmp_serde::to_vec(&feed)
                    .map_err(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "unable to serialize the new feed",
                        )
                    })?;

                sqlx::query!(
                    r"UPDATE accounts SET feed = ? WHERE account_id = ?",
                    serialized_feed,
                    account_id
                )
                .execute(&mut connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to update the feed",
                    )
                })?;
            }

            let mut links = Vec::with_capacity(feed.links.len());
            for (link_id, overall_score) in feed.links {
                let description = sqlx::query!(
                    r#"SELECT description as "description!" FROM links WHERE link_id = ?"#,
                    link_id,
                )
                .fetch_one(&mut connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to query for a link's information",
                    )
                })?
                .description;

                let (visited, rated) = sqlx::query!(
                    r#"SELECT rated as "rated!" FROM seen WHERE account_id = ? AND link_id = ?"#,
                    account_id,
                    link_id,
                )
                .fetch_optional(&mut connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to query the database of seen links for a link",
                    )
                })?.map(|result| (true, result.rated)).unwrap_or((false, false));

                links.push(Link {
                    id: link_id,
                    description,
                    rated,
                    rating: (visited && rated).then(|| overall_score.to_string()),
                    visited,
                });
            }

            trace!("sending response to {}", account_id);

            Ok((
                [("Content-Type", "application/xhtml+xml")],
                templates::Index {
                    account: Some(templates::Account {
                        id: account_id.to_string(),
                        links,
                    }),
                },
            ))
        } else {
            Err((
                StatusCode::BAD_REQUEST,
                "the requested account does not exist",
            ))
        }
    } else {
        trace!("sending logged-out index");

        Ok((
            [("Content-Type", "application/xhtml+xml")],
            templates::Index { account: None },
        ))
    }
}

pub async fn get_login() -> impl IntoResponse {
    (
        [("Content-Type", "application/xhtml+xml")],
        templates::Login,
    )
}

pub async fn post_login(
    Extension(sqlite): Extension<SqlitePool>,
    Extension(route_configuration): Extension<RouteConfiguration>,
    Form(model::PostLogin { account_id }): Form<model::PostLogin>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("login post-ed with account id {}", account_id);

    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    if sqlx::query!(
        r"SELECT COUNT(1) as count FROM accounts WHERE account_id = ?",
        account_id
    )
    .fetch_one(&mut connection)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?
    .count
        == 1
    {
        Ok((
            AppendHeaders([(
                SET_COOKIE,
                if route_configuration.secure_cookies {
                    format!(
                        "flock.id={}; SameSite=Strict; Expires={}; Max-Age=172800; HttpOnly; Secure",
                        account_id,
                        // lmao old browsers but why the heck not, it's hardly any effort
                        httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(172800))
                    )
                } else {
                    format!(
                        "flock.id={}; SameSite=Strict; Expires={}; Max-Age=172800; HttpOnly",
                        account_id,
                        // lmao old browsers but why the heck not, it's hardly any effort
                        httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(172800))
                    )
                },
            )]),
            Redirect::to("/"),
        ))
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "the requested account does not exist",
        ))
    }
}

pub async fn get_signup() -> impl IntoResponse {
    (
        [("Content-Type", "application/xhtml+xml")],
        templates::Signup,
    )
}

pub async fn post_signup(
    Extension(sqlite): Extension<SqlitePool>,
    Extension(route_configuration): Extension<RouteConfiguration>,
    Form(model::PostSignup { mut tags }): Form<model::PostSignup>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("signup post-ed, tags: \"{}\"", tags);

    let tags = string_to_tags(&mut tags)?;

    debug!("tags parsed as {:?} for account", tags);

    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    for tag in &tags {
        sqlx::query!(r"INSERT OR IGNORE INTO tags (tag) VALUES (?)", tag)
            .execute(&mut connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to insert a tag into the db",
                )
            })?;
    }

    let account_id = Ulid::with_source(&mut pcg_thread_rng()).to_string();

    debug!("account id generated: {}", account_id);

    let feed = rmp_serde::to_vec(&model::Feed {
        refreshed: 0,
        links: Default::default(),
    })
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to convert data to messagepack",
        )
    })?;

    sqlx::query!(
        r"INSERT INTO accounts (account_id, feed) VALUES (?, ?)",
        account_id,
        feed
    )
    .execute(&mut connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to insert a new account into the db",
        )
    })?;

    for tag in tags {
        let score = ScaledRating::new(
            0.0,
            350.0 / instant_glicko_2::constants::RATING_SCALING_RATIO,
            0.06,
        );
        let last_period = SystemTime::UNIX_EPOCH
            .elapsed()
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to calculate the amount of time that has passed since the unix epoch",
                )
            })?
            .as_secs();

        let score = rmp_serde::to_vec(&model::Score {
            score,
            last_period,
            result_queue: vec![],
        })
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to convert data to messagepack",
            )
        })?;

        sqlx::query!(
            r"INSERT INTO scores (id, tag, score) VALUES (?, ?, ?)",
            account_id,
            tag,
            score,
        )
        .execute(&mut connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to insert a tag score into the db",
            )
        })?;
    }

    Ok((
        AppendHeaders([(
            SET_COOKIE,
            if route_configuration.secure_cookies {
                format!(
                    "flock.id={}; SameSite=Strict; Expires={}; Max-Age=172800; HttpOnly; Secure",
                    account_id,
                    // lmao old browsers but why the heck not, it's hardly any effort
                    httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(172800))
                )
            } else {
                format!(
                    "flock.id={}; SameSite=Strict; Expires={}; Max-Age=172800; HttpOnly",
                    account_id,
                    // lmao old browsers but why the heck not, it's hardly any effort
                    httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(172800))
                )
            },
        )]),
        Redirect::to("/"),
    ))
}

pub async fn logout(
    cookies: Option<TypedHeader<Cookie>>,
    Extension(route_configuration): Extension<RouteConfiguration>,
) -> Response<UnsyncBoxBody<Bytes, axum::Error>> {
    trace!("logout requested, cookies: {:?}", cookies);

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        (
            AppendHeaders([
                (
                    SET_COOKIE,
                    if route_configuration.secure_cookies {
                        format!(
                            "flock.id={}; SameSite=Strict; Expires={}; Max-Age=0; HttpOnly; Secure",
                            account_id,
                            // lmao old browsers but why the heck not, it's hardly any effort
                            httpdate::fmt_http_date(SystemTime::UNIX_EPOCH)
                        )
                    } else {
                        format!(
                            "flock.id={}; SameSite=Strict; Expires={}; Max-Age=0; HttpOnly",
                            account_id,
                            // lmao old browsers but why the heck not, it's hardly any effort
                            httpdate::fmt_http_date(SystemTime::UNIX_EPOCH)
                        )
                    },
                ),
            ]),
            Redirect::to("/"),
        )
            .into_response()
    } else {
        Redirect::to("/").into_response()
    }
}

pub async fn tags(
    Extension(sqlite): Extension<SqlitePool>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    let tags = sqlx::query_scalar!(r#"SELECT tag as "tag!" FROM tags"#)
        .fetch_all(&mut connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to query the tags",
            )
        })?;

    Ok((
        [("Content-Type", "application/xhtml+xml")],
        templates::Tags { tags },
    ))
}

pub async fn get_post() -> impl IntoResponse {
    ([("Content-Type", "application/xhtml+xml")], templates::Post)
}

pub async fn post_post(
    Extension(sqlite): Extension<SqlitePool>,
    Form(model::PostPost {
        link,
        description,
        mut tags,
    }): Form<model::PostPost>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("post post-ed, tags: \"{}\"", tags);

    let tags = string_to_tags(&mut tags)?;

    debug!("tags parsed as {:?} for post", tags);

    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    for tag in &tags {
        sqlx::query!(r"INSERT OR IGNORE INTO tags (tag) VALUES (?)", tag)
            .execute(&mut connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to insert a tag into the db",
                )
            })?;
    }

    let link_id = Ulid::with_source(&mut pcg_thread_rng()).to_string();

    debug!("link id generated: {}", link_id);

    sqlx::query!(
        "INSERT INTO LINKS (link_id, link, description) VALUES (?, ?, ?)",
        link_id,
        link,
        description
    )
    .execute(&mut connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to insert the link into the db",
        )
    })?;

    //TODO(superwhiskers): this and the similar loop used in account creation (and likely
    //                     account tag modification) could be factored out
    for tag in tags {
        let score = ScaledRating::new(
            0.0,
            350.0 / instant_glicko_2::constants::RATING_SCALING_RATIO,
            0.06,
        );
        let last_period = SystemTime::UNIX_EPOCH
            .elapsed()
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to calculate the amount of time that has passed since the unix epoch",
                )
            })?
            .as_secs();

        let score = rmp_serde::to_vec(&model::Score {
            score,
            last_period,
            result_queue: vec![],
        })
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to convert data to messagepack",
            )
        })?;

        sqlx::query!(
            r"INSERT INTO scores (id, tag, score) VALUES (?, ?, ?)",
            link_id,
            tag,
            score,
        )
        .execute(&mut connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to insert a tag score into the db",
            )
        })?;
    }

    Ok(Redirect::to("/"))
}

pub async fn link(
    Extension(sqlite): Extension<SqlitePool>,
    cookies: Option<TypedHeader<Cookie>>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!(
        "link requested, cookies: {:?}, link id: {}",
        cookies,
        link_id
    );

    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    let link = sqlx::query_scalar!(
        r#"SELECT link as "link!" FROM links WHERE link_id = ?"#,
        link_id
    )
    .fetch_optional(&mut connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to query the db for the link",
        )
    })?
    .ok_or((StatusCode::BAD_REQUEST, "the requested link does not exist"))?;

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        debug!("account {} requested link {}", account_id, link_id);

        if sqlx::query!(
            r"SELECT COUNT(1) as count FROM accounts WHERE account_id = ?",
            account_id
        )
        .fetch_one(&mut connection)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?
        .count
            == 1
        {
            sqlx::query!(
                "INSERT OR IGNORE INTO seen (account_id, link_id, rated) VALUES (?, ?, false)",
                account_id, link_id
            ).execute(&mut connection).await.map_err(|_| {
                (StatusCode::INTERNAL_SERVER_ERROR, "unable to mark this link as seen")
            })?;

            Ok(Redirect::to(&link))
        } else {
            Err((
                StatusCode::BAD_REQUEST,
                "the requested account does not exist",
            ))
        }
    } else {
        Ok(Redirect::to(&link))
    }
}

//TODO(superwhiskers): require for a link to have been viewed (and potentially rated) before allowing one to edit it
pub async fn get_edit_link(
    Extension(sqlite): Extension<SqlitePool>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    let description = sqlx::query_scalar!(
        r#"SELECT description as "description!" FROM links WHERE link_id = ?"#,
        link_id
    )
    .fetch_optional(&mut connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to query the db for the link",
        )
    })?
    .ok_or((StatusCode::BAD_REQUEST, "the requested link does not exist"))?;

    let tags = sqlx::query_scalar!(r#"SELECT tag as "tag!" FROM scores WHERE id = ?"#, link_id)
        .fetch_all(&mut connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to query the tags for the link",
            )
        })?;

    Ok(templates::EditLink {
        id: link_id,
        description,
        tags: tags.join(","),
    })
}

pub async fn post_edit_link(
    Extension(sqlite): Extension<SqlitePool>,
    Path(link_id): Path<String>,
    Form(model::PostEditLink {
        description,
        mut tags,
    }): Form<model::PostEditLink>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    if sqlx::query!(
        r"SELECT COUNT(1) as count FROM links WHERE link_id = ?",
        link_id
    )
    .fetch_one(&mut connection)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?
    .count
        == 1
    {
        let tags = string_to_tags(&mut tags)?;
        let old_tags_owned =
            sqlx::query_scalar!(r#"SELECT tag as "tag!" FROM scores WHERE id = ?"#, link_id)
                .fetch_all(&mut connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to query the old tags",
                    )
                })?;
        let old_tags = old_tags_owned
            .iter()
            .map(|t| t.as_str())
            .collect::<HashSet<_>>();

        for tag in &tags {
            sqlx::query!(r"INSERT OR IGNORE INTO tags (tag) VALUES (?)", tag)
                .execute(&mut connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to insert a tag into the db",
                    )
                })?;
        }

        sqlx::query!(
            r"UPDATE links SET description = ? WHERE link_id = ?",
            description,
            link_id
        )
        .execute(&mut connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to update the link's information",
            )
        })?;

        for tag in &old_tags - &tags {
            sqlx::query!(r"DELETE FROM scores WHERE id = ? AND tag = ?", link_id, tag,)
                .execute(&mut connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to remove an old tag from the db",
                    )
                })?;
        }

        for tag in &tags - &old_tags {
            let score = ScaledRating::new(
                0.0,
                350.0 / instant_glicko_2::constants::RATING_SCALING_RATIO,
                0.06,
            );
            let last_period = SystemTime::UNIX_EPOCH
                .elapsed()
                .map_err(|_| {
                    (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to calculate the amount of time that has passed since the unix epoch",
                )
                })?
                .as_secs();

            let score = rmp_serde::to_vec(&model::Score {
                score,
                last_period,
                result_queue: vec![],
            })
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to convert data to messagepack",
                )
            })?;

            sqlx::query!(
                r"INSERT INTO scores (id, tag, score) VALUES (?, ?, ?)",
                link_id,
                tag,
                score,
            )
            .execute(&mut connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to insert a tag score into the db",
                )
            })?;
        }

        Ok(Redirect::to("/"))
    } else {
        Err((StatusCode::BAD_REQUEST, "the requested link does not exist"))
    }
}

//TODO(superwhiskers): implement link rating
pub async fn get_promote_link(
    Extension(sqlite): Extension<SqlitePool>,
    cookies: Option<TypedHeader<Cookie>>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    rate_link(sqlite, cookies, link_id, 0.75).await
}

pub async fn get_neutral_link(
    Extension(sqlite): Extension<SqlitePool>,
    cookies: Option<TypedHeader<Cookie>>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    rate_link(sqlite, cookies, link_id, 0.5).await
}

pub async fn get_demote_link(
    Extension(sqlite): Extension<SqlitePool>,
    cookies: Option<TypedHeader<Cookie>>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    rate_link(sqlite, cookies, link_id, 0.0).await
}

#[inline(always)]
pub async fn rate_link(
    sqlite: SqlitePool,
    cookies: Option<TypedHeader<Cookie>>,
    link_id: String,
    base_outcome: f64,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("link rated: {}, cookies: {:?}", link_id, cookies);

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        debug!("account {} rating link {} with base outcome {}", account_id, link_id, base_outcome);

        let mut connection = sqlite.acquire().await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to acqire a db connection",
            )
        })?;

        //TODO(superwhiskers): same thing mentioned in src/feed.rs, but we should
        //                     additionally consider making this a function at this point
        let user_scores = sqlx::query!(
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
        })?
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
        .collect::<Result<HashMap<String, model::Score>, _>>()?;

        let link_scores = sqlx::query!(
            r#"SELECT tag as "tag!", score as "score!" FROM scores WHERE id = ?"#,
            link_id
        )
        .fetch_all(&mut connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to query the link's tags from the db",
            )
        })?
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
        .collect::<Result<HashMap<String, model::Score>, _>>()?;

        let user_tags = user_scores
            .keys()
            .collect::<HashSet<_>>();

        let link_tags = link_scores
            .keys()
            .collect::<HashSet<_>>();

        for tag in HashSet::intersection(&user_tags, &link_tags) {
            let (mut user_score_data, mut link_score_data) = user_scores.get(*tag).cloned().zip(link_scores.get(*tag).cloned()).unwrap();
            let (user_score, link_score) =
                (
                    ScaledRatingWrapper(user_score_data.score).into(),
                    ScaledRatingWrapper(link_score_data.score).into(),
                );
            let comparative_volatility = ScaledRatingData::cmp_volatility(&user_score, &link_score).ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to compare the volatilities of scores",
            ))?;

            let (user_outcome, link_outcome) = if comparative_volatility == Ordering::Equal {
                (base_outcome, base_outcome)
            } else {
                let overlap = util::rating_overlap(user_score, link_score);
                if overlap.is_nan() {
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, "a nan was encountered while calculating overlap"));
                }

                let percent_overlap = overlap / if overlap.is_sign_positive() {
                    2.0 * f64::min(user_score.deviation, link_score.deviation)
                } else {
                     user_score.deviation + link_score.deviation + (user_score.rating - link_score.rating).abs()
                };

                let tweaked_outcome = base_outcome + (0.25 * percent_overlap);
                let (favorable_outcome, unfavorable_outcome) = if tweaked_outcome > base_outcome {
                    (tweaked_outcome, base_outcome)
                } else {
                    (base_outcome, tweaked_outcome)
                };

                if comparative_volatility == Ordering::Less {
                    (favorable_outcome, unfavorable_outcome)
                } else {
                    (unfavorable_outcome, favorable_outcome)
                }
            };

            user_score_data.result_queue.push(ScaledPlayerResult::new(link_score_data.score, user_outcome));
            link_score_data.result_queue.push(ScaledPlayerResult::new(user_score_data.score, link_outcome));

            util::decay_score(&mut user_score_data, 1)?;
            util::decay_score(&mut link_score_data, 12)?;

            let user_score_bin = rmp_serde::to_vec(&user_score_data).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to convert data to messagepack",
                )
            })?;

            sqlx::query!(
                "UPDATE scores SET score = ? WHERE id = ? AND tag = ?",
                user_score_bin,
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

            let link_score_bin = rmp_serde::to_vec(&link_score_data).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to convert data to messagepack",
                )
            })?;

            sqlx::query!(
                "UPDATE scores SET score = ? WHERE id = ? AND tag = ?",
                link_score_bin,
                link_id,
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

            // someone's going to try to rate before viewing. this handles that edge case
            sqlx::query!(
                "INSERT INTO seen (account_id, link_id, rated) VALUES (?, ?, true) ON CONFLICT (account_id, link_id) DO UPDATE SET rated = true",
                account_id,
                link_id
            )
            .execute(&mut connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to update the rated value",
                )
            })?;
        }
    }

    Ok(Redirect::to("/"))
}
