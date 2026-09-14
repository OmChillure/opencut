create table if not exists media_analysis (
    media_id uuid primary key references media (id) on delete cascade,
    look text not null default 'unknown',
    motion real not null default 0,
    scenes integer not null default 0,
    brightness real not null default 0,
    colorful boolean not null default false,
    has_video boolean not null default false,
    has_audio boolean not null default false,
    raw jsonb,
    updated_at timestamptz not null default now()
);
