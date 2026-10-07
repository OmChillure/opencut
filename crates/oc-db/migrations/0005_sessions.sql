create table if not exists users (
    email text primary key,
    password_hash text not null,
    created_at timestamptz not null default now()
);

create table if not exists sessions (
    token text primary key,
    email text not null references users (email) on delete cascade,
    created_at timestamptz not null default now()
);

create index if not exists sessions_email_idx on sessions (email);
