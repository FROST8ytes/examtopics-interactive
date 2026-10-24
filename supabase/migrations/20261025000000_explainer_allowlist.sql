-- Only allowlisted accounts may queue questions for the explainer, which runs on the
-- project owner's Claude subscription. Everyone can still read explanations that exist.

alter table public.profiles
  add column if not exists can_use_explainer boolean not null default false;

create or replace function public.can_use_explainer()
returns boolean
language sql stable security definer
set search_path = ''
as $$
  select coalesce(
    (select p.can_use_explainer from public.profiles p where p.id = auth.uid()),
    false
  )
$$;

-- Same as before, but refuses callers that are not on the allowlist.
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
  if not public.can_use_explainer() then
    raise exception 'explainer_not_allowed'
      using hint = 'Ask the owner to add you to the explainer allowlist, or use your own Claude.';
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

-- Answering only queues an explanation for allowlisted users.
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

  if public.can_use_explainer() then
    perform public.request_explanations(array[p_question_id]);
  end if;
end;
$$;

-- Worker: hand a job back without counting the attempt (e.g. the subscription's usage
-- limit was reached), so it is retried once the limit resets.
create or replace function public.release_ai_job(p_job_id bigint, p_reason text)
returns void
language sql security definer
set search_path = ''
as $$
  update public.ai_jobs
     set status = 'pending',
         attempts = greatest(attempts - 1, 0),
         error = left(p_reason, 2000),
         updated_at = now(),
         locked_at = null
   where id = p_job_id;
$$;

revoke execute on function public.release_ai_job(bigint, text) from public, anon, authenticated;
grant execute on function public.release_ai_job(bigint, text) to service_role;
