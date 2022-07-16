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
#![feature(option_result_contains)]
#![feature(once_cell)]
#![feature(let_chains)]

mod configuration;
mod model;
mod routes;
mod templates;
mod util;
mod feed;

use anyhow::Context;
use axum::{extract::Extension, routing::get, Router};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tracing::{debug, error, info, trace, warn};
use tracing_log::LogTracer;
use tracing_subscriber::FmtSubscriber;

use crate::configuration::Configuration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Configuration::new().context("failed to load the configuration")?;

    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(&config.general.logger)
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
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&config.sqlite.path)
                .create_if_missing(config.sqlite.create_if_missing),
        )
        .await
        .context("unable to open a db connection pool")?;

    trace!("initializing the server");

    let app = Router::new()
        .route("/", get(routes::index))
        .route("/login", get(routes::get_login).post(routes::post_login))
        .route("/signup", get(routes::get_signup).post(routes::post_signup))
        .route("/logout", get(routes::logout))
        .route("/post", get(routes::get_post).post(routes::post_post))
        .route("/tags", get(routes::tags))
        .nest(
            "/links/:link_id",
            Router::new()
                .route("/", get(routes::link))
                .route("/like", get(|| async {}))
                .route("/dislike", get(|| async {}))
                .route(
                    "/edit",
                    get(routes::get_edit_link).post(routes::post_edit_link),
                ),
        )
        .layer(Extension(sqlite.clone()));

    //TODO(superwhiskers): add https support
    info!("listening at http://{}", &config.http.address);

    axum::Server::bind(&config.http.address)
        .serve(app.into_make_service())
        .with_graceful_shutdown(util::signal_handler(sqlite))
        .await?;

    Ok(())
}
