create table if not exists chats (
    id uuid primary key default gen_random_uuid(),
    project_id uuid not null references projects (id) on delete cascade,
    user_email text not null,
    title text not null default 'New chat',
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists chats_owner_idx on chats (project_id, user_email, updated_at desc);

create table if not exists chat_messages (
    id uuid primary key default gen_random_uuid(),
    chat_id uuid not null references chats (id) on delete cascade,
    idx integer not null,
    role text not null,
    text text not null default '',
    tool_id text not null default '',
    tool_name text not null default '',
    tool_status text not null default '',
    tool_args text not null default '',
    tool_result text not null default '',
    created_at timestamptz not null default now(),
    unique (chat_id, idx)
);

create index if not exists chat_messages_chat_idx on chat_messages (chat_id, idx);
