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

#![allow(clippy::cognitive_complexity)]
#![warn(clippy::cargo_common_metadata)]
#![warn(clippy::dbg_macro)]
#![warn(clippy::explicit_deref_methods)]
#![warn(clippy::filetype_is_file)]
#![warn(clippy::imprecise_flops)]
#![warn(clippy::large_stack_arrays)]
#![warn(clippy::todo)]
#![warn(clippy::unimplemented)]
#![deny(clippy::await_holding_lock)]
#![deny(clippy::cast_lossless)]
#![deny(clippy::clone_on_ref_ptr)]
#![deny(clippy::doc_markdown)]
#![deny(clippy::empty_enum)]
#![deny(clippy::enum_glob_use)]
#![deny(clippy::exit)]
#![deny(clippy::explicit_into_iter_loop)]
#![deny(clippy::explicit_iter_loop)]
#![deny(clippy::fallible_impl_from)]
#![deny(clippy::inefficient_to_string)]
#![deny(clippy::large_digit_groups)]
#![deny(clippy::wildcard_dependencies)]
#![deny(clippy::wildcard_imports)]
#![deny(clippy::unused_self)]
#![deny(clippy::single_match_else)]
#![deny(clippy::option_option)]
#![deny(clippy::mut_mut)]
#![feature(lazy_cell)]
#![feature(let_chains)]
#![feature(int_roundings)]
#![feature(iter_intersperse)]
#![feature(map_try_insert)]

mod configuration;
mod feed;
mod locks;
mod model;
mod rand;
mod routes;
mod templates;
mod util;

use anyhow::Context;
use axum::{
    extract::Extension,
    http::{header, HeaderValue},
    middleware,
    routing::get,
    Router,
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    ConnectOptions,
};
use tower_http::{set_header::SetResponseHeaderLayer, trace::TraceLayer};
use tracing::{info, log::LevelFilter, trace, warn};
use tracing_log::LogTracer;
use tracing_subscriber::FmtSubscriber;

use crate::{configuration::Configuration, locks::LockMap};

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "dhat")]
    let _dhat_profiler = dhat::Profiler::new_heap();

    let config = Configuration::new().context("failed to load the configuration")?;

    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(&config.general.log_filter)
            .finish(),
    )?;

    LogTracer::init()?;

    trace!("opening a db connection pool");

    let mut sqlite_pool_options = SqlitePoolOptions::new();

    if let Some(min_connections) = config.sqlite.min_connections {
        sqlite_pool_options = sqlite_pool_options.min_connections(min_connections);
    }

    if let Some(max_connections) = config.sqlite.max_connections {
        sqlite_pool_options = sqlite_pool_options.max_connections(max_connections);
    }

    let sqlite = sqlite_pool_options
        .connect_with({
            SqliteConnectOptions::new()
                .filename(&config.sqlite.path)
                .log_statements(LevelFilter::Debug)
                //TODO(superwhiskers): we don't apply the schema yet
                //.create_if_missing(config.sqlite.create_if_missing)
                // performance
                // (from https://phiresky.github.io/blog/2020/sqlite-performance-tuning/)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal) // safe with a write-ahead-log
                .optimize_on_close(true, None)
        })
        .await
        .context("unable to open a db connection pool")?;

    let lock_map = LockMap::new();

    trace!("initializing the server");

    let app = Router::new()
        .route("/", get(routes::get_index))
        .route("/login", get(routes::get_login).post(routes::post_login))
        .route("/signup", get(routes::get_signup).post(routes::post_signup))
        .route("/logout", get(routes::get_logout))
        .route("/post", get(routes::get_post).post(routes::post_post))
        .route("/tags", get(routes::get_tags))
        .route("/welcome", get(routes::get_welcome))
        .nest(
            "/profile",
            Router::new()
                .route("/", get(routes::get_profile).post(routes::post_profile))
                .route("/tags", get(routes::get_profile_tags)),
        )
        .nest(
            "/links/:link_id",
            Router::new()
                .route("/", get(routes::link))
                .route("/promote", get(routes::get_promote_link))
                .route("/neutral", get(routes::get_neutral_link))
                .route("/demote", get(routes::get_demote_link)), // .route("/edit", get(routes::get_edit_link).post(routes::post_edit_link)),
        )
        .layer(middleware::from_fn(util::apply_style_id_extension))
        .route("/feed.xml", get(routes::get_feed_xml))
        .route("/styles/:style_id", get(routes::get_style))
        .layer(Extension(sqlite.clone()))
        .layer(Extension(config.routes))
        .layer(Extension(config.algorithm))
        .layer(Extension(config.http.clone()))
        .layer(Extension(lock_map))
        .layer(SetResponseHeaderLayer::appending(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'none'; style-src-elem 'self'; font-src data:; img-src data: 'self'",
            ),
        ))
        .layer(TraceLayer::new_for_http());

    //TODO(superwhiskers): add https support (this isn't necessary in production, though, as
    //                     we use nginx)
    info!("listening at http://{}", &config.http.address);

    axum::Server::bind(&config.http.address)
        .serve(app.into_make_service())
        .with_graceful_shutdown(util::signal_handler())
        .await?;

    info!("stopping the server");

    sqlite.close().await;

    Ok(())
}
