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

use askama_axum::Template;

use crate::model;

#[derive(Template)]
#[template(path = "index.html")]
pub struct Index {
    pub style_id: model::StyleId,
    pub account: Option<Account>,
}

pub struct Account {
    pub id: String,
    pub links: Vec<Link>,
}

pub struct Link {
    pub id: String,
    pub description: String,
    pub rated: bool,
    pub rating: Option<String>,
    pub visited: bool,
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct Login {
    pub style_id: model::StyleId,
    pub redirect_to: Option<String>,
}

#[derive(Template)]
#[template(path = "signup.html")]
pub struct Signup {
    pub style_id: model::StyleId,
}

#[derive(Template)]
#[template(path = "tags.html")]
pub struct Tags {
    pub style_id: model::StyleId,
    pub tags: Vec<model::TagRow>,
    pub after: Option<String>,
}

#[derive(Template)]
#[template(path = "tag-scores.html")]
pub struct TagScores {
    pub style_id: model::StyleId,
    pub id: String,
    pub tags: Vec<Tag>,
}

pub struct Tag {
    pub name: String,
    pub score: String,
}

#[derive(Template)]
#[template(path = "post.html")]
pub struct Post {
    pub style_id: model::StyleId,
}

#[derive(Template)]
#[template(path = "edit-link.html")]
pub struct EditLink {
    pub style_id: model::StyleId,
    pub id: String,
    pub description: String,
    pub tags: String,
}

#[derive(Template)]
#[template(path = "profile.html")]
pub struct Profile {
    pub style_id: model::StyleId,
    pub profile: ProfileInformation,
}

pub struct ProfileInformation {
    pub id: String,
    pub tags: String,
}

#[derive(Template)]
#[template(path = "feed-item.html")]
pub struct FeedItem {
    pub style_id: model::StyleId,
    pub flock_host: String,
    pub link_id: String,
}

#[derive(Template)]
#[template(path = "welcome.html")]
pub struct Welcome {
    pub style_id: model::StyleId,
    pub account_id: String,
    pub algorithm_feed_refresh_period: humantime::Duration,
}

#[derive(Template)]
#[template(path = "post-style.html")]
pub struct PostStyle {
    pub style_id: model::StyleId,
}

#[derive(Template)]
#[template(path = "post-style-result.html")]
pub struct PostStyleResult {
    pub style_id: model::StyleId,
    pub created_style_id: String,
}

mod filters {
    pub fn urlencoded(s: impl std::fmt::Display) -> ::askama::Result<String> {
        Ok(urlencoding::encode(&s.to_string()).to_string())
    }
}
