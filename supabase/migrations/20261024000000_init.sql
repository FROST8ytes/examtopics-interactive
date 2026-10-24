-- ExamTopics Interactive — initial schema.
-- Auth (email, Google, GitHub, anonymous, identity linking) is handled by Supabase Auth;
-- this file defines the app tables, row level security, storage and the explainer queue.

------------------------------------------------------------------------------
-- Helpers
------------------------------------------------------------------------------

-- True for users that signed in with email / OAuth (not anonymous guests).
create or replace function public.is_permanent_user()
returns boolean
language sql stable
as $$
  select auth.uid() is not null
     and coalesce((auth.jwt() ->> 'is_anonymous')::boolean, false) = false
$$;

------------------------------------------------------------------------------
-- Profiles
------------------------------------------------------------------------------

create table public.profiles (
  id           uuid primary key references auth.users (id) on delete cascade,
  display_name text,
  is_admin     boolean not null default false,
  created_at   timestamptz not null default now()
);

create or replace function public.is_admin()
returns boolean
language sql stable security definer
set search_path = ''
as $$
  select coalesce((select p.is_admin from public.profiles p where p.id = auth.uid()), false)
$$;

create or replace function public.handle_new_user()
returns trigger
language plpgsql security definer
set search_path = ''
as $$
begin
  insert into public.profiles (id, display_name)
  values (
    new.id,
    coalesce(
      new.raw_user_meta_data ->> 'full_name',
      new.raw_user_meta_data ->> 'name',
      new.raw_user_meta_data ->> 'user_name',
      nullif(split_part(coalesce(new.email, ''), '@', 1), '')
    )
  )
  on conflict (id) do nothing;
  return new;
end;
$$;

create trigger on_auth_user_created
  after insert on auth.users
  for each row execute function public.handle_new_user();

alter table public.profiles enable row level security;

create policy "profiles: read own or admin" on public.profiles
  for select to authenticated
  using (id = (select auth.uid()) or (select public.is_admin()));

create policy "profiles: update own" on public.profiles
  for update to authenticated
  using (id = (select auth.uid()))
  with check (id = (select auth.uid()));

-- Users may only change their display name, never is_admin.
revoke update on public.profiles from authenticated;
grant update (display_name) on public.profiles to authenticated;

------------------------------------------------------------------------------
-- Library: certifications → sets → questions
------------------------------------------------------------------------------

create table public.certifications (
  id              uuid primary key default gen_random_uuid(),
  code            text not null,
  name            text not null,
  vendor          text,
  description     text,
  source_filename text,
  question_count  integer not null default 0,
  created_by      uuid default auth.uid() references auth.users (id) on delete set null,
  created_at      timestamptz not null default now()
);

create table public.exam_sets (
  id               uuid primary key default gen_random_uuid(),
  certification_id uuid not null references public.certifications (id) on delete cascade,
  title            text not null,
  topic            integer not null default 1,
  -- Case-study text shared by the questions of this set, if any.
  intro            text,
  position         integer not null,
  question_count   integer not null default 0,
  created_at       timestamptz not null default now()
);
create index exam_sets_certification_idx on public.exam_sets (certification_id, position);

create table public.questions (
  id               uuid primary key default gen_random_uuid(),
  certification_id uuid not null references public.certifications (id) on delete cascade,
  set_id           uuid not null references public.exam_sets (id) on delete cascade,
  position         integer not null,
  number           integer not null,
  topic            integer not null,
  -- Current question (examcore::Question). Editable: fixes to OCR or to a wrong key.
  data             jsonb not null,
  -- The question exactly as parsed from the PDF. Never changes.
  source           jsonb not null,
  parse_warnings   text[] not null default '{}',
  edited_by        uuid references auth.users (id) on delete set null,
  edited_at        timestamptz,
  created_at       timestamptz not null default now(),
  unique (set_id, position)
);
create index questions_certification_idx on public.questions (certification_id);

create or replace function public.can_edit_certification(p_certification_id uuid)
returns boolean
language sql stable security definer
set search_path = ''
as $$
  select public.is_permanent_user() and (
    public.is_admin()
    or exists (
      select 1 from public.certifications c
      where c.id = p_certification_id and c.created_by = auth.uid()
    )
  )
$$;

alter table public.certifications enable row level security;
alter table public.exam_sets enable row level security;
alter table public.questions enable row level security;

-- Every signed-in user (guests included) can study everything in the library.
create policy "certifications: read" on public.certifications
  for select to authenticated using (true);
create policy "certifications: create (permanent users)" on public.certifications
  for insert to authenticated
  with check ((select public.is_permanent_user()) and created_by = (select auth.uid()));
create policy "certifications: update (owner/admin)" on public.certifications
  for update to authenticated
  using ((select public.can_edit_certification(id)))
  with check ((select public.can_edit_certification(id)));
create policy "certifications: delete (owner/admin)" on public.certifications
  for delete to authenticated
  using ((select public.can_edit_certification(id)));

create policy "exam_sets: read" on public.exam_sets
  for select to authenticated using (true);
create policy "exam_sets: write (owner/admin)" on public.exam_sets
  for all to authenticated
  using ((select public.can_edit_certification(certification_id)))
  with check ((select public.can_edit_certification(certification_id)));

create policy "questions: read" on public.questions
  for select to authenticated using (true);
create policy "questions: write (owner/admin)" on public.questions
  for all to authenticated
  using ((select public.can_edit_certification(certification_id)))
  with check ((select public.can_edit_certification(certification_id)));

------------------------------------------------------------------------------
-- Claude explanations + job queue (processed by the `explainer` worker,
-- which runs Claude Code on a Claude subscription).
------------------------------------------------------------------------------

create table public.explanations (
  question_id uuid primary key references public.questions (id) on delete cascade,
  -- examcore::Explanation
  content     jsonb not null,
  model       text,
  created_at  timestamptz not null default now()
);

create table public.ai_jobs (
  id           bigint generated always as identity primary key,
  question_id  uuid not null references public.questions (id) on delete cascade,
  status       text not null default 'pending'
               check (status in ('pending', 'processing', 'done', 'failed')),
  attempts     integer not null default 0,
  error        text,
  requested_by uuid default auth.uid() references auth.users (id) on delete set null,
  created_at   timestamptz not null default now(),
  updated_at   timestamptz not null default now(),
  locked_at    timestamptz
);
create unique index ai_jobs_one_active_per_question
  on public.ai_jobs (question_id) where status in ('pending', 'processing');
create index ai_jobs_pending_idx on public.ai_jobs (created_at) where status = 'pending';

alter table public.explanations enable row level security;
alter table public.ai_jobs enable row level security;

create policy "explanations: read" on public.explanations
  for select to authenticated using (true);
create policy "ai_jobs: read" on public.ai_jobs
  for select to authenticated using (true);
-- No write policies: writes go through the functions below.

-- Queue explanations for questions that don't have one yet.
-- With p_force (owners/admins only) existing explanations are regenerated.
create or replace function public.request_explanations(p_question_ids uuid[], p_force boolean default false)
returns integer
language plpgsql security definer
set search_path = ''
as $$
declare
  v_count integer;
begin
  if auth.uid() is null then
    raise exception 'not authenticated';
  end if;
  if array_length(p_question_ids, 1) > 2000 then
    raise exception 'too many questions in one request';
  end if;

  if p_force then
    delete from public.explanations e
    using public.questions q
    where e.question_id = q.id
      and q.id = any (p_question_ids)
      and public.can_edit_certification(q.certification_id);
  end if;

  insert into public.ai_jobs (question_id, requested_by)
  select q.id, auth.uid()
  from public.questions q
  where q.id = any (p_question_ids)
    and not exists (select 1 from public.explanations e where e.question_id = q.id)
  on conflict do nothing;
  get diagnostics v_count = row_count;
  return v_count;
end;
$$;

-- Worker: atomically claim pending jobs (also re-claims jobs stuck in processing).
create or replace function public.claim_ai_jobs(p_limit integer default 1)
returns setof public.ai_jobs
language sql security definer
set search_path = ''
as $$
  update public.ai_jobs j
     set status = 'processing', locked_at = now(), updated_at = now(), attempts = j.attempts + 1
   where j.id in (
     select id from public.ai_jobs
      where status = 'pending'
         or (status = 'processing' and locked_at < now() - interval '20 minutes')
      order by created_at
      limit greatest(p_limit, 1)
      for update skip locked
   )
  returning j.*;
$$;

-- Worker: store the result. p_kind (optional) replaces the question's kind when it is
-- still an image-only ("visual") question, turning it into dropdowns / drag & drop.
create or replace function public.complete_ai_job(
  p_job_id bigint,
  p_content jsonb,
  p_model text,
  p_kind jsonb default null
)
returns void
language plpgsql security definer
set search_path = ''
as $$
declare
  v_question uuid;
begin
  select question_id into v_question from public.ai_jobs where id = p_job_id;
  if v_question is null then
    raise exception 'job % not found', p_job_id;
  end if;

  insert into public.explanations (question_id, content, model)
  values (v_question, p_content, p_model)
  on conflict (question_id) do update
    set content = excluded.content, model = excluded.model, created_at = now();

  if p_kind is not null then
    update public.questions
       set data = jsonb_set(data, '{kind}', p_kind)
     where id = v_question
       and data -> 'kind' ->> 'type' = 'visual';
  end if;

  update public.ai_jobs
     set status = 'done', error = null, updated_at = now(), locked_at = null
   where id = p_job_id;
end;
$$;

create or replace function public.fail_ai_job(p_job_id bigint, p_error text)
returns void
language sql security definer
set search_path = ''
as $$
  update public.ai_jobs
     set status = case when attempts >= 3 then 'failed' else 'pending' end,
         error = left(p_error, 2000), updated_at = now(), locked_at = null
   where id = p_job_id;
$$;

revoke execute on function public.claim_ai_jobs(integer) from public, anon, authenticated;
revoke execute on function public.complete_ai_job(bigint, jsonb, text, jsonb) from public, anon, authenticated;
revoke execute on function public.fail_ai_job(bigint, text) from public, anon, authenticated;
grant execute on function public.claim_ai_jobs(integer) to service_role;
grant execute on function public.complete_ai_job(bigint, jsonb, text, jsonb) to service_role;
grant execute on function public.fail_ai_job(bigint, text) to service_role;

------------------------------------------------------------------------------
-- Progress
------------------------------------------------------------------------------

create table public.question_progress (
  user_id     uuid not null default auth.uid() references auth.users (id) on delete cascade,
  question_id uuid not null references public.questions (id) on delete cascade,
  set_id      uuid not null references public.exam_sets (id) on delete cascade,
  -- examcore::Response
  response    jsonb not null,
  is_correct  boolean not null,
  attempts    integer not null default 1,
  answered_at timestamptz not null default now(),
  primary key (user_id, question_id)
);
create index question_progress_set_idx on public.question_progress (user_id, set_id);

create table public.set_progress (
  user_id          uuid not null default auth.uid() references auth.users (id) on delete cascade,
  set_id           uuid not null references public.exam_sets (id) on delete cascade,
  current_position integer not null default 0,
  updated_at       timestamptz not null default now(),
  primary key (user_id, set_id)
);
create index set_progress_recent_idx on public.set_progress (user_id, updated_at desc);

alter table public.question_progress enable row level security;
alter table public.set_progress enable row level security;

create policy "question_progress: own" on public.question_progress
  for all to authenticated
  using (user_id = (select auth.uid()))
  with check (user_id = (select auth.uid()));
create policy "set_progress: own" on public.set_progress
  for all to authenticated
  using (user_id = (select auth.uid()))
  with check (user_id = (select auth.uid()));

-- Records an answer, moves the resume pointer and queues an explanation if needed.
create or replace function public.record_answer(
  p_question_id uuid,
  p_response jsonb,
  p_is_correct boolean,
  p_next_position integer
)
returns void
language plpgsql security invoker
set search_path = ''
as $$
declare
  v_set uuid;
begin
  select set_id into v_set from public.questions where id = p_question_id;
  if v_set is null then
    raise exception 'question not found';
  end if;

  insert into public.question_progress (user_id, question_id, set_id, response, is_correct)
  values (auth.uid(), p_question_id, v_set, p_response, p_is_correct)
  on conflict (user_id, question_id) do update
    set response = excluded.response,
        is_correct = excluded.is_correct,
        attempts = public.question_progress.attempts + 1,
        answered_at = now();

  insert into public.set_progress (user_id, set_id, current_position, updated_at)
  values (auth.uid(), v_set, greatest(p_next_position, 0), now())
  on conflict (user_id, set_id) do update
    set current_position = excluded.current_position, updated_at = now();

  perform public.request_explanations(array[p_question_id]);
end;
$$;

-- Moves the resume pointer without answering (navigation).
create or replace function public.save_position(p_set_id uuid, p_position integer)
returns void
language sql security invoker
set search_path = ''
as $$
  insert into public.set_progress (user_id, set_id, current_position, updated_at)
  values (auth.uid(), p_set_id, greatest(p_position, 0), now())
  on conflict (user_id, set_id) do update
    set current_position = excluded.current_position, updated_at = now();
$$;

create or replace function public.reset_set_progress(p_set_id uuid)
returns void
language sql security invoker
set search_path = ''
as $$
  delete from public.question_progress where user_id = auth.uid() and set_id = p_set_id;
  delete from public.set_progress where user_id = auth.uid() and set_id = p_set_id;
$$;

-- Per-set statistics for the signed-in user.
create view public.my_set_stats
with (security_invoker = true)
as
select
  s.id                as set_id,
  s.certification_id,
  s.title,
  s.topic,
  s.position,
  s.question_count,
  count(qp.question_id)::integer                          as answered,
  (count(qp.question_id) filter (where qp.is_correct))::integer as correct,
  sp.current_position,
  greatest(sp.updated_at, max(qp.answered_at))            as last_activity
from public.exam_sets s
left join public.question_progress qp
       on qp.set_id = s.id and qp.user_id = (select auth.uid())
left join public.set_progress sp
       on sp.set_id = s.id and sp.user_id = (select auth.uid())
group by s.id, sp.current_position, sp.updated_at;

------------------------------------------------------------------------------
-- Guest → account progress merge.
-- A guest that signs in to an *existing* account (instead of linking) creates a
-- merge token first; after signing in, the account redeems it.
------------------------------------------------------------------------------

create table public.guest_merge_tokens (
  token      uuid primary key default gen_random_uuid(),
  user_id    uuid not null default auth.uid() references auth.users (id) on delete cascade,
  created_at timestamptz not null default now()
);
alter table public.guest_merge_tokens enable row level security;
create policy "guest_merge_tokens: guests create their own" on public.guest_merge_tokens
  for insert to authenticated
  with check (user_id = (select auth.uid()) and not (select public.is_permanent_user()));
create policy "guest_merge_tokens: read own" on public.guest_merge_tokens
  for select to authenticated
  using (user_id = (select auth.uid()));

create or replace function public.merge_guest_progress(p_token uuid)
returns integer
language plpgsql security definer
set search_path = ''
as $$
declare
  v_guest uuid;
  v_count integer;
begin
  if auth.uid() is null then
    raise exception 'not authenticated';
  end if;

  delete from public.guest_merge_tokens t
   where t.token = p_token and t.created_at > now() - interval '1 day'
  returning t.user_id into v_guest;

  if v_guest is null or v_guest = auth.uid()
     or not exists (select 1 from auth.users u where u.id = v_guest and u.is_anonymous) then
    return 0;
  end if;

  insert into public.question_progress as qp
    (user_id, question_id, set_id, response, is_correct, attempts, answered_at)
  select auth.uid(), g.question_id, g.set_id, g.response, g.is_correct, g.attempts, g.answered_at
    from public.question_progress g
   where g.user_id = v_guest
  on conflict (user_id, question_id) do update
    set response = excluded.response,
        is_correct = excluded.is_correct,
        attempts = qp.attempts + excluded.attempts,
        answered_at = excluded.answered_at
    where excluded.answered_at > qp.answered_at;
  get diagnostics v_count = row_count;

  insert into public.set_progress as sp (user_id, set_id, current_position, updated_at)
  select auth.uid(), g.set_id, g.current_position, g.updated_at
    from public.set_progress g
   where g.user_id = v_guest
  on conflict (user_id, set_id) do update
    set current_position = excluded.current_position, updated_at = excluded.updated_at
    where excluded.updated_at > sp.updated_at;

  delete from public.question_progress where user_id = v_guest;
  delete from public.set_progress where user_id = v_guest;
  return v_count;
end;
$$;

------------------------------------------------------------------------------
-- Storage: page snapshots, stored as <certification_id>/<question_id>-{q,a}.webp
------------------------------------------------------------------------------

insert into storage.buckets (id, name, public, file_size_limit, allowed_mime_types)
values ('question-images', 'question-images', false, 5242880,
        array['image/webp', 'image/png', 'image/jpeg'])
on conflict (id) do nothing;

create policy "question-images: read" on storage.objects
  for select to authenticated
  using (bucket_id = 'question-images');

create policy "question-images: upload (owner/admin)" on storage.objects
  for insert to authenticated
  with check (
    bucket_id = 'question-images'
    and public.can_edit_certification(((storage.foldername(name))[1])::uuid)
  );

create policy "question-images: update (owner/admin)" on storage.objects
  for update to authenticated
  using (
    bucket_id = 'question-images'
    and public.can_edit_certification(((storage.foldername(name))[1])::uuid)
  );

create policy "question-images: delete (owner/admin)" on storage.objects
  for delete to authenticated
  using (
    bucket_id = 'question-images'
    and public.can_edit_certification(((storage.foldername(name))[1])::uuid)
  );
