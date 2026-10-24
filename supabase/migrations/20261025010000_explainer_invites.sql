-- Invite people to the explainer by email, before or after they sign up.
-- An account may use the explainer when its profile flag is set or its email is invited.

create table if not exists public.explainer_invites (
  email      text primary key check (email = lower(email)),
  created_at timestamptz not null default now()
);
-- Managed from the SQL editor only: no policies, so the API cannot read or change it.
alter table public.explainer_invites enable row level security;

create or replace function public.can_use_explainer()
returns boolean
language sql stable security definer
set search_path = ''
as $$
  select coalesce(
    (select p.can_use_explainer from public.profiles p where p.id = auth.uid()),
    false
  )
  or exists (
    select 1
      from auth.users u
      join public.explainer_invites i on i.email = lower(u.email)
     where u.id = auth.uid()
       and u.email_confirmed_at is not null
  )
$$;

grant execute on function public.can_use_explainer() to authenticated;

insert into public.explainer_invites (email) values
  ('elyshasoffiea@gmail.com'),
  ('elysharksoffiea@gmail.com')
on conflict do nothing;
