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
use instant_glicko_2::{algorithm as glicko_2, ScaledRating};
use regex::Regex;
use sqlx::SqlitePool;
use std::{
    collections::HashSet,
    sync::LazyLock,
    time::{Duration, SystemTime},
};
use tracing::debug;
use ulid::Ulid;

use crate::{feed, model, templates};

static TAG_DELIMITER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*,\s*").expect("unable to compile a regex"));

static TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[[:alnum:]_\-]+").expect("unable to compile a regex"));

pub fn string_to_tags(tags: &str) -> Result<HashSet<&'_ str>, (StatusCode, &'static str)> {
    let tags = TAG_DELIMITER_REGEX.split(tags).collect::<HashSet<_>>();

    if !tags.iter().all(|tag| TAG_REGEX.is_match(tag)) {
        return Err((StatusCode::BAD_REQUEST, "the provided tags are invalid"));
    }

    Ok(tags)
}

pub async fn index(
    Extension(sqlite): Extension<SqlitePool>,
    cookies: Option<TypedHeader<Cookie>>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
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
            //TODO(superwhiskers): check for the feed age & update the feed using the algorithm you've described prior if it is too old (>a day)
            let mut feed = rmp_serde::from_slice::<model::Feed>(&feed).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to deserialize the feed"))?;

            if (SystemTime::UNIX_EPOCH + Duration::from_secs(feed.refreshed))
                .elapsed()
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "unable to calculate the amount of time that has passed since the last time the feed was refreshed"))?
                .as_secs()
                    > (60 * 60 * 24) {
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

                    sqlx::query!(r"UPDATE ")
                        //TODO(superwhiskers): finish
                }

            Ok((
                [("Content-Type", "application/xhtml+xml")],
                templates::Index {
                    account: Some(templates::Account {
                        id: account_id.to_string(),
                        links: vec![],
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
    Form(model::PostLogin { account_id }): Form<model::PostLogin>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
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
            AppendHeaders([
                // we should have a toggle in the config to add `Secure;` here or something for if
                // https is set up
                (
                    SET_COOKIE,
                    format!(
                        "flock.id={}; SameSite=Lax; Expires={}; Max-Age=172800; HttpOnly",
                        account_id,
                        // lmao old browsers but why the heck not, it's hardly any effort
                        httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(172800))
                    ),
                ),
            ]),
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
    Form(model::PostSignup { tags }): Form<model::PostSignup>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let tags = string_to_tags(&tags)?;

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

    let account_id = Ulid::new().to_string();
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
            ratings_since_last_period: 0,
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
        AppendHeaders([
            // we should have a toggle in the config to add `Secure;` here or something for if
            // https is set up
            (
                SET_COOKIE,
                format!(
                    "flock.id={}; SameSite=Lax; Expires={}; Max-Age=172800; HttpOnly",
                    account_id,
                    // lmao old browsers but why the heck not, it's hardly any effort
                    httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(172800))
                ),
            ),
        ]),
        Redirect::to("/"),
    ))
}

pub async fn logout(
    cookies: Option<TypedHeader<Cookie>>,
) -> Response<UnsyncBoxBody<Bytes, axum::Error>> {
    if let Some(cookies) = cookies
        && let Some(account_id) = cookies.get("flock.id") {
        (
            AppendHeaders([
                // we should have a toggle in the config to add `Secure;` here or something for if
                // https is set up
                (
                    SET_COOKIE,
                    format!(
                        "flock.id={}; SameSite=Lax; Expires={}; Max-Age=0; HttpOnly",
                        account_id,
                        // lmao old browsers but why the heck not, it's hardly any effort
                        httpdate::fmt_http_date(SystemTime::UNIX_EPOCH)
                    ),
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
        tags,
    }): Form<model::PostPost>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let tags = string_to_tags(&tags)?;

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

    let link_id = Ulid::new().to_string();

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
            ratings_since_last_period: 0,
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
                "INSERT OR IGNORE INTO seen (account_id, link_id, rated) VALUES (?, ?, ?)",
                account_id, link_id, false
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
        description: description,
        tags: tags.join(","),
    })
}

pub async fn post_edit_link(
    Extension(sqlite): Extension<SqlitePool>,
    Path(link_id): Path<String>,
    Form(model::PostEditLink { description, tags }): Form<model::PostEditLink>,
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
        let tags = string_to_tags(&tags)?;
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
                ratings_since_last_period: 0,
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
