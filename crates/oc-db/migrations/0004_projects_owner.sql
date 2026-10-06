alter table projects add column if not exists owner_email text;

create index if not exists projects_owner_email_idx on projects (owner_email);
