CREATE TABLE IF NOT EXISTS accounts (
    account_id TEXT PRIMARY KEY,
    feed BLOB
);

CREATE TABLE IF NOT EXISTS links (
    --TODO: allow for links to be purely textual
    link_id TEXT PRIMARY KEY,
    link TEXT,
    description TEXT
);

CREATE TABLE IF NOT EXISTS scores (
    id TEXT,
    tag TEXT,
    score BLOB,
    PRIMARY KEY (id, tag)
);

CREATE TABLE IF NOT EXISTS seen (
    account_id TEXT,
    link_id TEXT,
    rated BOOLEAN,
    PRIMARY KEY (account_id, link_id)
);

CREATE TABLE IF NOT EXISTS tags (tag TEXT PRIMARY KEY);
