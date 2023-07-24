CREATE TABLE IF NOT EXISTS accounts (
    account_id TEXT NOT NULL PRIMARY KEY,
    feed BLOB NOT NULL,
    style_id TEXT
);

CREATE TABLE IF NOT EXISTS links (
    --TODO: allow for links to be purely textual
    link_id TEXT NOT NULL PRIMARY KEY,
    link TEXT NOT NULL,
    description TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS scores (
    id TEXT NOT NULL,
    tag_id TEXT NOT NULL,
    score BLOB NOT NULL,
    PRIMARY KEY (id, tag_id)
);

CREATE TABLE IF NOT EXISTS seen (
    account_id TEXT NOT NULL,
    link_id TEXT NOT NULL,
    rated BOOLEAN NOT NULL,
    PRIMARY KEY (account_id, link_id)
);

CREATE TABLE IF NOT EXISTS styles (
    style_id TEXT NOT NULL PRIMARY KEY,
    creator TEXT NOT NULL,
    style TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tags (
    tag_id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
