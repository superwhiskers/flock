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

//TODO(superwhiskers): this file needs a massive refactoring to reduce code duplication. i
//                     strongly suggest leaving this alone until that is complete

use askama::Template;
use axum::{
    body::Bytes,
    extract::{Form, Multipart, Path, Query},
    headers::Cookie,
    http::{header::SET_COOKIE, Response, StatusCode},
    response::{AppendHeaders, IntoResponse, Redirect},
    Extension, TypedHeader,
};
use http_body::combinators::UnsyncBoxBody;
use instant_glicko_2::{algorithm::ScaledPlayerResult, ScaledRating};
use regex::Regex;
use sqlx::{pool::PoolConnection, Sqlite, SqlitePool};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::LazyLock,
    time::{Duration, SystemTime},
};
use tracing::{debug, trace};
use ulid::Ulid;

use crate::{
    configuration::{
        Algorithm as AlgorithmConfiguration, Http as HttpConfiguration,
        Routes as RouteConfiguration,
    },
    feed,
    locks::LockMap,
    model,
    rand::pcg_thread_rng,
    templates::{self, Link},
    util::{self, ScaledRatingData, ScaledRatingWrapper},
};

static TAG_DELIMITER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*,\s*").expect("unable to compile a regex"));

static TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[[:alnum:]\-]+\z").expect("unable to compile a regex"));

/// Shorthand for checking if the feature gate is enabled
macro_rules! coz_progress {
    () => {{
        #[cfg(feature = "coz")]
        ::coz::progress!();
    }};

    ($name:expr) => {{
        #[cfg(feature = "coz")]
        ::coz::progress!($name);
    }};
}

pub fn string_to_tags(tags: &mut str) -> Result<HashSet<&'_ str>, (StatusCode, &'static str)> {
    tags.make_ascii_lowercase();

    let tags = TAG_DELIMITER_REGEX.split(tags).collect::<HashSet<&str>>();

    if !tags.iter().all(|tag| TAG_REGEX.is_match(tag)) {
        return Err((StatusCode::BAD_REQUEST, "the provided tags are invalid"));
    }

    Ok(tags)
}

//TODO(superwhiskers): remove when the heuristics are corrected and/or fix
#[allow(clippy::needless_pass_by_ref_mut)]
pub async fn retrieve_tags_from_string(
    connection: &mut PoolConnection<Sqlite>,
    mut names: String,
) -> Result<Vec<String>, (StatusCode, &'static str)> {
    trace!("retrieving tags from \"{}\"", names);

    let names = string_to_tags(&mut names)?;
    let mut ids = Vec::with_capacity(names.len());

    debug!("parsed tag names as {:?}", names);

    for name in names {
        let id = Ulid::with_source(&mut pcg_thread_rng()).to_string();

        sqlx::query!(
            r"INSERT OR IGNORE INTO tags (tag_id, name) VALUES (?, ?)",
            id,
            name
        )
        .execute(&mut **connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to insert a tag into the db",
            )
        })?;

        // it is necessary for this to be after as it ensures if any racy initialization of
        // a tag happens that the correct tag_id will be retrieved
        ids.push(
            sqlx::query_scalar!(
                r#"SELECT tag_id as "tag_id!" FROM tags WHERE name = ?"#,
                name
            )
            .fetch_one(&mut **connection)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?,
        );
    }

    Ok(ids)
}

pub async fn get_index(
    Extension(style_id): Extension<model::StyleId>,
    Extension(sqlite): Extension<SqlitePool>,
    Extension(algorithm_configuration): Extension<AlgorithmConfiguration>,
    Extension(lock_map): Extension<&'static LockMap>,
    cookies: Option<TypedHeader<Cookie>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("index requested, cookies: {:?}", cookies);

    coz_progress!();

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
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?
        {
            let mut feed = rmp_serde::from_slice::<model::Feed>(&feed).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to deserialize the feed"))?;

            debug!("deserialized feed for {}: {:?}", account_id, feed);

            if (SystemTime::UNIX_EPOCH + Duration::from_secs(feed.refreshed))
                .elapsed()
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to calculate the amount of time that has passed since the last time the feed was refreshed",
                    )
                })?
                    > algorithm_configuration.feed_refresh_period {
                trace!("generating new feed for {}", account_id);

                trace!("locking the account's tags");

                let _tag_lock = lock_map.lock(account_id).ok_or((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "a lock is currently held on your account's tag information. try again in a few seconds",
                ))?;

                feed = model::Feed {
                    links: feed::generate_feed(&algorithm_configuration, sqlite.acquire().await.map_err(|_| {
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
                .execute(&mut *connection)
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
                .fetch_one(&mut *connection)
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
                .fetch_optional(&mut *connection)
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
                [("Content-Type", "application/xhtml+xml"), ("Cache-Control", "private, no-store")],
                templates::Index {
                    style_id,
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
            [("Content-Type", "application/xhtml+xml"), ("Cache-Control", "private, no-store")],
            templates::Index { style_id, account: None },
        ))
    }
}

pub async fn get_login(
    Extension(style_id): Extension<model::StyleId>,
    Query(model::Login { redirect_to }): Query<model::Login>,
    cookies: Option<TypedHeader<Cookie>>,
) -> Response<UnsyncBoxBody<Bytes, axum::Error>> {
    coz_progress!();

    if let Some(cookies) = cookies
        && cookies.get("flock.id").is_some() {
        if let Some(url) = redirect_to {
            Redirect::to(&url)
        } else {
            Redirect::to("/")
        }
        .into_response()
    } else {
        (
            [("Content-Type", "application/xhtml+xml")],
            templates::Login { style_id, redirect_to },
        )
        .into_response()
    }
}

pub async fn post_login(
    Extension(sqlite): Extension<SqlitePool>,
    Extension(route_configuration): Extension<RouteConfiguration>,
    Query(model::Login { redirect_to }): Query<model::Login>,
    Form(model::PostLogin { account_id }): Form<model::PostLogin>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("login post-ed with account id {}", account_id);

    coz_progress!();

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
    .fetch_one(&mut *connection)
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
            if let Some(url) = redirect_to {
                Redirect::to(&url)
            } else {
                Redirect::to("/")
            },
        ))
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "the requested account does not exist",
        ))
    }
}

pub async fn get_signup(Extension(style_id): Extension<model::StyleId>) -> impl IntoResponse {
    coz_progress!();

    (
        [("Content-Type", "application/xhtml+xml")],
        templates::Signup { style_id },
    )
}

pub async fn post_signup(
    Extension(sqlite): Extension<SqlitePool>,
    Extension(route_configuration): Extension<RouteConfiguration>,
    Form(model::PostSignup { tags }): Form<model::PostSignup>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("signup post-ed, tags: \"{}\"", tags);

    coz_progress!();

    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    let account_id = Ulid::with_source(&mut pcg_thread_rng()).to_string();

    debug!("account id generated: {}", account_id);

    for tag in retrieve_tags_from_string(&mut connection, tags).await? {
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
            r"INSERT INTO scores (id, tag_id, score) VALUES (?, ?, ?)",
            account_id,
            tag,
            score,
        )
        .execute(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to insert a tag score into the db",
            )
        })?;
    }

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
        r"INSERT INTO accounts (account_id, feed, style_id) VALUES (?, ?, null)",
        account_id,
        feed
    )
    .execute(&mut *connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to insert a new account into the db",
        )
    })?;

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
        Redirect::to("/welcome"),
    ))
}

pub async fn get_logout(
    cookies: Option<TypedHeader<Cookie>>,
    Extension(route_configuration): Extension<RouteConfiguration>,
) -> Response<UnsyncBoxBody<Bytes, axum::Error>> {
    trace!("logout requested, cookies: {:?}", cookies);

    coz_progress!();

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

pub async fn get_tags(
    Extension(style_id): Extension<model::StyleId>,
    Extension(sqlite): Extension<SqlitePool>,
    Query(model::Tags { after }): Query<model::Tags>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    coz_progress!();

    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    let tags = if let Some(after) = after {
        sqlx::query_as!(
            model::TagRow,
            r#"SELECT name as "name!", tag_id as "id!" FROM tags WHERE tag_id > ? ORDER BY tag_id LIMIT 100"#,
            after
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to query the tags",
            )
        })?
    } else {
        sqlx::query_as!(
            model::TagRow,
            r#"SELECT name as "name!", tag_id as "id!" FROM tags ORDER BY tag_id LIMIT 100"#
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to query the tags",
            )
        })?
    };

    Ok((
        [("Content-Type", "application/xhtml+xml")],
        templates::Tags {
            style_id,
            after: if tags.len() == 100 {
                tags.last().map(|t| t.id.clone())
            } else {
                None
            },
            tags,
        },
    ))
}

pub async fn get_post(Extension(style_id): Extension<model::StyleId>) -> impl IntoResponse {
    coz_progress!();

    (
        [("Content-Type", "application/xhtml+xml")],
        templates::Post { style_id },
    )
}

pub async fn post_post(
    Extension(sqlite): Extension<SqlitePool>,
    Form(model::PostPost {
        link,
        description,
        tags,
    }): Form<model::PostPost>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("post post-ed, tags: \"{}\"", tags);

    coz_progress!();

    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    let link_id = Ulid::with_source(&mut pcg_thread_rng()).to_string();

    debug!("link id generated: {}", link_id);

    //TODO(superwhiskers): this and the similar loop used in account creation (and likely
    //                     account tag modification) could be factored out
    for tag in retrieve_tags_from_string(&mut connection, tags).await? {
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
            r"INSERT INTO scores (id, tag_id, score) VALUES (?, ?, ?)",
            link_id,
            tag,
            score,
        )
        .execute(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to insert a tag score into the db",
            )
        })?;
    }

    sqlx::query!(
        "INSERT INTO links (link_id, link, description) VALUES (?, ?, ?)",
        link_id,
        link,
        description
    )
    .execute(&mut *connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to insert the link into the db",
        )
    })?;

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

    coz_progress!();

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
    .fetch_optional(&mut *connection)
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
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?
        .count
            == 1
        {
            sqlx::query!(
                "INSERT OR IGNORE INTO seen (account_id, link_id, rated) VALUES (?, ?, false)",
                account_id, link_id
            ).execute(&mut *connection).await.map_err(|_| {
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
    Extension(style_id): Extension<model::StyleId>,
    Extension(sqlite): Extension<SqlitePool>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    coz_progress!();

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
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to query the db for the link",
        )
    })?
    .ok_or((StatusCode::BAD_REQUEST, "the requested link does not exist"))?;

    let tags = sqlx::query_scalar!(r#"SELECT tags.name as "name!" FROM scores INNER JOIN tags ON scores.tag_id = tags.tag_id WHERE scores.id = ?"#, link_id)
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to query the tags for the link",
            )
        })?;

    Ok(templates::EditLink {
        style_id,
        id: link_id,
        description,
        tags: tags.join(","),
    })
}

//TODO(superwhiskers): this needs to be tweaked to rely on voting and to make it lock parts
//                     of the db
pub async fn post_edit_link(
    Extension(sqlite): Extension<SqlitePool>,
    Path(link_id): Path<String>,
    Form(model::PostEditLink { description, tags }): Form<model::PostEditLink>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    coz_progress!();

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
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?
    .count
        == 1
    {
        let tags_owned = retrieve_tags_from_string(&mut connection, tags).await?;
        let tags = tags_owned
            .iter()
            .map(|t| t.as_str())
            .collect::<HashSet<_>>();

        let old_tags_owned = sqlx::query_scalar!(
            r#"SELECT tag_id as "tag_id!" FROM scores WHERE id = ?"#,
            link_id
        )
        .fetch_all(&mut *connection)
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

        sqlx::query!(
            r"UPDATE links SET description = ? WHERE link_id = ?",
            description,
            link_id
        )
        .execute(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to update the link's information",
            )
        })?;

        for tag in &old_tags - &tags {
            sqlx::query!(
                r"DELETE FROM scores WHERE id = ? AND tag_id = ?",
                link_id,
                tag
            )
            .execute(&mut *connection)
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
                r"INSERT INTO scores (id, tag_id, score) VALUES (?, ?, ?)",
                link_id,
                tag,
                score,
            )
            .execute(&mut *connection)
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
    Extension(algorithm_configuration): Extension<AlgorithmConfiguration>,
    Extension(sqlite): Extension<SqlitePool>,
    Extension(lock_map): Extension<&'static LockMap>,
    cookies: Option<TypedHeader<Cookie>>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    rate_link(
        algorithm_configuration,
        sqlite,
        lock_map,
        cookies,
        link_id,
        0.75,
    )
    .await
}

pub async fn get_neutral_link(
    Extension(algorithm_configuration): Extension<AlgorithmConfiguration>,
    Extension(sqlite): Extension<SqlitePool>,
    Extension(lock_map): Extension<&'static LockMap>,
    cookies: Option<TypedHeader<Cookie>>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    rate_link(
        algorithm_configuration,
        sqlite,
        lock_map,
        cookies,
        link_id,
        0.5,
    )
    .await
}

pub async fn get_demote_link(
    Extension(algorithm_configuration): Extension<AlgorithmConfiguration>,
    Extension(sqlite): Extension<SqlitePool>,
    Extension(lock_map): Extension<&'static LockMap>,
    cookies: Option<TypedHeader<Cookie>>,
    Path(link_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    rate_link(
        algorithm_configuration,
        sqlite,
        lock_map,
        cookies,
        link_id,
        0.0,
    )
    .await
}

#[inline(always)]
pub async fn rate_link(
    algorithm_configuration: AlgorithmConfiguration,
    sqlite: SqlitePool,
    lock_map: &'static LockMap,
    cookies: Option<TypedHeader<Cookie>>,
    link_id: String,
    base_outcome: f64,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("link rated: {}, cookies: {:?}", link_id, cookies);

    coz_progress!();

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        debug!("account {} rating link {} with base outcome {}", account_id, link_id, base_outcome);

        let mut connection = sqlite.acquire().await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to acqire a db connection",
            )
        })?;

        if sqlx::query_scalar!(
            r#"SELECT 1 FROM accounts WHERE account_id = ?"#,
            account_id
        )
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to check if an account exists",
            )
        })?
        .is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "the requested account does not exist",
            ));
        }

        if sqlx::query_scalar!(
            r#"SELECT 1 FROM links where link_id = ?"#,
            link_id
        )
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to check if a link exists",
            )
        })?
        .is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "the requested link does not exist",
            ));
        }

        debug!("rating link {} / account {}", link_id, account_id);

        let _user_tag_lock = lock_map.lock(account_id).ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "a lock is currently held on your account's tag information, try again in a few seconds",
        ))?;

        let _link_tag_lock = lock_map.lock(link_id.as_str()).ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "a lock is currently held on the link's tag information, try again in a few seconds",
        ))?;

        //TODO(superwhiskers): same thing mentioned in src/feed.rs, but we should
        //                     additionally consider making this a function at this point
        let user_scores = sqlx::query!(
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
        .collect::<Result<HashMap<String, model::Score>, _>>()?;

        let link_scores = sqlx::query!(
            r#"SELECT tag_id as "tag_id!", score as "score!" FROM scores WHERE id = ?"#,
            link_id
        )
        .fetch_all(&mut *connection)
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
                .map(|score| (tag.tag_id, score))
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
            let (user_score, link_score): (ScaledRatingData, ScaledRatingData) =
                (
                    ScaledRatingWrapper(user_score_data.score).into(),
                    ScaledRatingWrapper(link_score_data.score).into(),
                );
            let comparative_volatility = user_score.partial_cmp(&link_score).ok_or((
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

                let tweaked_outcome = base_outcome + (if base_outcome == 0.0 { 0.75 } else { 0.25 } * percent_overlap).max(0.0);
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

            debug!("link outcome: {}, user outcome: {}", link_outcome, user_outcome);

            user_score_data.result_queue.push(ScaledPlayerResult::new(link_score_data.score, user_outcome));
            link_score_data.result_queue.push(ScaledPlayerResult::new(user_score_data.score, link_outcome));

            util::decay_score(&algorithm_configuration, &mut user_score_data, 1)?;
            util::decay_score(&algorithm_configuration, &mut link_score_data, 12)?;

            let user_score_bin = rmp_serde::to_vec(&user_score_data).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to convert data to messagepack",
                )
            })?;

            sqlx::query!(
                "UPDATE scores SET score = ? WHERE id = ? AND tag_id = ?",
                user_score_bin,
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

            let link_score_bin = rmp_serde::to_vec(&link_score_data).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to convert data to messagepack",
                )
            })?;

            sqlx::query!(
                "UPDATE scores SET score = ? WHERE id = ? AND tag_id = ?",
                link_score_bin,
                link_id,
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

            // someone's going to try to rate before viewing. this handles that edge case
            sqlx::query!(
                "INSERT INTO seen (account_id, link_id, rated) VALUES (?, ?, true) ON CONFLICT (account_id, link_id) DO UPDATE SET rated = true",
                account_id,
                link_id
            )
            .execute(&mut *connection)
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

pub async fn get_profile_tags(
    Extension(style_id): Extension<model::StyleId>,
    Extension(sqlite): Extension<SqlitePool>,
    cookies: Option<TypedHeader<Cookie>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("profile tag information requested, cookies: {:?}", cookies);

    coz_progress!();

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        debug!("account {} requesting profile tag information", account_id);

        let mut connection = sqlite.acquire().await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to acquire a db connection",
            )
        })?;

        if sqlx::query_scalar!(
            r#"SELECT 1 FROM accounts WHERE account_id = ?"#,
            account_id
        )
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to check if an account exists",
            )
        })?
        .is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "the requested account does not exist",
            ));
        }

        let tags = sqlx::query!(
            r#"SELECT tags.name as "name!", scores.score as "score!" FROM scores INNER JOIN tags ON scores.tag_id = tags.tag_id WHERE scores.id = ?"#,
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
                .map(|score: model::Score| <ScaledRatingWrapper as Into<ScaledRatingData>>::into(ScaledRatingWrapper(score.score)).to_string())
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to deserialize the score data for a tag",
                    )
                })
                .map(|score| {
                    templates::Tag {
                        name: tag.name,
                        score,
                    }
                })
        }).collect::<Result<Vec<templates::Tag>, _>>()?;

        return Ok((
            [("Content-Type", "application/xhtml+xml")],
            templates::TagScores {
                style_id,
                id: account_id.to_string(),
                tags,
            }
        ));
    }

    Err((StatusCode::BAD_REQUEST, "you are not logged in"))
}

//TODO(superwhiskers): decouple account ids from the id used to log in
pub async fn get_profile(
    Extension(style_id): Extension<model::StyleId>,
    Extension(sqlite): Extension<SqlitePool>,
    cookies: Option<TypedHeader<Cookie>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("profile requested for account, cookies: {:?}", cookies);

    coz_progress!();

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        debug!("account {} requesting profile information", account_id);

        let mut connection = sqlite.acquire().await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to acquire a db connection",
            )
        })?;

        let tags = sqlx::query_scalar!(
            r#"SELECT tags.name as "name!" FROM scores INNER JOIN tags ON scores.tag_id = tags.tag_id WHERE scores.id = ?"#,
            account_id
        )
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to query the account's tags from the db",
            )
        })?;

        Ok((
            [("Content-Type", "application/xhtml+xml")],
            templates::Profile {
                style_id,
                profile: templates::ProfileInformation {
                    id: account_id.to_string(),
                    tags: tags.iter().map(|tag| tag.as_str()).intersperse(",").collect::<String>(),
                },
            }
        ))
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "you are not logged in",
        ))
    }
}

pub async fn post_profile(
    Extension(style_id): Extension<model::StyleId>,
    Extension(sqlite): Extension<SqlitePool>,
    Extension(lock_map): Extension<&'static LockMap>,
    cookies: Option<TypedHeader<Cookie>>,
    Form(model::PostProfile {
        refresh_account_id,
        tags,
        new_style_id,
    }): Form<model::PostProfile>,
) -> impl IntoResponse {
    trace!(
        "profile post-ed, tags: \"{}\", refresh_account_id: {}, cookies: {:?}",
        tags,
        refresh_account_id,
        cookies
    );

    coz_progress!();

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        debug!(
            "account {} making modifications to their profile, tags: \"{}\", refresh_account_id: {}",
            account_id,
            tags,
            refresh_account_id,
        );

        let mut connection = sqlite.acquire().await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to acquire a db connection",
            )
        })?;

        if sqlx::query_scalar!(
            "SELECT 1 FROM accounts WHERE account_id = ?",
            account_id
        )
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to check if an account exists",
            )
        })?
        .is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "the requested account does not exist",
            ));
        }

        if new_style_id != style_id.0.as_deref().unwrap_or("") {
            if new_style_id.is_empty() {
                sqlx::query!(
                    "UPDATE accounts SET style_id = null WHERE account_id = ?",
                    account_id
                )
                .execute(&mut *connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to update an account",
                    )
                })?;
            } else if sqlx::query_scalar!(
                "SELECT 1 FROM styles WHERE style_id = ?",
                new_style_id
            )
            .fetch_optional(&mut *connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to check if a style exists",
                )
            })?
            .is_some() {
                sqlx::query!(
                    "UPDATE accounts SET style_id = ? WHERE account_id = ?",
                    new_style_id,
                    account_id
                )
                .execute(&mut *connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to update an account",
                    )
                })?;
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "invalid style id",
                ));
            }
        }

        let tags_owned = retrieve_tags_from_string(&mut connection, tags).await?;
        let tags = tags_owned
            .iter()
            .map(|t| t.as_str())
            .collect::<HashSet<_>>();

        let _tag_lock = lock_map.lock(account_id).ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "a lock is currently held on your account's tag information. try again in a few seconds",
        ))?;

        let old_tags_owned = sqlx::query_scalar!(
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
        })?;
        let old_tags = old_tags_owned
            .iter()
            .map(|t| t.as_str())
            .collect::<HashSet<_>>();

        for tag in &old_tags - &tags {
            sqlx::query!(r"DELETE FROM scores WHERE id = ? AND tag_id = ?", account_id, tag)
                .execute(&mut *connection)
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
                r"INSERT INTO scores (id, tag_id, score) VALUES (?, ?, ?)",
                account_id,
                tag,
                score
            )
            .execute(&mut *connection)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to insert a tag score into the db",
                )
            })?;
        }
    }

    Ok(Redirect::to("/"))
}

pub async fn get_feed_xml(
    Extension(sqlite): Extension<SqlitePool>,
    Extension(algorithm_configuration): Extension<AlgorithmConfiguration>,
    Extension(lock_map): Extension<&'static LockMap>,
    Extension(http_configuration): Extension<HttpConfiguration>,
    Query(model::FeedXml { account_id }): Query<model::FeedXml>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("feed.xml requested, account id: {}", &account_id);

    coz_progress!();

    //TODO(superwhiskers): factor out this and the body of the get_index function into a
    //                     separate function or something
    if !account_id.is_empty() {
        trace!("preparing feed.xml for {}", &account_id);

        let mut connection = sqlite.acquire().await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to acquire a db connection",
            )
        })?;

        if let Some(account) = sqlx::query!(
            r#"SELECT feed as "feed!", style_id as "style_id?" FROM accounts WHERE account_id = ?"#,
            account_id
        )
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to query the db"))?
        {
            let mut feed = rmp_serde::from_slice::<model::Feed>(&account.feed).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unable to deserialize the feed",
                )
            })?;

            debug!("deserialized feed for {}: {:?}", &account_id, feed);

            if (SystemTime::UNIX_EPOCH + Duration::from_secs(feed.refreshed))
                .elapsed()
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to calculate the amount of time that has passed since the last time the feed was refreshed",
                    )
                })?
                    > algorithm_configuration.feed_refresh_period {
                trace!("generating new feed for {}", &account_id);

                trace!("locking the account's tags");

                let _tag_lock = lock_map.lock(&account_id).ok_or((
                    StatusCode::SERVICE_UNAVAILABLE,
                    "a lock is currently held on your account's tag information. try again in a few seconds",
                ))?;

                feed = model::Feed {
                    links: feed::generate_feed(&algorithm_configuration, sqlite.acquire().await.map_err(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "unable to acquire a db connection",
                        )
                    })?, &account_id).await?,
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

                debug!("new feed for {}: {:?}", &account_id, feed);

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
                .execute(&mut *connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to update the feed",
                    )
                })?;
            }

            let mut links = Vec::with_capacity(feed.links.len());
            for (link_id, _) in feed.links {
                let description = sqlx::query!(
                    r#"SELECT description as "description!" FROM links WHERE link_id = ?"#,
                    link_id,
                )
                .fetch_one(&mut *connection)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "unable to query for a link's information",
                    )
                })?
                .description;

                links.push(
                    rss::ItemBuilder::default()
                        .title(description)
                        .description(
                            templates::FeedItem {
                                style_id: model::StyleId(account.style_id.clone()),
                                flock_host: http_configuration.host.clone(),
                                link_id: link_id.clone(),
                            }
                            .render()
                            .map_err(|_| {
                                (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "unable to render an rss feed item's description",
                                )
                            })?,
                        )
                        .link(format!(
                            "{}/links/{}",
                            http_configuration.host.clone(),
                            link_id.clone()
                        ))
                        .guid(
                            rss::GuidBuilder::default()
                                .value(link_id)
                                .permalink(false)
                                .build(),
                        )
                        .build(),
                );
            }

            trace!("sending response to {}", &account_id);

            Ok((
                [("Content-Type", "application/rss+xml")],
                rss::ChannelBuilder::default()
                    .title("flock")
                    .description(format!("the flock feed for account {}", account_id))
                    .link(http_configuration.host)
                    .docs("https://www.rssboard.org/rss-specification".to_string())
                    .items(links)
                    .build()
                    .to_string(),
            ))
        } else {
            Err((
                StatusCode::BAD_REQUEST,
                "the requested account does not exist",
            ))
        }
    } else {
        trace!("an attempt was made to access an rss feed without an account");

        Err((
            StatusCode::BAD_REQUEST,
            "in order to use the rss feed, you must provide an account id",
        ))
    }
}

pub async fn get_style(
    Extension(sqlite): Extension<SqlitePool>,
    Path(style_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("style requested, style id: {}", &style_id);

    coz_progress!();

    let mut connection = sqlite.acquire().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to acquire a db connection",
        )
    })?;

    let style = sqlx::query_scalar!(
        r#"SELECT style as "style!" FROM styles WHERE style_id = ?"#,
        style_id
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "unable to query the db for the style",
        )
    })?
    .ok_or((
        StatusCode::BAD_REQUEST,
        "the requested style does not exist",
    ))?;

    Ok(([("Content-Type", "text/css")], style))
}

pub async fn get_welcome(
    Extension(style_id): Extension<model::StyleId>,
    Extension(algorithm_configuration): Extension<AlgorithmConfiguration>,
    cookies: Option<TypedHeader<Cookie>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("welcome requested, cookies: {:?}", cookies);

    coz_progress!();

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        Ok((
            [("Content-Type", "application/xhtml+xml")],
            templates::Welcome {
                style_id,
                account_id: account_id.to_string(),
                algorithm_feed_refresh_period: algorithm_configuration.feed_refresh_period.into(),
            }
        ))
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "you are not logged in",
        ))
    }
}

pub async fn get_post_style(
    Extension(style_id): Extension<model::StyleId>,
) -> impl IntoResponse {
    trace!("post-style requested");

    coz_progress!();

    (
        [("Content-Type", "application/xhtml+xml")],
        templates::PostStyle { style_id },
    )
}

pub async fn post_post_style(
    Extension(sqlite): Extension<SqlitePool>,
    Extension(style_id): Extension<model::StyleId>,
    cookies: Option<TypedHeader<Cookie>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    trace!("post-style posted, cookies: {:?}", cookies);

    coz_progress!();

    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        let mut connection = sqlite.acquire().await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to acquire a db connection",
            )
        })?;

        if sqlx::query_scalar!(
            r#"SELECT 1 FROM accounts WHERE account_id = ?"#,
            account_id
        )
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to check if an account exists",
            )
        })?
        .is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "the requested account does not exist",
            ));
        }

        let (name, stylesheet) = loop {
            if let Some(field) = multipart
                .next_field()
                .await
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "unable to read multipart form data",
                    )
                })? {
                if field.name() == Some("stylesheet")
                   && field.content_type() == Some("text/css") {
                    break (
                        field
                            .file_name()
                            .map(|n| n.trim_end_matches(".css").to_string())
                            .unwrap_or_else(|| "unnamed".to_string()),
                        field
                            .text()
                            .await
                            .map_err(|_| {
                                (
                                    StatusCode::BAD_REQUEST,
                                    "unable to read multipart form data",
                                )
                            })?
                    );
                }
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "no useful multipart form data was found",
                ));
            }
        };

        let new_style_id = Ulid::with_source(&mut pcg_thread_rng()).to_string();

        debug!("style id generated: {}", new_style_id);

        sqlx::query!(
            "INSERT INTO styles (style_id, name, creator, style) VALUES (?, ?, ?, ?)",
            new_style_id,
            name,
            account_id,
            stylesheet
        )
        .execute(&mut *connection)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "unable to insert the stylesheet into the db",
            )
        })?;

        Ok((
            [("Content-Type", "application/xhtml+xml")],
            templates::PostStyleResult {
                style_id,
                created_style_id: new_style_id,
            },
        ))
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "you are not logged in",
        ))
    }
}
