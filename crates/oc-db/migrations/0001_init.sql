create extension if not exists pgcrypto;

create table if not exists projects (
    id uuid primary key default gen_random_uuid(),
    name text not null,
    timeline jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create table if not exists media (
    id uuid primary key default gen_random_uuid(),
    project_id uuid not null references projects (id) on delete cascade,
    r2_key text not null,
    filename text not null,
    content_type text not null,
    byte_size bigint,
    duration_ticks bigint,
    width integer,
    height integer,
    status text not null default 'uploading',
    created_at timestamptz not null default now()
);

create index if not exists media_project_idx on media (project_id);

create table if not exists transcripts (
    id uuid primary key default gen_random_uuid(),
    media_id uuid not null references media (id) on delete cascade,
    language text,
    full_text text not null,
    raw jsonb,
    created_at timestamptz not null default now()
);

create table if not exists cues (
    id uuid primary key default gen_random_uuid(),
    transcript_id uuid not null references transcripts (id) on delete cascade,
    start_ticks bigint not null,
    end_ticks bigint not null,
    text text not null,
    speaker text,
    idx integer not null
);

create index if not exists cues_transcript_idx on cues (transcript_id);

create table if not exists jobs (
    id uuid primary key default gen_random_uuid(),
    kind text not null,
    status text not null default 'queued',
    payload jsonb not null default '{}'::jsonb,
    error text,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists jobs_status_idx on jobs (status, created_at);
