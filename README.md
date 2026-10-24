# ExamTopics Interactive

Turn the OCR'd ExamTopics PDFs into interactive practice exams, with progress that follows
you across devices and Claude explaining every answer.

- **Upload a PDF** → questions are parsed in the browser (pdf.js + a Rust parser), kept word for word, and grouped into **sets** under a **certification**.
- **Every ExamTopics question type**: single choice, multiple choice ("choose two"), Yes/No and hotspot **dropdowns**, **drag & drop** / ordering, and image-only questions (page snapshots + self-grading until Claude converts them into dropdowns / drag & drop).
- **Resume where you left off**: per-user answers, per-set position, and a "Continue" card on the home page.
- **Wrong answer keys**: shows the community vote, Claude says whether it agrees with the key, and the uploader can fix the key (the original stays visible).
- **Claude explanations** after every answer, generated with your **Claude subscription** through Claude Code — no API key.
- **Auth**: email + password, Google, GitHub, and guest (anonymous) access. Accounts with the same verified email are linked automatically, Google / GitHub users can set a password, and guests can link an account later without losing progress.

| Part | Tech |
| --- | --- |
| Web app | [Dioxus 0.7](https://dioxuslabs.com) (Rust → WASM) with the shadcn-style [Dioxus components](https://dioxuslabs.com/components) (`dx components add …`) |
| PDF reading | [pdf.js 6.4](https://mozilla.github.io/pdf.js/) (vendored in `crates/web/public/vendor/pdfjs`) + parser in `crates/core` |
| Auth, database, file storage | [Supabase](https://supabase.com) (Auth, Postgres with row level security, Storage) |
| Explanations | `crates/worker`: runs [Claude Code](https://code.claude.com) headless (`claude -p --json-schema …`) |
| Hosting | Vercel (static site) |

```
crates/core     question model, grading, ExamTopics PDF parser (unit tested)
crates/web      Dioxus web app
crates/worker   `explainer`: Claude explanation worker
supabase/       config.toml (local dev) + migrations (schema, RLS, storage, queue)
scripts/        Vercel install/build scripts
```

## How it fits together

```
Browser (Dioxus/WASM) ──auth, data, files──▶ Supabase (Auth · Postgres · Storage)
                                                   ▲                        │
                                                   │ results                │ queued jobs (ai_jobs)
                                                   │                        ▼
                                     explainer worker ──▶ Claude Code (your Claude subscription)
```

1. Answering a question records your answer and adds the question to the `ai_jobs` queue if it has no explanation yet.
2. The `explainer` worker claims queued jobs, runs `claude -p` with the question text, the answer key, the community votes and the page snapshots, and stores the structured result. For image-only hotspot or drag & drop questions it also writes the interactive version.
3. The quiz page polls until the explanation is ready and then shows it. Each question is explained once and shared with everyone, so it uses very little of your subscription.

## 1. Supabase

1. Create a project at [supabase.com](https://supabase.com/dashboard).
2. Apply the schema. Either run `supabase/migrations/20261024000000_init.sql` in the SQL editor, or use the CLI:
   ```bash
   supabase link --project-ref <ref>
   supabase db push
   ```
3. **Authentication → Sign In / Providers**
   - Email: keep **Confirm email** turned on. Automatic account linking only trusts verified emails.
   - Turn on **Allow anonymous sign-ins** (guest access).
   - Turn on **Allow manual linking** (lets a guest link Google or GitHub, and lets users connect a provider that has a different email).
   - **Google**: create an OAuth client (Web application) in Google Cloud Console. Set the authorized redirect URI to `https://<ref>.supabase.co/auth/v1/callback` and paste the client ID and secret into Supabase.
   - **GitHub**: create an OAuth App (GitHub → Settings → Developer settings). Set the callback URL to `https://<ref>.supabase.co/auth/v1/callback` and paste the client ID and secret into Supabase.
4. **Authentication → URL Configuration**
   - Site URL: `https://<your-app>.vercel.app`
   - Redirect URLs: `https://<your-app>.vercel.app/auth/callback`, `http://localhost:8080/auth/callback`, and if you use preview deployments, `https://*-<team>.vercel.app/auth/callback`.
5. **Project Settings → API Keys**: copy the **publishable** key (for the web app) and a **secret** key (for the worker only).
6. Optional: make someone an admin, who can then edit any certification:
   ```sql
   update public.profiles set is_admin = true where id = (select id from auth.users where email = 'you@company.com');
   ```

### How account linking works

| Situation | Result |
| --- | --- |
| Signed up with Google, later signs in with GitHub (same email) | Supabase links GitHub to the same user automatically (verified emails only). |
| Signed up with Google, wants to log in with email | Account → **Password** → set a password, then sign in with email + password. |
| Guest wants to keep their progress | Account → **Link Google / GitHub** or add an email. The same user is kept, so nothing moves. |
| Guest signs in to an existing account | The guest's progress is merged into that account through a one-time merge token. |
| Wants a provider with a *different* email | Account → Sign-in methods → **Connect**. |

## 2. Run locally

Requirements: Rust (stable) with `wasm32-unknown-unknown`, and the Dioxus CLI 0.7.10.

```bash
rustup target add wasm32-unknown-unknown
cargo binstall dioxus-cli@0.7.10   # or: cargo install dioxus-cli@0.7.10 --locked
```

To use a local Supabase instead of a hosted one (needs Docker and the [Supabase CLI](https://supabase.com/docs/guides/local-development)):

```bash
supabase start          # applies supabase/migrations; prints the API URL and keys
```

Locally, emails such as sign-up confirmations and password resets go to Mailpit at http://127.0.0.1:54324.

Start the web app. The `SUPABASE_*` values are baked in at compile time:

```bash
export SUPABASE_URL=http://127.0.0.1:54321
export SUPABASE_PUBLISHABLE_KEY=<publishable key from `supabase status`>
cd crates/web && dx serve        # http://localhost:8080
```

Run the tests:

```bash
cargo test                                         # core parser + worker
cargo check -p web --target wasm32-unknown-unknown # web app
```

## 3. Claude explanations (Claude subscription, no API key)

Explanations come from the `explainer` worker, which runs **Claude Code** in headless mode. Claude Code signs in with your Claude Pro/Max/Team/Enterprise subscription, so no Anthropic API key is involved.

On your own machine:

```bash
claude            # once: log in with your Claude account (/login)
cp .env.example .env   # set SUPABASE_URL and SUPABASE_SECRET_KEY
cargo run --release -p explainer             # watches the queue
cargo run --release -p explainer -- --once   # drain the queue and exit
```

Options (flag or env var): `--model` / `EXPLAINER_MODEL` (default `sonnet`, which stretches your limits further; `opus` for harder questions), `--effort`, `--web` (lets Claude check the docs online: slower, more accurate), `--concurrency`. The worker removes `ANTHROPIC_API_KEY` from Claude Code's environment so the subscription is always used. Pass `--allow-api-key` if you want the key used instead.

Without a machine that's always on, run it on a schedule with GitHub Actions instead. `.github/workflows/explainer.yml` runs every 15 minutes. Create a long-lived subscription token with `claude setup-token`, then add the repository secrets `CLAUDE_CODE_OAUTH_TOKEN`, `SUPABASE_URL` and `SUPABASE_SECRET_KEY`.

> **Usage policy:** the worker uses Claude Code, Anthropic's own client, with *your* login, and stores the results for your team. Anthropic doesn't allow third-party apps to offer claude.ai login to *their* users, which is why the web app never asks users to connect their Claude account.

### Who can use your subscription

Only accounts on the **explainer allowlist** (`profiles.can_use_explainer`) can queue questions for your worker. The database enforces this: `request_explanations` refuses everyone else, and answering a question only queues it for allowlisted users. Explanations that already exist are visible to everyone, since they cost nothing more to show. Everyone else gets a **Use your own Claude** button on the question, which copies a ready-made prompt and opens claude.ai, so they use their own account.

Add someone to the allowlist (SQL editor):

```sql
update public.profiles set can_use_explainer = true
where id = (select id from auth.users where email = 'person@example.com');
```

Allowlisted uploaders also get **Explain all with Claude** on a certification, which queues every question in advance. Otherwise questions are queued as allowlisted people answer them.

### Staying within your subscription

- Each question is explained once and cached, no matter how many people answer it.
- Keep **extra usage** turned off in your Claude account settings. When you hit a usage limit, the worker puts the question back in the queue without counting the attempt, then stops (`--once`) or waits 15 minutes before trying again.
- `ANTHROPIC_API_KEY` is removed from Claude Code's environment, so the API is never billed by accident.
- GitHub Actions minutes are free for public repositories.

## 4. Deploy to Vercel

The web app is a static site, so Vercel only serves files. Supabase does the rest.

1. Import the repository in Vercel and keep **Root Directory** at the repository root. `vercel.json` sets everything else:
   - install: `scripts/vercel-install.sh` installs Rust, the wasm target and `dx` 0.7.10
   - build: `scripts/vercel-build.sh` runs `dx bundle --web --release` into `dist/`
   - every route rewrites to `index.html` (client-side routing)
2. Add the environment variables `SUPABASE_URL` and `SUPABASE_PUBLISHABLE_KEY` for Production and Preview.
3. Deploy, then add the deployment URL to Supabase's redirect URLs (step 1.4).

Building with Rust on Vercel takes a few minutes. To make it faster, build in CI and upload the result: `vercel build && vercel deploy --prebuilt`.

## Using it

- **Upload PDF** (signed-in, non-guest users) → choose the PDF → check the preview and its warnings → choose a new or existing certification and the questions per set → import. Large topics are split into sets, e.g. "Topic 1 · Set 3 (Q101–150)". Page snapshots are stored in the private `question-images` bucket.
- **Library** → certification → set → answer → **Check answer** → you see the result, the official key, the community vote and Claude's explanation.
- **Wrong key?** If Claude disagrees, the uploader can click **Use Claude's answer**, or open **Edit** to fix OCR text, options, the key, or convert an image question into dropdowns or drag & drop. The original text from the PDF stays available.
- **Guests** can study and track progress. Uploading and editing need a permanent account.

## Notes on the PDF parser

The parser (`crates/core/src/parse.rs`) expects the ExamTopics layout (`Question #N Topic M`, lettered options, `Correct Answer:`, `Community vote distribution`, `HOTSPOT -` / `DRAG DROP -` markers, `Hot Area:` / `Select and Place:`, case studies with `Introductory Info`). It tolerates common OCR noise and removes running headers, footers and page numbers. Anything it is unsure about shows up as a warning in the upload preview and on the question for editors. If your PDFs use a variation it misses, add a sample to `crates/core/tests/parser.rs` and adjust the regexes at the top of `parse.rs`.
